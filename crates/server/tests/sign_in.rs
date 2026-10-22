//! Signing in against a stand-in identity provider served over real HTTP. Discovery, the key set
//! and the code exchange go from reqwest to an axum server, and its ID tokens are signed with real
//! keys: `tests/fixtures/`, generated for these tests and used nowhere else.
//!
//! The provider checks what a real one does: the client's credentials, the code, the redirect URI
//! and the PKCE verifier. Each test that makes it misbehave checks that nobody was signed in.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use remote_proto::account::{
    Credentials, DeviceKind, LoginParams, NewDevice, Secret, TokenRequest,
};
use remote_proto::{DeviceId, ErrorCode, Platform, RemoteError, ServerId};
use serde_json::{json, Value};
use url::Url;
use yardsort_server::oidc::{self, ProviderConfig, KEYS_REFETCH_MS};
use yardsort_server::records::MemoryRecords;
use yardsort_server::{
    tokens, Caller, ControlService, LoginFinish, ServerConfig, ACCESS_TTL_MS, HANDOFF_TTL_MS,
    LOGIN_TTL_MS, REFRESH_TTL_MS,
};

const CLIENT_ID: &str = "yardsort-server";
/// A provider such as ZITADEL puts its project in `aud` beside the client, and the client in `azp`.
const PROJECT: &str = "project-at-the-provider";
const APP: &str = "yardsort://signed-in";

const RSA_A: &[u8] = include_bytes!("fixtures/rsa-a.der");
const RSA_B: &[u8] = include_bytes!("fixtures/rsa-b.der");
const EC: &[u8] = include_bytes!("fixtures/ec-p256.der");

