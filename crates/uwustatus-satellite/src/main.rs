use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use uwustatus_satellite::config::{Config, config_file};
use uwustatus_satellite::deadman::NotifyNotifier;
use uwustatus_satellite::state::{Saved, Store};
use uwustatus_satellite::{agent, utc};
use uwustatus_tunnel::{PairingCode, Timing, Token};

#[derive(Parser)]
#[command(name = "uwustatus-satellite", version, about = "UwUStatus-Satellit")]
struct Cli {
    /// TOML file (default: /etc/uwustatus-satellite/satellite.toml if present).
    #[arg(long, env = "UWUSTATUS_SATELLITE_CONFIG", global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Creates the identity and prints a one-time pairing code for the main instance.
    Init,
    /// Runs the satellite (the systemd service).
    Run,
    /// Shows pairing, connection and buffer.
    Status,
    /// Forgets the main instance, its checks and the dead man's switch.
    Unpair,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = Config::load(config_file(cli.config).as_deref()).and_then(|config| match cli.command {
        Command::Init => init(&config),
        Command::Run => run(&config),
        Command::Status => status(&config),
        Command::Unpair => unpair(&config),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uwustatus-satellite: {e}");
            ExitCode::FAILURE
        }
    }
}

fn store(config: &Config) -> Result<(Store, Saved), String> {
    let store = Store::open(&config.state_dir).map_err(|e| format!("{}: {e}", config.state_dir.display()))?;
    let saved = store.saved().map_err(|e| format!("Zustand: {e}"))?;
    Ok((store, saved))
}

fn init(config: &Config) -> Result<(), String> {
    let (store, mut saved) = store(config)?;
    if saved.main.is_some() {
        return Err("schon mit einer Hauptinstanz gekoppelt; erst `uwustatus-satellite unpair`".into());
    }
    let identity = store.identity().map_err(|e| format!("Identität: {e}"))?;
    let ips = match config.public_addresses.as_slice() {
        [] => uwustatus_tunnel::net::public_addresses(),
        given => given.to_vec(),
    };
    if ips.is_empty() {
        return Err("keine öffentliche Adresse gefunden; public_addresses in der Konfiguration setzen".into());
    }
    let token = Token::generate();
    saved.token = Some(token.to_text());
    store.save(&saved).map_err(|e| format!("Zustand: {e}"))?;
    let addresses = ips.iter().map(|ip| (*ip, config.listen.port()).into()).collect();
    let code = PairingCode { addresses, fingerprint: identity.fingerprint(), token }.encode();
    println!("Fingerabdruck: {}", identity.fingerprint());
    println!("Kopplungscode (einmalig, in UwUStatus unter Satelliten eintragen):\n\n{code}\n");
    println!("Danach den Dienst neu starten: systemctl restart uwustatus-satellite");
    Ok(())
}

fn run(config: &Config) -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(&config.log).unwrap_or_else(|_| "info".into()))
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let agent = agent::start(config, Timing::default(), Arc::new(NotifyNotifier::new()?)).await?;
        tracing::info!(
            "listening on {} (UDP){}",
            agent.listener.local_addr().map_err(|e| e.to_string())?,
            agent.page.map(|a| format!(", emergency page on http://{a}")).unwrap_or_default()
        );
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        agent.stop();
        Ok(())
    })
}

fn status(config: &Config) -> Result<(), String> {
    let (store, saved) = store(config)?;
    match saved.main {
        Some(main) => println!("Gekoppelt mit: {main}"),
        None if saved.token.is_some() => println!("Nicht gekoppelt, Kopplungscode wartet"),
        None => println!("Nicht gekoppelt (uwustatus-satellite init)"),
    }
    println!("Prüfungen: {}", saved.assignments.len());
    match &saved.dead_man {
        Some(d) if d.after_s > 0 => println!(
            "Totmann: nach {} min, {} Kanäle{}",
            d.after_s / 60,
            d.channels.len(),
            if saved.alarmed { ", Alarm ausgelöst" } else { "" }
        ),
        _ => println!("Totmann: aus"),
    }
    if let Some(s) = store.status().map_err(|e| e.to_string())? {
        let since = s.since.map(utc).unwrap_or_default();
        println!("Verbindung: {} seit {since}", if s.connected { "verbunden" } else { "getrennt" });
        println!("Puffer: {} Ergebnisse", s.buffered);
    }
    Ok(())
}

fn unpair(config: &Config) -> Result<(), String> {
    let (store, _) = store(config)?;
    store.save(&Saved::default()).map_err(|e| e.to_string())?;
    println!("Kopplung gelöscht. Dienst neu starten, dann `uwustatus-satellite init` für einen neuen Code.");
    Ok(())
}
