//! The local fallback admin: Argon2 password, optional TOTP (RFC 6238).

use crate::app::AppState;
use crate::auth::start_session;
use crate::error::{ApiError, ApiResult, Body};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use serde::Deserialize;

const TOTP_STEP: i64 = 30;

/// Argon2id PHC string. Production uses `Params::default()`, tests something cheap.
pub fn hash_password(password: &str, params: Params) -> Result<String, String> {
    let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>()).map_err(|e| e.to_string())?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Argon2: {e}"))
}

/// A complete Argon2 PHC string (parameters, salt and hash), so a typo cannot lock the admin out silently.
pub fn check_hash(hash: &str) -> Result<(), String> {
    let parsed = PasswordHash::new(hash).map_err(|e| format!("kein gültiger Argon2-Hash ({e})"))?;
    if parsed.hash.is_none() || parsed.salt.is_none() || Params::try_from(&parsed).is_err() {
        return Err("Argon2-Hash unvollständig".into());
    }
    Ok(())
}

/// Parameters come from the hash itself.
pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// Base32 secret as authenticator apps show it (spaces, lower case and padding allowed).
pub fn totp_key(secret: &str) -> Result<Vec<u8>, String> {
    let clean: String =
        secret.chars().filter(|c| !c.is_whitespace() && *c != '=').map(|c| c.to_ascii_uppercase()).collect();
    let key = data_encoding::BASE32_NOPAD.decode(clean.as_bytes()).map_err(|_| "kein gültiges Base32".to_owned())?;
    if key.len() < 10 {
        return Err("TOTP-Secret zu kurz (mindestens 80 Bit)".into());
    }
    Ok(key)
}

/// 6-digit HOTP of time step `step` (HMAC-SHA1, as every authenticator app does).
pub fn totp_code(key: &[u8], step: i64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&(step as u64).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 0x0f);
    let value =
        u32::from_be_bytes([digest[offset], digest[offset + 1], digest[offset + 2], digest[offset + 3]]) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

/// The step `code` matches (now ±1 step for clock drift), if any.
pub fn totp_match(key: &[u8], code: &str, now: i64) -> Option<i64> {
    let current = now / TOTP_STEP;
    (current - 1..=current + 1).find(|&step| crate::same(&totp_code(key, step), code))
}

/// A fresh secret for `UWUSTATUS_ADMIN_TOTP_SECRET_FILE`.
pub fn new_totp_secret() -> String {
    data_encoding::BASE32_NOPAD.encode(&rand::random::<[u8; 20]>())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    user: String,
    password: String,
    totp: Option<String>,
}

pub async fn login(State(state): State<AppState>, Body(login): Body<Login>) -> ApiResult<Response> {
    let admin = state.config.admin.as_ref().ok_or_else(|| ApiError::not_found("Lokale Anmeldung ist aus."))?;
    if !state.local_failures.has_room() {
        return Err(ApiError::rate_limited("Zu viele Fehlversuche, bitte eine Minute warten."));
    }
    let hash = admin.password_hash.expose().to_owned();
    let password = login.password;
    // Argon2 takes a while; keep it off the async workers.
    let password_ok =
        tokio::task::spawn_blocking(move || verify_password(&hash, &password)).await.map_err(ApiError::internal)?;
    let user_ok = crate::same(&login.user, &admin.user);
    let totp_ok = match &admin.totp {
        None => true,
        Some(secret) => {
            let key = totp_key(secret.expose()).map_err(ApiError::internal)?;
            let code = login.totp.as_deref().unwrap_or_default().trim();
            match totp_match(&key, code, crate::now()) {
                // Each code works once: the step must be newer than the last one used.
                Some(step) if user_ok && password_ok => {
                    let mut last = state.totp_last_step.lock().unwrap_or_else(|e| e.into_inner());
                    let fresh = step > *last;
                    if fresh {
                        *last = step;
                    }
                    fresh
                }
                Some(_) => true,
                None => false,
            }
        }
    };
    if !(user_ok && password_ok && totp_ok) {
        state.local_failures.hit();
        tracing::warn!("local login failed for user {:?}", login.user.chars().take(64).collect::<String>());
        return Err(ApiError::forbidden("Anmeldung fehlgeschlagen."));
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    start_session(&state, &mut response, &format!("local:{}", admin.user), &admin.user, None, "local")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors() {
        // RFC 6238 appendix B, SHA-1 key; the 8-digit codes cut to 6.
        let key = b"12345678901234567890";
        for (time, code) in [(59, "287082"), (1111111109, "081804"), (1234567890, "005924"), (2000000000, "279037")] {
            assert_eq!(totp_code(key, time / 30), code, "t={time}");
        }
    }

    #[test]
    fn totp_window_and_key() {
        let secret = new_totp_secret();
        let key = totp_key(&secret.to_lowercase()).unwrap();
        let now = 1_700_000_000;
        assert_eq!(totp_match(&key, &totp_code(&key, now / 30 - 1), now), Some(now / 30 - 1));
        assert_eq!(totp_match(&key, &totp_code(&key, now / 30 - 2), now), None);
        assert!(totp_key("not base32!").is_err());
        assert!(totp_key("AAAA").is_err());
    }

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("correct horse", Params::new(8, 1, 1, None).unwrap()).unwrap();
        check_hash(&hash).unwrap();
        assert!(verify_password(&hash, "correct horse"));
        assert!(!verify_password(&hash, "wrong horse"));
        assert!(check_hash("$argon2id$nope").is_err());
    }
}
