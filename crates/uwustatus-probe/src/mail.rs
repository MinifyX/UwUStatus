//! SMTP und IMAP: Banner lesen, optional STARTTLS bzw. TLS und Zertifikat-Ablauf.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::{Ctx, Details, MailCheck, MailTls, Outcome, Status, set_cert, short};

const MAX_LINE: u64 = 2048;
const MAX_LINES: usize = 50;

#[derive(Clone, Copy)]
pub(crate) enum Proto {
    Smtp,
    Imap,
}

pub(crate) async fn mail(c: &MailCheck, ctx: &Ctx, proto: Proto) -> Result<Outcome, String> {
    let tcp = TcpStream::connect((c.host.as_str(), c.port)).await.map_err(|e| e.to_string())?;
    let config = if c.ignore_tls_errors { &ctx.tls_insecure } else { &ctx.tls_verified };
    let mut details = Details::default();
    let banner = match c.tls {
        MailTls::None => {
            let mut r = BufReader::new(tcp);
            let banner = greeting(&mut r, proto).await?;
            quit(r.get_mut(), proto).await;
            banner
        }
        MailTls::Tls => {
            let tls = handshake(tcp, &c.host, config).await?;
            set_cert(&mut details, peer_cert(&tls));
            let mut r = BufReader::new(tls);
            let banner = greeting(&mut r, proto).await?;
            quit(r.get_mut(), proto).await;
            banner
        }
        MailTls::StartTls => {
            let mut r = BufReader::new(tcp);
            let banner = greeting(&mut r, proto).await?;
            starttls(&mut r, proto).await?;
            // Gepufferte Bytes nach der STARTTLS-Antwort werden bewusst verworfen (Injection).
            let mut tls = handshake(r.into_inner(), &c.host, config).await?;
            set_cert(&mut details, peer_cert(&tls));
            quit(&mut tls, proto).await;
            banner
        }
    };
    details.banner = Some(banner.clone());
    Ok(Outcome::new(Status::Up, banner).with(details))
}

/// Begrüßung lesen und prüfen, liefert den Banner-Text.
pub(crate) async fn greeting<R: AsyncBufRead + Unpin>(r: &mut R, proto: Proto) -> Result<String, String> {
    match proto {
        Proto::Smtp => match smtp_reply(r).await? {
            (220, text) => Ok(text),
            (code, text) => Err(format!("SMTP {code} {text}")),
        },
        Proto::Imap => {
            let line = read_line(r).await?;
            let upper = line.to_ascii_uppercase();
            if upper.starts_with("* OK") || upper.starts_with("* PREAUTH") {
                Ok(short(line.get(2..).unwrap_or_default()))
            } else {
                Err(format!("IMAP greeting: {}", short(&line)))
            }
        }
    }
}

async fn starttls<S: AsyncRead + AsyncWrite + Unpin>(r: &mut BufReader<S>, proto: Proto) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    match proto {
        Proto::Smtp => {
            r.get_mut().write_all(b"EHLO uwustatus.invalid\r\n").await.map_err(io)?;
            let (code, text) = smtp_reply(r).await?;
            if code != 250 {
                return Err(format!("EHLO: SMTP {code} {text}"));
            }
            if !text.to_ascii_uppercase().contains("STARTTLS") {
                return Err("STARTTLS not offered".into());
            }
            r.get_mut().write_all(b"STARTTLS\r\n").await.map_err(io)?;
            match smtp_reply(r).await? {
                (220, _) => Ok(()),
                (code, text) => Err(format!("STARTTLS: SMTP {code} {text}")),
            }
        }
        Proto::Imap => {
            r.get_mut().write_all(b"u1 STARTTLS\r\n").await.map_err(io)?;
            for _ in 0..MAX_LINES {
                let line = read_line(r).await?;
                if let Some(rest) = line.strip_prefix("u1 ") {
                    return match rest.get(..2).is_some_and(|s| s.eq_ignore_ascii_case("OK")) {
                        true => Ok(()),
                        false => Err(format!("STARTTLS: {}", short(rest))),
                    };
                }
            }
            Err("STARTTLS: no tagged response".into())
        }
    }
}

