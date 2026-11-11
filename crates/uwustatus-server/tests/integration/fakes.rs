//! A local OIDC provider (discovery, JWKS, token) at 127.0.0.1:0.

use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub const CLIENT_ID: &str = "uwustatus";
pub const CLIENT_SECRET: &str = "oidc-test-client-secret-0123456789";

pub struct Fake {
    pub base: String,
    /// Claims for the next ID token; `nonce` comes from the login redirect.
    pub claims: Mutex<Value>,
    key: (Vec<u8>, Value),
}

pub type FakeState = Arc<Fake>;

async fn discovery(State(f): State<FakeState>) -> Json<Value> {
    let issuer = format!("{}/oidc", f.base);
    Json(json!({
        "issuer": issuer, "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"), "jwks_uri": format!("{issuer}/jwks"),
    }))
}

async fn jwks(State(f): State<FakeState>) -> Json<Value> {
    Json(json!({ "keys": [f.key.1.clone()] }))
}

async fn token(State(f): State<FakeState>, headers: HeaderMap) -> Response {
    let expected =
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}")));
    if headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()) != Some(expected.as_str()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid_client" }))).into_response();
    }
    let now = jiff::Timestamp::now().as_second();
    let mut claims = json!({ "iss": format!("{}/oidc", f.base), "aud": CLIENT_ID, "iat": now, "exp": now + 300 });
    for (k, v) in f.claims.lock().unwrap().as_object().unwrap() {
        claims[k] = v.clone();
    }
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some("k1".into());
    let id_token = jsonwebtoken::encode(&header, &claims, &jsonwebtoken::EncodingKey::from_ec_der(&f.key.0)).unwrap();
    Json(json!({ "access_token": "at", "token_type": "Bearer", "id_token": id_token })).into_response()
}

fn signing_key() -> (Vec<u8>, Value) {
    let pair = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap();
    let public = pair.public_key().as_ref().to_vec();
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let jwk = json!({ "kty": "EC", "crv": "P-256", "x": b64(&public[1..33]), "y": b64(&public[33..65]),
                      "kid": "k1", "alg": "ES256", "use": "sig" });
    (pair.to_pkcs8v1().unwrap().as_ref().to_vec(), jwk)
}

pub async fn start() -> FakeState {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fake = Arc::new(Fake { base, claims: Mutex::new(json!({})), key: signing_key() });
    let app = Router::new()
        .route("/oidc/.well-known/openid-configuration", get(discovery))
        .route("/oidc/jwks", get(jwks))
        .route("/oidc/token", post(token))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    fake
}
