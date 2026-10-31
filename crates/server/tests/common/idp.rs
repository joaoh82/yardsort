//! A stand-in identity provider served over real HTTP. Discovery, the key set and the code
//! exchange go from reqwest to an axum server, and its ID tokens are signed with real keys:
//! `tests/fixtures/`, generated for these tests and used nowhere else.
//!
//! It checks what a real one does: the client's credentials, the code, the redirect URI and the
//! PKCE verifier. [`Tamper`] makes it get its ID token wrong in one way at a time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use url::Url;
use yardsort_server::tokens;

pub const CLIENT_ID: &str = "yardsort-server";
/// A provider such as ZITADEL puts its project in `aud` beside the client, and the client in `azp`.
pub const PROJECT: &str = "project-at-the-provider";
pub const APP: &str = "yardsort://signed-in";

pub const RSA_A: &[u8] = include_bytes!("../fixtures/rsa-a.der");
pub const RSA_B: &[u8] = include_bytes!("../fixtures/rsa-b.der");
pub const EC: &[u8] = include_bytes!("../fixtures/ec-p256.der");

pub fn jwk(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/{name}.jwk.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// How the provider gets its ID token wrong, if it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tamper {
    Nothing,
    Issuer,
    Audience,
    OtherAzp,
    SeveralAudiencesNoAzp,
    Nonce,
    NoNonce,
    Expired,
    FromTheFuture,
    /// Signed with another key, under the id of the provider's own.
    UnpublishedKey,
    /// Signed under a key id the provider never published.
    UnknownKeyId,
    AlgNone,
    HmacWithThePublicKey,
    RsaSignatureUnderAnEcKeyId,
}

#[derive(Clone)]
pub struct Signer {
    pub kid: &'static str,
    pub alg: Algorithm,
    pub key: EncodingKey,
}

pub struct Grant {
    pub challenge: String,
    pub redirect_uri: String,
    pub nonce: String,
    pub subject: String,
}

pub struct IdpState {
    pub base: String,
    pub secret: Option<String>,
    pub signer: Signer,
    pub published: Vec<Value>,
    pub discovery_issuer: Option<String>,
    pub tamper: Tamper,
    pub grants: HashMap<String, Grant>,
    pub token_calls: usize,
    pub jwks_calls: usize,
    /// Who signs in at `/authorize`, where the browser comes when nothing calls [`Idp::approve`].
    pub subject: String,
}

pub type Shared = Arc<Mutex<IdpState>>;

pub struct Idp {
    pub state: Shared,
}

impl Idp {
    pub async fn start(secret: Option<&str>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(IdpState {
            base,
            secret: secret.map(str::to_owned),
            signer: Signer {
                kid: "rsa-a",
                alg: Algorithm::RS256,
                key: EncodingKey::from_rsa_der(RSA_A),
            },
            published: vec![jwk("rsa-a"), jwk("ec-p256")],
            discovery_issuer: None,
            tamper: Tamper::Nothing,
            grants: HashMap::new(),
            token_calls: 0,
            jwks_calls: 0,
            subject: "someone".into(),
        }));
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { state }
    }

    pub fn issuer(&self) -> String {
        self.state.lock().unwrap().base.clone()
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut IdpState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// What happens in the browser at the provider: the person signs in as `subject`, and the
    /// provider sends the browser back to the server with a code and the server's `state`.
    pub fn approve(&self, authorize: &Url, subject: &str) -> (String, String) {
        let q: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], CLIENT_ID);
        assert_eq!(
            q["scope"], "openid",
            "nothing beyond who signed in is asked for"
        );
        assert_eq!(q["code_challenge_method"], "S256");
        let code = tokens::random();
        self.with(|s| {
            assert!(authorize
                .as_str()
                .starts_with(&format!("{}/authorize?", s.base)));
            s.grants.insert(
                code.clone(),
                Grant {
                    challenge: q["code_challenge"].clone(),
                    redirect_uri: q["redirect_uri"].clone(),
                    nonce: q["nonce"].clone(),
                    subject: subject.to_owned(),
                },
            )
        });
        (code, q["state"].clone())
    }

    pub fn calls(&self) -> (usize, usize) {
        self.with(|s| (s.token_calls, s.jwks_calls))
    }
}

pub async fn discovery(State(state): State<Shared>) -> Json<Value> {
    let s = state.lock().unwrap();
    let base = &s.base;
    Json(json!({
        "issuer": s.discovery_issuer.clone().unwrap_or_else(|| base.clone()),
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "code_challenge_methods_supported": ["S256"],
    }))
}

