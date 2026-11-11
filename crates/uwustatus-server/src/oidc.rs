//! OpenID Connect relying party: discovery, authorization URL (PKCE S256), code exchange and
//! ID-token validation. Server-to-server calls can go to a backchannel origin; the issuer is
//! still checked against the configured one.

use crate::config::OidcConfig;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

#[derive(Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

/// What the app uses from the ID token.
#[derive(Debug, Clone)]
pub struct Identity {
    pub sub: String,
    pub name: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
    pub auth_time: Option<i64>,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    aud: Value,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    auth_time: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    groups: Value,
}

pub struct Oidc {
    http: reqwest::Client,
    config: OidcConfig,
    discovery: Mutex<Option<Discovery>>,
    jwks: Mutex<Option<JwkSet>>,
}

fn same_issuer(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

impl Oidc {
    pub fn new(http: reqwest::Client, config: OidcConfig) -> Self {
        Self { http, config, discovery: Mutex::new(None), jwks: Mutex::new(None) }
    }

    /// `url` with its origin swapped for the backchannel, if one is configured.
    fn backchannel(&self, url: &str) -> Result<String, String> {
        let Some(back) = &self.config.backchannel else { return Ok(url.to_owned()) };
        let mut url = url::Url::parse(url).map_err(|_| "OIDC: ungültige Endpunkt-URL".to_owned())?;
        let back = url::Url::parse(back).map_err(|_| "OIDC: ungültige Backchannel-URL".to_owned())?;
        url.set_scheme(back.scheme()).ok();
        url.set_host(back.host_str()).map_err(|_| "OIDC: ungültige Backchannel-URL".to_owned())?;
        url.set_port(back.port()).ok();
        Ok(url.to_string())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str, what: &str) -> Result<T, String> {
        let response = self.http.get(url).send().await.map_err(|e| format!("OIDC {what}: {}", e.without_url()))?;
        if !response.status().is_success() {
            return Err(format!("OIDC {what}: HTTP {}", response.status().as_u16()));
        }
        response.json().await.map_err(|_| format!("OIDC {what}: unlesbare Antwort"))
    }

    async fn discovery(&self) -> Result<Discovery, String> {
        let mut cached = self.discovery.lock().await;
        if let Some(d) = cached.as_ref() {
            return Ok(d.clone());
        }
        let url = format!("{}/.well-known/openid-configuration", self.config.issuer.trim_end_matches('/'));
        let d: Discovery = self.get_json(&self.backchannel(&url)?, "Discovery").await?;
        if !same_issuer(&d.issuer, &self.config.issuer) {
            return Err("OIDC: Issuer der Discovery passt nicht zur Konfiguration".into());
        }
        let issuer_origin = crate::config::origin_of(&d.issuer, "issuer")?;
        for endpoint in [&d.token_endpoint, &d.jwks_uri, &d.authorization_endpoint] {
            if crate::config::origin_of(endpoint, "OIDC-Endpunkt")? != issuer_origin {
                return Err("OIDC: Endpunkte liegen nicht beim Issuer".into());
            }
        }
        *cached = Some(d.clone());
        Ok(d)
    }

    pub async fn authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        verifier: &str,
        step_up: bool,
    ) -> Result<String, String> {
        let d = self.discovery().await?;
        let mut url = url::Url::parse(&d.authorization_endpoint).map_err(|_| "OIDC: ungültiger Endpunkt".to_owned())?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("scope", "openid profile email groups")
                .append_pair("state", state)
                .append_pair("nonce", nonce)
                .append_pair("code_challenge", &crate::sha256_b64(verifier))
                .append_pair("code_challenge_method", "S256");
            if step_up {
                q.append_pair("prompt", "login").append_pair("max_age", "0");
            }
        }
        Ok(url.into())
    }

    async fn key(&self, d: &Discovery, kid: &str) -> Result<DecodingKey, String> {
        let mut jwks = self.jwks.lock().await;
        for refetch in [false, true] {
            if refetch || jwks.is_none() {
                *jwks = Some(self.get_json(&self.backchannel(&d.jwks_uri)?, "JWKS").await?);
            }
            if let Some(jwk) = jwks.as_ref().and_then(|set| set.find(kid)) {
                return DecodingKey::from_jwk(jwk).map_err(|_| "OIDC: Schlüssel unbrauchbar".to_owned());
            }
        }
        Err("OIDC: unbekannter Signaturschlüssel".into())
    }

    /// Exchanges the code and validates the ID token (signature, iss, aud/azp, exp, nonce).
    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        nonce: &str,
    ) -> Result<Identity, String> {
        let d = self.discovery().await?;
        let response = self
            .http
            .post(self.backchannel(&d.token_endpoint)?)
            .basic_auth(&self.config.client_id, Some(self.config.client_secret.expose()))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|e| format!("OIDC Token: {}", e.without_url()))?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|_| "OIDC Token: unlesbare Antwort".to_owned())?;
        if !status.is_success() {
            let error = body.get("error").and_then(Value::as_str).unwrap_or("unbekannt");
            return Err(format!("OIDC Token: {error} (HTTP {})", status.as_u16()));
        }
        let token = body.get("id_token").and_then(Value::as_str).ok_or("OIDC Token: kein id_token")?;
        let header = jsonwebtoken::decode_header(token).map_err(|_| "OIDC: ID-Token unlesbar".to_owned())?;
        if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
            return Err("OIDC: Signaturverfahren nicht erlaubt".into());
        }
        let key = self.key(&d, header.kid.as_deref().unwrap_or_default()).await?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&d.issuer]);
        validation.set_audience(&[&self.config.client_id]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = 60;
        let claims = jsonwebtoken::decode::<IdClaims>(token, &key, &validation)
            .map_err(|e| format!("OIDC: ID-Token ungültig ({:?})", e.kind()))?
            .claims;
        if !claims.nonce.as_deref().is_some_and(|n| crate::same(n, nonce)) {
            return Err("OIDC: Nonce passt nicht".into());
        }
        if claims.aud.as_array().is_some_and(|a| a.len() > 1)
            && claims.azp.as_deref() != Some(self.config.client_id.as_str())
        {
            return Err("OIDC: azp passt nicht".into());
        }
        let groups = match &claims.groups {
            Value::Array(items) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
            Value::String(one) => vec![one.clone()],
            _ => Vec::new(),
        };
        let email = claims.email.filter(|e| !e.is_empty());
        let name = [&claims.name, &claims.preferred_username, &email]
            .into_iter()
            .flatten()
            .find(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| claims.sub.clone());
        Ok(Identity { sub: claims.sub, name, email, groups, auth_time: claims.auth_time })
    }
}
