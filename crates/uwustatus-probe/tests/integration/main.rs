//! Eine Integrationstest-Binary: alle Fakes laufen lokal auf 127.0.0.1:0.

mod dns;
mod http;
mod mail;
mod net;

use std::sync::{Arc, LazyLock};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use uwustatus_probe::{Check, CheckKind, Ctx, Outcome, run};

pub static CTX: LazyLock<Ctx> = LazyLock::new(|| Ctx::new().unwrap());

/// Selbstsigniertes Zertifikat für `localhost`, gültig bis 2040-05-06.
pub static ACCEPTOR: LazyLock<TlsAcceptor> = LazyLock::new(|| {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut p = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    p.not_after = rcgen::date_time_ymd(2040, 5, 6);
    let cert = p.self_signed(&key).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
    TlsAcceptor::from(Arc::new(config))
});

pub async fn check(timeout_ms: u64, kind: CheckKind) -> Outcome {
    run(&Check { timeout_ms, kind }, &CTX).await
}

pub async fn listener() -> (TcpListener, u16) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub type Handler = fn(&Request) -> Vec<u8>;

/// Minimaler HTTP/1.1-Fake: eine Antwort je Verbindung, dann schließen.
pub async fn http_server(tls: bool, handler: Handler) -> String {
    let (l, port) = listener().await;
    tokio::spawn(async move {
        loop {
            let (tcp, _) = l.accept().await.unwrap();
            tokio::spawn(async move {
                if tls {
                    if let Ok(s) = ACCEPTOR.accept(tcp).await {
                        serve_one(s, handler).await;
                    }
                } else {
                    serve_one(tcp, handler).await;
                }
            });
        }
    });
    format!("{}://127.0.0.1:{port}", if tls { "https" } else { "http" })
}

async fn serve_one<S: AsyncRead + AsyncWrite + Unpin>(s: S, handler: Handler) {
    let mut r = BufReader::new(s);
    let mut line = String::new();
    if r.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
    let mut headers = Vec::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await.unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let len = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; len];
    let _ = r.read_exact(&mut body).await;
    let req = Request { method, path, headers, body: String::from_utf8_lossy(&body).into() };
    let resp = handler(&req);
    let _ = r.get_mut().write_all(&resp).await;
    let _ = r.get_mut().shutdown().await;
}

pub fn resp(code: u16, extra_headers: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}", body.len())
        .into_bytes()
}

/// Server, der Verbindungen annimmt und nie antwortet.
pub async fn silent_server() -> u16 {
    let (l, port) = listener().await;
    tokio::spawn(async move {
        let mut keep = Vec::new();
        while let Ok((s, _)) = l.accept().await {
            keep.push(s);
        }
    });
    port
}

/// Zeilenbasierter Fake für SMTP/IMAP: `replies` ordnet Befehlspräfixe Antworten zu.
/// Nach einem Befehl mit `STARTTLS` wird auf TLS umgeschaltet.
pub async fn mail_server(
    greeting: &'static str,
    replies: &'static [(&'static str, &'static str)],
    implicit: bool,
) -> u16 {
    let (l, port) = listener().await;
    tokio::spawn(async move {
        loop {
            let (tcp, _) = l.accept().await.unwrap();
            tokio::spawn(async move {
                if implicit {
                    let Ok(tls) = ACCEPTOR.accept(tcp).await else { return };
                    let mut r = BufReader::new(tls);
                    let _ = r.get_mut().write_all(greeting.as_bytes()).await;
                    lines(&mut r, replies).await;
                } else {
                    let mut r = BufReader::new(tcp);
                    let _ = r.get_mut().write_all(greeting.as_bytes()).await;
                    if lines(&mut r, replies).await {
                        let tcp: TcpStream = r.into_inner();
                        let Ok(tls) = ACCEPTOR.accept(tcp).await else { return };
                        lines(&mut BufReader::new(tls), replies).await;
                    }
                }
            });
        }
    });
    port
}

async fn lines<S: AsyncRead + AsyncWrite + Unpin>(r: &mut BufReader<S>, replies: &[(&str, &str)]) -> bool {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await.unwrap_or(0) == 0 {
            return false;
        }
        let reply = replies.iter().find(|(p, _)| line.starts_with(p)).map(|(_, r)| *r).unwrap_or("500 ?\r\n");
        if r.get_mut().write_all(reply.as_bytes()).await.is_err() {
            return false;
        }
        if line.to_ascii_uppercase().contains("STARTTLS") {
            return true;
        }
    }
}

#[test]
fn check_json_roundtrip() {
    let json = r#"{"type":"http","timeout_ms":5000,"url":"https://example.com/","keyword":"ok"}"#;
    let c: Check = serde_json::from_str(json).unwrap();
    assert_eq!(c.timeout_ms, 5000);
    let CheckKind::Http(h) = &c.kind else { panic!("{c:?}") };
    assert_eq!((h.method.as_str(), h.follow_redirects, h.keyword.as_deref()), ("GET", true, Some("ok")));
    assert_eq!(serde_json::from_str::<Check>(&serde_json::to_string(&c).unwrap()).unwrap(), c);

    let c: Check =
        serde_json::from_str(r#"{"type":"smtp","host":"mx.example.com","port":25,"tls":"start_tls"}"#).unwrap();
    assert_eq!(c.timeout_ms, 0);
    assert!(matches!(
        c.kind,
        CheckKind::Smtp(uwustatus_probe::MailCheck { tls: uwustatus_probe::MailTls::StartTls, .. })
    ));
    let c: Check = serde_json::from_str(r#"{"type":"dns","name":"example.com"}"#).unwrap();
    assert!(matches!(c.kind, CheckKind::Dns(ref d) if d.record_type == "A"));
}