/// A browser at the provider with [`IdpState::subject`] already signed in: sent straight back to
/// the server with a code, as [`Idp::approve`] does for tests that drive each step themselves.
pub async fn authorize(
    State(state): State<Shared>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let field = |name: &str| q.get(name).cloned().unwrap_or_default();
    if field("response_type") != "code" || field("client_id") != CLIENT_ID {
        return (StatusCode::BAD_REQUEST, "not this provider's client").into_response();
    }
    let code = tokens::random();
    let mut back = match Url::parse(&field("redirect_uri")) {
        Ok(url) => url,
        Err(_) => return (StatusCode::BAD_REQUEST, "no redirect").into_response(),
    };
    back.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &field("state"));
    let mut s = state.lock().unwrap();
    let subject = s.subject.clone();
    s.grants.insert(
        code,
        Grant {
            challenge: field("code_challenge"),
            redirect_uri: field("redirect_uri"),
            nonce: field("nonce"),
            subject,
        },
    );
    (StatusCode::SEE_OTHER, [("location", back.to_string())]).into_response()
}

pub async fn jwks(State(state): State<Shared>) -> Json<Value> {
    let mut s = state.lock().unwrap();
    s.jwks_calls += 1;
    Json(json!({ "keys": s.published }))
}

pub async fn token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut s = state.lock().unwrap();
    s.token_calls += 1;
    let refuse =
        |status: StatusCode, error: &str| (status, Json(json!({ "error": error }))).into_response();
    let client = match &s.secret {
        Some(secret) => basic(&headers) == Some((encoded(CLIENT_ID), encoded(secret))),
        None => {
            form.get("client_id").map(String::as_str) == Some(CLIENT_ID)
                && !headers.contains_key("authorization")
        }
    };
    if !client {
        return refuse(StatusCode::UNAUTHORIZED, "invalid_client");
    }
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return refuse(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }
    let Some(grant) = form.get("code").and_then(|code| s.grants.remove(code)) else {
        return refuse(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    if form.get("redirect_uri") != Some(&grant.redirect_uri)
        || tokens::s256(&verifier) != grant.challenge
    {
        return refuse(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let id_token = id_token(&s, &grant);
    Json(json!({
        "access_token": "the-providers-own-access-token",
        "token_type": "Bearer",
        "expires_in": 3600,
        "id_token": id_token,
    }))
    .into_response()
}

pub fn basic(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let decoded = STANDARD.decode(value.strip_prefix("Basic ")?).ok()?;
    let (id, secret) = std::str::from_utf8(&decoded).ok()?.split_once(':')?;
    Some((id.to_owned(), secret.to_owned()))
}

pub fn encoded(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

pub fn id_token(s: &IdpState, grant: &Grant) -> String {
    let now = now_ms() / 1000;
    let mut claims = json!({
        "iss": s.base,
        "sub": grant.subject,
        "aud": [CLIENT_ID, PROJECT],
        "azp": CLIENT_ID,
        "exp": now + 3600,
        "iat": now,
        "auth_time": now,
        "nonce": grant.nonce,
        "email": "someone@example.invalid",
    });
    let c = claims.as_object_mut().unwrap();
    match s.tamper {
        Tamper::Issuer => {
            c.insert("iss".into(), json!(format!("{}/elsewhere", s.base)));
        }
        Tamper::Audience => {
            c.insert("aud".into(), json!(["another-client"]));
            c.insert("azp".into(), json!("another-client"));
        }
        Tamper::OtherAzp => {
            c.insert("azp".into(), json!("another-client"));
        }
        Tamper::SeveralAudiencesNoAzp => {
            c.remove("azp");
        }
        Tamper::Nonce => {
            c.insert("nonce".into(), json!("a-nonce-from-another-sign-in"));
        }
        Tamper::NoNonce => {
            c.remove("nonce");
        }
        Tamper::Expired => {
            c.insert("iat".into(), json!(now - 7200));
            c.insert("exp".into(), json!(now - 3600));
        }
        Tamper::FromTheFuture => {
            c.insert("iat".into(), json!(now + 3600));
            c.insert("exp".into(), json!(now + 7200));
        }
        _ => {}
    }
    let sign = |kid: &str, alg: Algorithm, key: &EncodingKey| {
        let mut header = Header::new(alg);
        header.kid = Some(kid.to_owned());
        jsonwebtoken::encode(&header, &claims, key).unwrap()
    };
    match s.tamper {
        Tamper::UnpublishedKey => {
            sign("rsa-a", Algorithm::RS256, &EncodingKey::from_rsa_der(RSA_B))
        }
        Tamper::UnknownKeyId => sign(
            "nobody-publishes-this",
            Algorithm::RS256,
            &EncodingKey::from_rsa_der(RSA_B),
        ),
        Tamper::AlgNone => {
            let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","kid":"rsa-a"}"#);
            let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
            format!("{header}.{payload}.")
        }
        // The classic confusion: the published RSA key's bytes used as an HMAC secret.
        Tamper::HmacWithThePublicKey => {
            let modulus = URL_SAFE_NO_PAD
                .decode(jwk("rsa-a")["n"].as_str().unwrap())
                .unwrap();
            sign(
                "rsa-a",
                Algorithm::HS256,
                &EncodingKey::from_secret(&modulus),
            )
        }
        Tamper::RsaSignatureUnderAnEcKeyId => sign(
            "ec-p256",
            Algorithm::RS256,
            &EncodingKey::from_rsa_der(RSA_A),
        ),
        _ => sign(s.signer.kid, s.signer.alg, &s.signer.key),
    }
}