fn jwk(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/{name}.jwk.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// How the provider gets its ID token wrong, if it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tamper {
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
struct Signer {
    kid: &'static str,
    alg: Algorithm,
    key: EncodingKey,
}

struct Grant {
    challenge: String,
    redirect_uri: String,
    nonce: String,
    subject: String,
}

struct IdpState {
    base: String,
    secret: Option<String>,
    signer: Signer,
    published: Vec<Value>,
    discovery_issuer: Option<String>,
    tamper: Tamper,
    grants: HashMap<String, Grant>,
    token_calls: usize,
    jwks_calls: usize,
}

type Shared = Arc<Mutex<IdpState>>;

struct Idp {
    state: Shared,
}

impl Idp {
    async fn start(secret: Option<&str>) -> Self {
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
        }));
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { state }
    }

    fn issuer(&self) -> String {
        self.state.lock().unwrap().base.clone()
    }

    fn with<T>(&self, f: impl FnOnce(&mut IdpState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// What happens in the browser at the provider: the person signs in as `subject`, and the
    /// provider sends the browser back to the server with a code and the server's `state`.
    fn approve(&self, authorize: &Url, subject: &str) -> (String, String) {
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

    fn calls(&self) -> (usize, usize) {
        self.with(|s| (s.token_calls, s.jwks_calls))
    }
}

async fn discovery(State(state): State<Shared>) -> Json<Value> {
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

async fn jwks(State(state): State<Shared>) -> Json<Value> {
    let mut s = state.lock().unwrap();
    s.jwks_calls += 1;
    Json(json!({ "keys": s.published }))
}

async fn token(
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

fn basic(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let decoded = STANDARD.decode(value.strip_prefix("Basic ")?).ok()?;
    let (id, secret) = std::str::from_utf8(&decoded).ok()?.split_once(':')?;
    Some((id.to_owned(), secret.to_owned()))
}

fn encoded(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

fn id_token(s: &IdpState, grant: &Grant) -> String {
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

struct Setup {
    idp: Idp,
    service: ControlService<MemoryRecords>,
}

async fn setup(secret: Option<&str>) -> Setup {
    let idp = Idp::start(secret).await;
    let config = ServerConfig {
        server: ServerId::new("srv-test").unwrap(),
        name: "Test server".into(),
        public_url: Url::parse("https://yardsort.example.invalid").unwrap(),
        provider: ProviderConfig {
            issuer: idp.issuer(),
            client_id: CLIENT_ID.into(),
            client_secret: secret.map(Secret::new),
        },
        device_redirects: vec![APP.into(), "http://127.0.0.1/signed-in".into()],
    };
    let http = oidc::http_client_builder().no_proxy().build().unwrap();
    let service = ControlService::new(config, MemoryRecords::default(), http).unwrap();
    Setup { idp, service }
}

fn device(name: &str) -> NewDevice {
    NewDevice {
        name: name.into(),
        kind: DeviceKind::Phone,
        platform: Platform::Android,
    }
}

/// A device's side of starting: its verifier, kept, and the sign-in it asked for.
fn params(verifier: &str, redirect: &str) -> LoginParams {
    LoginParams {
        challenge: tokens::s256(verifier),
        challenge_method: "S256".into(),
        redirect: redirect.into(),
    }
}

fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Through the browser: start, sign in at the provider as `subject`, and come back. The device's
/// verifier and the one-time code its redirect received.
async fn browse(s: &Setup, subject: &str, now: i64) -> (String, String) {
    let verifier = tokens::random();
    let authorize = s
        .service
        .begin_login(&params(&verifier, APP), now)
        .await
        .unwrap();
    let (code, state) = s.idp.approve(&authorize, subject);
    match s
        .service
        .finish_login(Some(&code), Some(&state), None, now)
        .await
    {
        LoginFinish::SignedIn(back) => {
            assert!(back.as_str().starts_with(APP), "{back}");
            (verifier, query(&back, "code").unwrap())
        }
        other => panic!("{other:?}"),
    }
}

async fn redeem(
    s: &Setup,
    code: &str,
    verifier: &str,
    name: &str,
    now: i64,
) -> Result<Credentials, RemoteError> {
    s.service
        .token(
            &TokenRequest::SignIn {
                code: Secret::new(code),
                verifier: Secret::new(verifier),
                device: device(name),
            },
            now,
        )
        .await
}

async fn sign_in(s: &Setup, subject: &str, name: &str, now: i64) -> Credentials {
    let (verifier, code) = browse(s, subject, now).await;
    redeem(s, &code, &verifier, name, now).await.unwrap()
}

async fn refresh(s: &Setup, refresh: &Secret, now: i64) -> Result<Credentials, RemoteError> {
    s.service
        .token(
            &TokenRequest::Refresh {
                refresh: refresh.clone(),
            },
            now,
        )
        .await
}

async fn caller(s: &Setup, credentials: &Credentials, now: i64) -> Result<Caller, RemoteError> {
    s.service
        .authenticate(credentials.access.expose(), now)
        .await
}

fn code_of<T: std::fmt::Debug>(result: Result<T, RemoteError>) -> ErrorCode {
    result.unwrap_err().code
}

#[tokio::test]
async fn signing_in_asks_the_provider_once_and_the_device_then_holds_the_servers_own_credentials() {
    let s = setup(Some("a secret: with % and +")).await;
    let now = now_ms();

    // The provider is sent only the server's own callback.
    let verifier = tokens::random();
    let authorize = s
        .service
        .begin_login(&params(&verifier, APP), now)
        .await
        .unwrap();
    assert_eq!(
        query(&authorize, "redirect_uri").as_deref(),
        Some(s.service.callback_url().as_str())
    );
    assert_eq!(
        s.service.callback_url().as_str(),
        "https://yardsort.example.invalid/v1/login/callback"
    );
    assert_ne!(
        query(&authorize, "code_challenge"),
        Some(tokens::s256(&verifier)),
        "the device's challenge and the provider's are separate"
    );
    let (code, state) = s.idp.approve(&authorize, "subject-1");
    let LoginFinish::SignedIn(back) = s
        .service
        .finish_login(Some(&code), Some(&state), None, now)
        .await
    else {
        panic!()
    };
    let pairs: Vec<_> = back.query_pairs().map(|(k, _)| k.into_owned()).collect();
    assert_eq!(pairs, ["code"], "only the one-time code goes to the device");
    let credentials = redeem(&s, &query(&back, "code").unwrap(), &verifier, "Phone", now)
        .await
        .unwrap();
    assert!(credentials.access.expose().starts_with("ys1.at."));
    assert_eq!(credentials.access_expires_in_ms, ACCESS_TTL_MS as u64);

    let me = caller(&s, &credentials, now).await.unwrap();
    assert_eq!(me.device, credentials.device);
    let page = s.service.devices(&me).await.unwrap();
    assert_eq!(page.devices.len(), 1);
    assert!(page.devices[0].current);
    assert_eq!(page.devices[0].name, "Phone");
    assert_eq!(page.devices[0].last_seen_at, Some(now));
    assert_eq!(s.idp.calls(), (1, 1));

    // A month of use: refreshing and calling never reach the provider, so a provider that bills
    // by active user counts this device once, for the day it signed in.
    let mut held = credentials;
    for day in 1..=30 {
        let at = now + day * 24 * 60 * 60_000;
        held = refresh(&s, &held.refresh, at).await.unwrap();
        caller(&s, &held, at).await.unwrap();
    }
    assert_eq!(
        s.idp.calls(),
        (1, 1),
        "the provider heard from no device after sign-in"
    );
}

#[tokio::test]
async fn a_public_client_sends_its_id_and_no_secret() {
    let s = setup(None).await;
    let credentials = sign_in(&s, "subject-1", "Phone", now_ms()).await;
    assert!(caller(&s, &credentials, now_ms()).await.is_ok());
}

#[tokio::test]
async fn an_account_is_a_subject_and_sees_and_revokes_only_its_own_devices() {
    let s = setup(None).await;
    let now = now_ms();
    let phone = sign_in(&s, "subject-1", "Phone", now).await;
    let laptop = sign_in(&s, "subject-1", "Laptop", now + 1).await;
    let theirs = sign_in(&s, "subject-2", "Their phone", now + 2).await;
    assert_eq!(s.service.records().account_count(), 2);

    let me = caller(&s, &phone, now).await.unwrap();
    let mine = s.service.devices(&me).await.unwrap();
    let names: Vec<_> = mine.devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        ["Laptop", "Phone"],
        "same subject, same account; newest first"
    );

    let them = caller(&s, &theirs, now).await.unwrap();
    let their_list = s.service.devices(&them).await.unwrap();
    assert_eq!(their_list.devices.len(), 1);
    assert_eq!(their_list.devices[0].id, theirs.device);

    // Another account's device is not found, exactly as a device that does not exist, and is
    // left working.
    let missing = DeviceId::new("no-such-device").unwrap();
    let not_mine = s.service.revoke(&me, &theirs.device, now).await;
    let nothing = s.service.revoke(&me, &missing, now).await;
    assert_eq!(not_mine, nothing);
    assert_eq!(code_of(not_mine), ErrorCode::NotFound);
    assert!(caller(&s, &theirs, now).await.is_ok());
    assert!(refresh(&s, &theirs.refresh, now).await.is_ok());

    // Its own other device it can revoke, and that device stops at once.
    s.service.revoke(&me, &laptop.device, now).await.unwrap();
    assert_eq!(
        code_of(caller(&s, &laptop, now).await),
        ErrorCode::Unauthenticated
    );
    assert_eq!(
        code_of(refresh(&s, &laptop.refresh, now).await),
        ErrorCode::Unauthenticated
    );
    let listed = s.service.devices(&me).await.unwrap();
    let laptop_row = listed
        .devices
        .iter()
        .find(|d| d.id == laptop.device)
        .unwrap();
    assert_eq!(laptop_row.revoked_at, Some(now));
    assert!(caller(&s, &phone, now).await.is_ok());
}

#[tokio::test]
async fn a_sign_in_code_works_once_briefly_and_only_with_its_verifier() {
    let s = setup(None).await;
    let now = now_ms();

    let (verifier, code) = browse(&s, "subject-1", now).await;
    let wrong = tokens::random();
    assert_eq!(
        code_of(redeem(&s, &code, &wrong, "Phone", now).await),
        ErrorCode::Unauthenticated
    );
    assert_eq!(
        code_of(redeem(&s, &code, &verifier, "Phone", now).await),
        ErrorCode::Unauthenticated,
        "a wrong verifier spends the code: there is no second guess"
    );

    let (verifier, code) = browse(&s, "subject-1", now).await;
    assert!(redeem(&s, &code, &verifier, "Phone", now).await.is_ok());
    assert!(
        redeem(&s, &code, &verifier, "Phone", now).await.is_err(),
        "used once"
    );

    let (verifier, code) = browse(&s, "subject-1", now).await;
    assert!(
        redeem(&s, &code, &verifier, "Phone", now + HANDOFF_TTL_MS)
            .await
            .is_err(),
        "a minute at most"
    );

    // Something that is not a sign-in code at all, such as a refresh credential.
    let credentials = sign_in(&s, "subject-1", "Phone", now).await;
    assert!(
        redeem(&s, credentials.refresh.expose(), &verifier, "Phone", now)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_providers_answer_counts_once_and_only_while_the_sign_in_is_fresh() {
    let s = setup(None).await;
    let now = now_ms();
    let start = || async {
        let authorize = s
            .service
            .begin_login(&params(&tokens::random(), APP), now)
            .await
            .unwrap();
        s.idp.approve(&authorize, "subject-1")
    };

    let (code, state) = start().await;
    assert!(matches!(
        s.service
            .finish_login(Some(&code), Some(&state), None, now)
            .await,
        LoginFinish::SignedIn(_)
    ));
    assert!(
        matches!(
            s.service
                .finish_login(Some(&code), Some(&state), None, now)
                .await,
            LoginFinish::Unknown(_)
        ),
        "the same answer twice"
    );
    assert!(matches!(
        s.service
            .finish_login(Some(&code), Some("a-state-never-issued"), None, now)
            .await,
        LoginFinish::Unknown(_)
    ));

    let (code, state) = start().await;
    let late = s
        .service
        .finish_login(Some(&code), Some(&state), None, now + LOGIN_TTL_MS)
        .await;
    let LoginFinish::Refused { to, .. } = late else {
        panic!("{late:?}")
    };
    assert_eq!(query(&to, "error").as_deref(), Some("unauthenticated"));
    assert_eq!(query(&to, "code"), None);

    // The person cancelled at the provider.
    let (_, state) = start().await;
    let cancelled = s
        .service
        .finish_login(None, Some(&state), Some("access_denied"), now)
        .await;
    let LoginFinish::Refused { to, why } = cancelled else {
        panic!("{cancelled:?}")
    };
    assert!(to.as_str().starts_with(APP));
    assert!(why.contains("access_denied"));
    assert_eq!(s.service.records().account_count(), 1);
}

#[tokio::test]
async fn a_sign_in_goes_back_only_to_an_allowed_redirect() {
    let s = setup(None).await;
    let now = now_ms();
    for redirect in [
        "https://attacker.example.invalid/catch",
        "yardsort://signed-in/elsewhere",
        "yardsort://signed-in#fragment",
        "http://localhost:4000/signed-in",
        "http://127.0.0.1:4000/other",
        "not a url",
    ] {
        let refused = s
            .service
            .begin_login(&params(&tokens::random(), redirect), now)
            .await;
        assert_eq!(code_of(refused), ErrorCode::InvalidRequest, "{redirect}");
    }
    assert_eq!(s.idp.calls(), (0, 0));

    // A computer's client on whatever port it found free.
    let verifier = tokens::random();
    let loopback = "http://127.0.0.1:49731/signed-in";
    let authorize = s
        .service
        .begin_login(&params(&verifier, loopback), now)
        .await
        .unwrap();
    let (code, state) = s.idp.approve(&authorize, "subject-1");
    let LoginFinish::SignedIn(back) = s
        .service
        .finish_login(Some(&code), Some(&state), None, now)
        .await
    else {
        panic!()
    };
    assert!(back.as_str().starts_with(loopback));
}

#[tokio::test]
async fn an_id_token_is_checked_before_anyone_is_signed_in() {
    for tamper in [
        Tamper::Issuer,
        Tamper::Audience,
        Tamper::OtherAzp,
        Tamper::SeveralAudiencesNoAzp,
        Tamper::Nonce,
        Tamper::NoNonce,
        Tamper::Expired,
        Tamper::FromTheFuture,
        Tamper::UnpublishedKey,
        Tamper::UnknownKeyId,
        Tamper::AlgNone,
        Tamper::HmacWithThePublicKey,
        Tamper::RsaSignatureUnderAnEcKeyId,
    ] {
        let s = setup(None).await;
        let now = now_ms();
        s.idp.with(|i| i.tamper = tamper);
        let authorize = s
            .service
            .begin_login(&params(&tokens::random(), APP), now)
            .await
            .unwrap();
        let (code, state) = s.idp.approve(&authorize, "subject-1");
        let finish = s
            .service
            .finish_login(Some(&code), Some(&state), None, now)
            .await;
        let LoginFinish::Refused { to, why } = finish else {
            panic!("{tamper:?} was accepted: {finish:?}")
        };
        assert_eq!(query(&to, "code"), None, "{tamper:?}");
        assert!(why.contains("ID token"), "{tamper:?}: {why}");
        assert_eq!(s.service.records().account_count(), 0, "{tamper:?}");
    }
}

#[tokio::test]
async fn either_kind_of_published_key_signs_a_person_in() {
    let s = setup(None).await;
    s.idp.with(|i| {
        i.signer = Signer {
            kid: "ec-p256",
            alg: Algorithm::ES256,
            key: EncodingKey::from_ec_der(EC),
        }
    });
    let credentials = sign_in(&s, "subject-1", "Phone", now_ms()).await;
    assert!(caller(&s, &credentials, now_ms()).await.is_ok());
}

#[tokio::test]
async fn new_provider_keys_are_fetched_but_unknown_ones_cannot_make_the_server_hammer_it() {
    let s = setup(None).await;
    let now = now_ms();
    sign_in(&s, "subject-1", "Phone", now).await;
    assert_eq!(s.idp.calls().1, 1);

    // The provider rotates to a key the server has not seen.
    s.idp.with(|i| {
        i.published.push(jwk("rsa-b"));
        i.signer = Signer {
            kid: "rsa-b",
            alg: Algorithm::RS256,
            key: EncodingKey::from_rsa_der(RSA_B),
        };
    });
    let later = now + KEYS_REFETCH_MS;
    sign_in(&s, "subject-1", "Phone", later).await;
    assert_eq!(s.idp.calls().1, 2, "fetched again for the new key");

    // Tokens naming a key nobody publishes, one after another: at most one fetch a minute.
    s.idp.with(|i| i.tamper = Tamper::UnknownKeyId);
    for n in 0..5 {
        let authorize = s
            .service
            .begin_login(&params(&tokens::random(), APP), later + KEYS_REFETCH_MS + n)
            .await
            .unwrap();
        let (code, state) = s.idp.approve(&authorize, "subject-1");
        let finish = s
            .service
            .finish_login(Some(&code), Some(&state), None, later + KEYS_REFETCH_MS + n)
            .await;
        assert!(matches!(finish, LoginFinish::Refused { .. }));
    }
    assert_eq!(s.idp.calls().1, 3);
}

#[tokio::test]
async fn a_provider_that_names_another_issuer_is_not_used() {
    let s = setup(None).await;
    s.idp
        .with(|i| i.discovery_issuer = Some("https://auth.example.invalid".into()));
    let refused = s
        .service
        .begin_login(&params(&tokens::random(), APP), now_ms())
        .await;
    assert_eq!(code_of(refused), ErrorCode::Internal);

    let config = |issuer: &str| ServerConfig {
        server: ServerId::new("srv-test").unwrap(),
        name: "Test server".into(),
        public_url: Url::parse("https://yardsort.example.invalid").unwrap(),
        provider: ProviderConfig {
            issuer: issuer.into(),
            client_id: CLIENT_ID.into(),
            client_secret: None,
        },
        device_redirects: vec![],
    };
    let http = oidc::http_client().unwrap();
    assert!(ControlService::new(
        config("http://auth.example.invalid"),
        MemoryRecords::default(),
        http.clone()
    )
    .is_err());
    assert!(ControlService::new(
        config("https://auth.example.invalid"),
        MemoryRecords::default(),
        http
    )
    .is_ok());
}

#[tokio::test]
async fn a_refresh_credential_works_once_and_a_copy_signs_the_device_out() {
    let s = setup(None).await;
    let now = now_ms();
    let first = sign_in(&s, "subject-1", "Phone", now).await;
    let other = sign_in(&s, "subject-1", "Laptop", now).await;

    let second = refresh(&s, &first.refresh, now + 1).await.unwrap();
    assert_eq!(second.device, first.device);
    assert_ne!(second.refresh, first.refresh);
    assert!(caller(&s, &second, now + 1).await.is_ok());

    // The spent one again: only a copy could hold it, so the device is signed out.
    assert_eq!(
        code_of(refresh(&s, &first.refresh, now + 2).await),
        ErrorCode::Unauthenticated
    );
    assert!(caller(&s, &second, now + 2).await.is_err());
    assert!(refresh(&s, &second.refresh, now + 2).await.is_err());
    let me = caller(&s, &other, now + 2).await.unwrap();
    let listed = s.service.devices(&me).await.unwrap();
    let phone = listed
        .devices
        .iter()
        .find(|d| d.id == first.device)
        .unwrap();
    assert_eq!(phone.revoked_at, Some(now + 2));
    assert!(
        caller(&s, &other, now + 2).await.is_ok(),
        "only that device"
    );

    // Lapsed: an access after its quarter hour, a refresh after its months unused.
    assert!(caller(&s, &other, now + ACCESS_TTL_MS).await.is_err());
    assert!(refresh(&s, &other.refresh, now + REFRESH_TTL_MS)
        .await
        .is_err());
    // Credentials of one kind are not accepted as another.
    assert!(s
        .service
        .authenticate(other.refresh.expose(), now)
        .await
        .is_err());
    assert!(refresh(&s, &other.access, now).await.is_err());
}
