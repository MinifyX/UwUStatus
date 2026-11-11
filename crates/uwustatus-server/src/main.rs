use std::process::ExitCode;
use uwustatus_server::config::Config;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("serve") => serve(),
        Some("healthcheck") => healthcheck(),
        Some("totp-secret") => {
            let secret = uwustatus_server::local::new_totp_secret();
            println!("{secret}\notpauth://totp/UwUStatus?secret={secret}&issuer=UwUStatus");
            Ok(())
        }
        _ => Err("Aufruf: uwustatus [serve | healthcheck | totp-secret]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uwustatus: {e}");
            ExitCode::FAILURE
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())
}

fn serve() -> Result<(), String> {
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,hyper=warn,reqwest=warn".into()),
        )
        .init();
    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.data_dir).map_err(|e| format!("{}: {e}", config.data_dir.display()))?;
    let db = uwustatus_server::db::Db::open(&config.data_dir.join("uwustatus.sqlite3"))
        .map_err(|e| format!("SQLite: {e}"))?;
    runtime()?.block_on(async move {
        let bind = config.bind;
        let state = uwustatus_server::app::build(config, db)?;
        uwustatus_server::incidents::spawn(&state);
        uwustatus_server::planner::spawn(&state)?;
        uwustatus_server::alerts::spawn(&state);
        let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| format!("{bind}: {e}"))?;
        tracing::info!("listening on {bind}");
        let shutdown = async {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async { match term.as_mut() { Some(t) => { t.recv().await; } None => std::future::pending().await } } => {}
            }
        };
        axum::serve(listener, uwustatus_server::app::router(state))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| e.to_string())
    })
}

fn healthcheck() -> Result<(), String> {
    let port = std::env::var("UWUSTATUS_BIND")
        .ok()
        .and_then(|b| b.parse::<std::net::SocketAddr>().ok())
        .map_or(8080, |a| a.port());
    runtime()?.block_on(async {
        let response = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status().is_success() { Ok(()) } else { Err(format!("HTTP {}", response.status())) }
    })
}