/// Höflich abmelden; Fehler sind egal.
async fn quit<W: AsyncWrite + Unpin>(w: &mut W, proto: Proto) {
    let cmd: &[u8] = match proto {
        Proto::Smtp => b"QUIT\r\n",
        Proto::Imap => b"u2 LOGOUT\r\n",
    };
    let _ = w.write_all(cmd).await;
    let _ = w.shutdown().await;
}

/// SMTP-Antwort (auch mehrzeilig `250-…`), liefert Code und zusammengefügten Text.
pub(crate) async fn smtp_reply<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<(u16, String), String> {
    let mut text = String::new();
    for _ in 0..MAX_LINES {
        let line = read_line(r).await?;
        let code = line
            .get(..3)
            .filter(|c| c.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| format!("invalid SMTP reply: {}", short(&line)))?;
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line.get(4..).unwrap_or_default().trim());
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok((code, short(&text)));
        }
    }
    Err("SMTP reply too long".into())
}

/// Eine Zeile, höchstens `MAX_LINE` Bytes, ohne Zeilenende.
async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<String, String> {
    let mut buf = Vec::new();
    let n = r.take(MAX_LINE).read_until(b'\n', &mut buf).await.map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    Ok(String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_owned())
}

async fn handshake(tcp: TcpStream, host: &str, config: &Arc<ClientConfig>) -> Result<TlsStream<TcpStream>, String> {
    let name = ServerName::try_from(host.to_owned()).map_err(|e| format!("invalid TLS name {host:?}: {e}"))?;
    TlsConnector::from(config.clone()).connect(name, tcp).await.map_err(|e| format!("TLS: {e}"))
}

fn peer_cert(tls: &TlsStream<TcpStream>) -> Option<&[u8]> {
    tls.get_ref().1.peer_certificates()?.first().map(|c| c.as_ref())
}

/// (geprüft über die Plattform-Zertifikate, ungeprüft)
pub(crate) fn tls_configs() -> Result<(Arc<ClientConfig>, Arc<ClientConfig>), crate::Error> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone())?;
    let verified = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let insecure = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
        .with_no_client_auth();
    Ok((Arc::new(verified), Arc::new(insecure)))
}

/// Für „TLS-Fehler ignorieren“: nimmt jedes Zertifikat, wir wollen nur Erreichbarkeit und Ablauf.
#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn parse(bytes: &[u8]) {
        let _ = smtp_reply(&mut BufReader::new(bytes)).await;
        let _ = greeting(&mut BufReader::new(bytes), Proto::Imap).await;
        let _ = greeting(&mut BufReader::new(bytes), Proto::Smtp).await;
    }

    #[tokio::test]
    async fn replies() {
        let r = smtp_reply(&mut BufReader::new(&b"250-a.test\r\n250-PIPELINING\r\n250 STARTTLS\r\n"[..])).await;
        assert_eq!(r.unwrap(), (250, "a.test PIPELINING STARTTLS".into()));
        assert!(smtp_reply(&mut BufReader::new(&b"25\r\n"[..])).await.is_err());
        assert!(smtp_reply(&mut BufReader::new(&b"+50 x\r\n"[..])).await.is_err());
        assert!(smtp_reply(&mut BufReader::new(&b"250-x\r\n"[..])).await.is_err());
        let long = "250-x\r\n".repeat(100);
        assert_eq!(smtp_reply(&mut BufReader::new(long.as_bytes())).await.unwrap_err(), "SMTP reply too long");
    }

    #[tokio::test]
    async fn fuzz_banners() {
        let pieces: [&[u8]; 12] =
            [b"220", b"-", b" ", b"\r\n", b"\n", b"* OK", b"* BYE", b"\xff\xfe", "ä".as_bytes(), b"2", b"\0", b"x"];
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..5_000 {
            let mut buf = Vec::new();
            for _ in 0..(x % 16) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                buf.extend_from_slice(pieces[(x % pieces.len() as u64) as usize]);
            }
            parse(&buf).await;
        }
        parse(&vec![b'2'; 100_000]).await;
    }
}
