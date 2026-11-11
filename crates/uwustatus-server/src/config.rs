//! Configuration from the environment (`UWUSTATUS_*`). Secrets only come from files (`*_FILE`),
//! are read once and live in [`Secret`], which never prints its value.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

/// A credential. `Debug` and `Display` never show it; only [`Secret::expose`] does.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Secret,
    /// Origin for server-to-server calls (discovery, token, JWKS) instead of the issuer's.
    pub backchannel: Option<String>,
}

/// The local fallback admin, for when UwUAuth is down.
#[derive(Clone, Debug)]
pub struct AdminConfig {
    pub user: String,
    /// Argon2 PHC string (`$argon2id$…`).
    pub password_hash: Secret,
    /// Base32 TOTP secret; `None`: password only.
    pub totp: Option<Secret>,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Origin of the admin app as the browser sees it, e.g. `https://status.example.com` (no slash).
    /// Every other `Host` is treated as a status page.
    pub public_url: String,
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub oidc: Option<OidcConfig>,
    pub allowed_group: Option<String>,
    pub allowed_subjects: Vec<String>,
    pub admin: Option<AdminConfig>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Reads the secret file named by `name` (e.g. `UWUSTATUS_OIDC_CLIENT_SECRET_FILE`). Unset → `None`.
fn secret(name: &str) -> Result<Option<Secret>, String> {
    let Some(path) = var(name) else { return Ok(None) };
    let value = std::fs::read_to_string(&path).map_err(|e| format!("{name}: {path} lässt sich nicht lesen ({e})"))?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| Secret::new(value)))
}

fn list(name: &str) -> Vec<String> {
    var(name)
        .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// `https://host[:port]` of a URL, or an error naming `what`.
pub fn origin_of(value: &str, what: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|_| format!("{what}: keine gültige URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(format!("{what}: nur http(s)-URLs"));
    }
    Ok(url.origin().ascii_serialization())
}

fn oidc() -> Result<Option<OidcConfig>, String> {
    let Some(issuer) = var("UWUSTATUS_OIDC_ISSUER") else { return Ok(None) };
    origin_of(&issuer, "UWUSTATUS_OIDC_ISSUER")?;
    let backchannel =
        var("UWUSTATUS_OIDC_BACKCHANNEL_URL").map(|v| origin_of(&v, "UWUSTATUS_OIDC_BACKCHANNEL_URL")).transpose()?;
    Ok(Some(OidcConfig {
        issuer,
        client_id: var("UWUSTATUS_OIDC_CLIENT_ID").ok_or("UWUSTATUS_OIDC_CLIENT_ID fehlt")?,
        client_secret: secret("UWUSTATUS_OIDC_CLIENT_SECRET_FILE")?.ok_or("UWUSTATUS_OIDC_CLIENT_SECRET_FILE fehlt")?,
        backchannel,
    }))
}

/// The password file holds the password itself or an Argon2 PHC string; a password is hashed here.
fn admin() -> Result<Option<AdminConfig>, String> {
    let Some(password) = secret("UWUSTATUS_ADMIN_PASSWORD_FILE")? else { return Ok(None) };
    let password_hash = if password.expose().starts_with("$argon2") {
        crate::local::check_hash(password.expose()).map_err(|e| format!("UWUSTATUS_ADMIN_PASSWORD_FILE: {e}"))?;
        password
    } else {
        if password.expose().chars().count() < 12 {
            return Err("UWUSTATUS_ADMIN_PASSWORD_FILE: Passwort mit mindestens 12 Zeichen".into());
        }
        Secret::new(crate::local::hash_password(password.expose(), argon2::Params::default())?)
    };
    let totp = secret("UWUSTATUS_ADMIN_TOTP_SECRET_FILE")?;
    if let Some(t) = &totp {
        crate::local::totp_key(t.expose()).map_err(|e| format!("UWUSTATUS_ADMIN_TOTP_SECRET_FILE: {e}"))?;
    }
    Ok(Some(AdminConfig { user: var("UWUSTATUS_ADMIN_USER").unwrap_or_else(|| "admin".into()), password_hash, totp }))
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let public_url =
            origin_of(&var("UWUSTATUS_PUBLIC_URL").ok_or("UWUSTATUS_PUBLIC_URL fehlt")?, "UWUSTATUS_PUBLIC_URL")?;
        let bind = var("UWUSTATUS_BIND")
            .unwrap_or_else(|| "0.0.0.0:8080".into())
            .parse()
            .map_err(|_| "UWUSTATUS_BIND: erwartet Adresse:Port")?;
        let (oidc, admin) = (oidc()?, admin()?);
        if oidc.is_none() && admin.is_none() {
            return Err(
                "Keine Anmeldung möglich: UWUSTATUS_OIDC_* und/oder UWUSTATUS_ADMIN_PASSWORD_FILE setzen".into()
            );
        }
        Ok(Self {
            public_url,
            bind,
            data_dir: var("UWUSTATUS_DATA_DIR").unwrap_or_else(|| "/data".into()).into(),
            oidc,
            allowed_group: var("UWUSTATUS_ALLOWED_GROUP"),
            allowed_subjects: list("UWUSTATUS_ALLOWED_SUBJECTS"),
            admin,
        })
    }

    /// The host (and port, if any) of the admin app; every other `Host` is a status page.
    pub fn public_authority(&self) -> String {
        self.public_url.split_once("://").map_or(self.public_url.as_str(), |(_, rest)| rest).to_owned()
    }

    pub fn redirect_uri(&self) -> String {
        format!("{}/api/auth/callback", self.public_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?} {s}"), "Secret(<redacted>) <redacted>");
    }

    #[test]
    fn origin() {
        assert_eq!(origin_of("https://Status.Example.com/x/", "u").unwrap(), "https://status.example.com");
        assert_eq!(origin_of("http://localhost:8080", "u").unwrap(), "http://localhost:8080");
        assert!(origin_of("ftp://example.com", "u").is_err());
    }
}
