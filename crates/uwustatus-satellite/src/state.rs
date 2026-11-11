//! What the satellite keeps in its state directory, every file mode 0600: `identity.json` (its key),
//! `state.json` (paired main instance, pending token, assignments, dead man) and `status.json`
//! (written by `run` for `status`).

use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uwustatus_tunnel::{Assignment, Fingerprint, Identity};

use crate::deadman::DeadManConfig;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub main: Option<Fingerprint>,
    /// The one-time token of the pairing code, until a main instance used it.
    pub token: Option<String>,
    pub assignments: Vec<Assignment>,
    pub dead_man: Option<DeadManConfig>,
    /// Unix seconds since the main instance is away (survives restarts, so the deadline does too).
    pub lost_since: Option<i64>,
    /// The alarm went out and waits for its all-clear.
    pub alarmed: bool,
}

/// Written by `run` on every change, read by `status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub connected: bool,
    /// Unix seconds of the last connect or loss.
    pub since: Option<i64>,
    pub buffered: usize,
    pub assignments: usize,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(dir: &Path) -> io::Result<Store> {
        std::fs::create_dir_all(dir)?;
        Ok(Store { dir: dir.to_path_buf() })
    }

    /// The stored identity, or a new one written now.
    pub fn identity(&self) -> io::Result<Identity> {
        let path = self.dir.join("identity.json");
        if let Some(identity) = read_json(&path)? {
            return Ok(identity);
        }
        let identity = Identity::generate().map_err(io::Error::other)?;
        write_json(&path, &identity)?;
        Ok(identity)
    }

    pub fn saved(&self) -> io::Result<Saved> {
        Ok(read_json(&self.dir.join("state.json"))?.unwrap_or_default())
    }

    pub fn save(&self, saved: &Saved) -> io::Result<()> {
        write_json(&self.dir.join("state.json"), saved)
    }

    pub fn status(&self) -> io::Result<Option<Status>> {
        read_json(&self.dir.join("status.json"))
    }

    pub fn save_status(&self, status: &Status) -> io::Result<()> {
        write_json(&self.dir.join("status.json"), status)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Atomic (temporary file, then rename) and private.
fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn kept_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let first = store.identity().unwrap().fingerprint();
        assert_eq!(store.identity().unwrap().fingerprint(), first);
        store.save(&Saved { main: Some(first), alarmed: true, ..Saved::default() }).unwrap();
        let saved = store.saved().unwrap();
        assert_eq!(saved.main, Some(first));
        assert!(saved.alarmed);
        for file in ["identity.json", "state.json"] {
            let mode = std::fs::metadata(dir.path().join(file)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{file}");
        }
    }
}
