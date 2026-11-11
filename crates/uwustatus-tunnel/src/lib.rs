//! The tunnel between the UwUStatus main instance and its satellites.
//!
//! A satellite runs on a machine with a public address and listens on a UDP port; the main
//! instance sits at home behind NAT and dials it over QUIC (TLS 1.3, ALPN `uwustatus-tunnel/1`),
//! keeping the connection alive. Both sides present a self-signed Ed25519 certificate and pin the
//! other one's SHA-256 fingerprint: the main instance learns the satellite's from the `uwusat1…`
//! pairing code, the satellite learns the main instance's when it brings the code's one-time
//! token. One control stream carries length-prefixed JSON, see [`proto`].

pub mod code;
mod identity;
mod limits;
mod main_client;
pub mod net;
pub mod proto;
mod quic;
mod satellite;
mod verify;

pub use code::{CodeError, PairingCode, Token};
pub use identity::{Fingerprint, Identity};
pub use main_client::{MainClient, MainEvent, MainSettings};
pub use proto::{Assignment, MainMessage, Refusal, ResultItem, SatelliteMessage};
pub use quic::{Timing, client_config};
pub use satellite::{SatelliteEvent, SatelliteListener, SatelliteSettings};

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("certificate: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("QUIC: {0}")]
    Quic(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}
