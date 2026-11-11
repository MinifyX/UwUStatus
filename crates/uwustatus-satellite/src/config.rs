//! Configuration from an optional TOML file, overridable with `UWUSTATUS_SATELLITE_*` environment
//! variables. Nested keys use a double underscore: `UWUSTATUS_SATELLITE_EMERGENCY__LISTEN=`.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use serde::Deserialize;

pub const INSTALLED_CONFIG: &str = "/etc/uwustatus-satellite/satellite.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// UDP address the main instance dials.
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    /// Addresses written into the pairing code; empty = the global addresses of this host.
    pub public_addresses: Vec<IpAddr>,
    /// Results kept while the main instance is away.
    pub buffer_limit: usize,
    pub emergency: EmergencyConfig,
    /// `tracing` filter.
    pub log: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: "[::]:7843".parse().expect("valid default"),
            state_dir: PathBuf::from("/var/lib/uwustatus-satellite"),
            public_addresses: Vec::new(),
            buffer_limit: 10_000,
            emergency: EmergencyConfig::default(),
            log: "info".into(),
        }
    }
}

/// The emergency status page: plain HTTP, meant to sit behind Caddy (or another TLS proxy).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmergencyConfig {
    /// For example `127.0.0.1:8090`; unset = no page. Whether it shows anything is set in the main instance.
    pub listen: Option<SocketAddr>,
    /// Optional picture (PNG) shown on the page, e.g. a Nyu sticker.
    pub image: Option<PathBuf>,
}

/// The file that was named, else the installed one if it exists.
pub fn config_file(named: Option<PathBuf>) -> Option<PathBuf> {
    let installed = Path::new(INSTALLED_CONFIG);
    named.or_else(|| installed.exists().then(|| installed.to_path_buf()))
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Config, String> {
        let mut figment = Figment::new();
        if let Some(path) = path {
            if !path.exists() {
                return Err(format!("Konfiguration {} fehlt", path.display()));
            }
            figment = figment.merge(Toml::file(path));
        }
        figment = figment.merge(Env::prefixed("UWUSTATUS_SATELLITE_").split("__").ignore(&["config"]));
        figment.extract().map_err(|e| format!("Konfiguration ungültig: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("satellite.toml");
        std::fs::write(&path, "listen = \"0.0.0.0:9000\"\n[emergency]\nlisten = \"127.0.0.1:8090\"\n").unwrap();
        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.listen.port(), 9000);
        assert_eq!(config.emergency.listen, Some("127.0.0.1:8090".parse().unwrap()));
        assert_eq!(config.buffer_limit, 10_000);
        std::fs::write(&path, "unknown = 1\n").unwrap();
        assert!(Config::load(Some(&path)).is_err());
    }
}
