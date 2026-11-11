//! UwUStatus server: admin app, status pages, (later) scheduler and alerts.

pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod local;
pub mod oidc;
pub mod settings;
pub mod status;
pub mod web;

use base64::Engine as _;
use sha2::Digest as _;

/// Unix seconds.
pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// Unix seconds as RFC 3339 (UTC).
pub fn iso(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds).map(|t| t.to_string()).unwrap_or_default()
}

/// 256 random bits, base64url.
pub fn random_token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

pub fn sha256_hex(value: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(value.as_bytes()))
}

pub fn sha256_b64(value: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(value.as_bytes()))
}

/// Comparison that takes as long whatever the inputs share.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
