//! QUIC endpoints with the tunnel's TLS settings.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Connection, Endpoint, IdleTimeout, ServerConfig, TransportConfig, VarInt};
use rustls::crypto::CryptoProvider;
use rustls_pki_types::CertificateDer;

use crate::TunnelError;
use crate::identity::{Fingerprint, Identity};
use crate::verify::{AnyClient, PinnedServer};

const ALPN: &[u8] = b"uwustatus-tunnel/1";
/// What the satellite lets a peer buffer before it has shown to be the paired main instance: no
/// more than a hello needs. The one control stream is all anyone ever gets.
const UNPAIRED_RECEIVE_WINDOW: u32 = 256 * 1024;
/// Room for a full assignment list once the main instance is known.
const PAIRED_RECEIVE_WINDOW: u32 = 16 * 1024 * 1024;

/// Timings of the tunnel. The defaults suit the internet; tests make them short.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Keeps the mapping of home routers and carrier-grade NAT open while nothing else is sent.
    pub keep_alive: Duration,
    /// A connection without any sign of life for this long counts as lost.
    pub idle_timeout: Duration,
    /// Wait after the first failed dial; doubles up to `retry_max`.
    pub retry_min: Duration,
    pub retry_max: Duration,
    /// After a refusal nothing changes until someone acts; asking often would only fill the log.
    pub refused_retry: Duration,
    /// For dialling, the TLS handshake and the hello.
    pub handshake_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            keep_alive: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(30),
            retry_min: Duration::from_secs(1),
            retry_max: Duration::from_secs(60),
            refused_retry: Duration::from_secs(300),
            handshake_timeout: Duration::from_secs(10),
        }
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// `bidi_streams` the peer may open, all through one window of `receive_window` (default if `None`).
fn transport(timing: &Timing, bidi_streams: u32, receive_window: Option<u32>) -> Arc<TransportConfig> {
    let mut transport = TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(VarInt::from_u32(bidi_streams))
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .keep_alive_interval(Some(timing.keep_alive))
        .max_idle_timeout(IdleTimeout::try_from(timing.idle_timeout).ok());
    if let Some(window) = receive_window {
        transport.receive_window(VarInt::from_u32(window));
    }
    Arc::new(transport)
}

/// The satellite's endpoint: waits for the main instance on `address` (UDP). Anyone can connect;
/// until [`open_up`] a stranger gets the one control stream and a quarter of a megabyte in flight.
pub fn server_endpoint(address: SocketAddr, identity: &Identity, timing: &Timing) -> Result<Endpoint, TunnelError> {
    let provider = provider();
    let verifier = Arc::new(AnyClient { algorithms: provider.signature_verification_algorithms });
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![identity.certificate()], identity.private_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicServerConfig::try_from(tls).map_err(|err| TunnelError::Quic(err.to_string()))?;
    let mut config = ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(transport(timing, 1, Some(UNPAIRED_RECEIVE_WINDOW)));
    // No connection migration: a new home address means a new handshake, which the per-network
    // handshake limits see.
    config.migration(false);
    Ok(Endpoint::server(config, address)?)
}

/// Gives a connection the satellite accepted the full window, once it is the paired main instance's.
pub(crate) fn open_up(connection: &Connection) {
    connection.set_receive_window(VarInt::from_u32(PAIRED_RECEIVE_WINDOW));
}

/// The main instance's side: shows `identity` and trusts only the certificate with `satellite`'s
/// fingerprint. The satellite never opens streams.
pub fn client_config(
    identity: &Identity,
    satellite: Fingerprint,
    timing: &Timing,
) -> Result<ClientConfig, TunnelError> {
    let provider = provider();
    let verifier = Arc::new(PinnedServer { pin: satellite, algorithms: provider.signature_verification_algorithms });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![identity.certificate()], identity.private_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicClientConfig::try_from(tls).map_err(|err| TunnelError::Quic(err.to_string()))?;
    let mut config = ClientConfig::new(Arc::new(crypto));
    config.transport_config(transport(timing, 0, None));
    Ok(config)
}

/// The fingerprint of the certificate the other side showed in the handshake.
pub(crate) fn peer_fingerprint(connection: &Connection) -> Option<Fingerprint> {
    let identity = connection.peer_identity()?;
    let certificates = identity.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certificates.first().map(|certificate| Fingerprint::of(certificate))
}
