//! A device's side of the Yardsort server's HTTP API ([`remote_proto::account`] and
//! [`remote_proto::pairing`]).
//!
//! **Signing in** goes through the system browser and comes back to a listener on this machine's
//! loopback address, on whichever port was free ([`SignIn`]). The browser never carries a
//! credential: it brings back a one-time code that is worth nothing without the verifier this
//! process kept.
//!
//! **Credentials** are handed to the caller and never kept here. The refresh credential is the one
//! that matters: each works once, and presenting a spent one signs the device out, so a caller
//! must keep each new one ([`Session::access`] says how).
//!
//! **The relay** ([`Client::open_relay`]) is a WebSocket opened with a single-use ticket, for a
//! host's own link or a device's way to a host.
//!
//! Only HTTPS servers are spoken to, apart from a server on a loopback address written as one
//! (`http://127.0.0.1:8787`), which is how a development server runs. Certificates are checked
//! against the operating system's roots; there is no switch to skip that.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use remote_proto::account::{
    Credentials, DevicePage, NewDevice, Secret, ServerDescription, TokenRequest, CHALLENGE_METHOD,
};
use remote_proto::pairing::{
    GrantDecision, GrantPage, GrantSummary, HostPage, HostRegistration, HostSummary, Offer,
    PairingRequest,
};
use remote_proto::{DeviceId, ErrorCode, GrantId, HostId, RemoteError};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

mod relay;
pub use relay::{Incoming, LinkReader, LinkWriter, RelayLink};

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long a person has to sign in at the provider. The server keeps a sign-in for ten minutes.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);
/// An access credential is refreshed this long before it would lapse.
const REFRESH_MARGIN_MS: u64 = 60_000;
/// How long a connection to the loopback listener has to send its request. Connections are
/// served one at a time, so one that sends nothing must not hold up the browser's.
const RETURN_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The server refused, with its reason.
    #[error("{0}")]
    Remote(RemoteError),
    #[error("Could not reach the server: {0}")]
    Unreachable(String),
    /// The server answered something this build does not understand.
    #[error("The server's answer was not understood: {0}")]
    Unexpected(String),
    #[error("{0}")]
    Invalid(String),
    #[error("The sign-in did not come back in time. Start again.")]
    TimedOut,
    /// The server issued a new refresh credential and the caller could not keep it. The session
    /// still holds it, so it works until this process ends.
    #[error("The new sign-in could not be kept: {0}")]
    NotKept(String),
}

impl ClientError {
    /// The server's reason, when it gave one.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Remote(error) => Some(error.code),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// One server, as a device speaks to it.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    /// Ends in `/`, so paths join onto it.
    base: Url,
}

impl Client {
    /// A client of the server at `address`: `https://remote.example.com`, or a loopback address
    /// for a development server.
    pub fn new(address: &str) -> Result<Self> {
        let mut base = Url::parse(address.trim())
            .map_err(|e| ClientError::Invalid(format!("{address:?} is not an address: {e}")))?;
        if !secure_enough(&base) {
            return Err(ClientError::Invalid(
                "A server's address starts with https:// (plain http only on 127.0.0.1 or [::1])."
                    .into(),
            ));
        }
        if base.query().is_some() || base.fragment().is_some() || !base.username().is_empty() {
            return Err(ClientError::Invalid(
                "A server's address has no query, fragment or user name.".into(),
            ));
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .user_agent(concat!("Yardsort/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ClientError::Invalid(chain(&e)))?;
        Ok(Self { http, base })
    }

    /// The server's address, as given, ending in `/`.
    pub fn address(&self) -> &Url {
        &self.base
    }

    /// `GET /v1/server`.
    pub async fn describe(&self) -> Result<ServerDescription> {
        self.send(self.http.get(self.url("v1/server")?)).await
    }

    /// Start signing in: listen on a free loopback port, and make the address to open in the
    /// browser. [`SignIn::finish`] waits for it to come back.
    pub async fn begin_sign_in(&self) -> Result<SignIn> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| ClientError::Invalid(format!("listening for the sign-in: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| ClientError::Invalid(e.to_string()))?
            .port();
        let verifier = random();
        let mut url = self.url("v1/login")?;
        url.query_pairs_mut()
            .append_pair("challenge", &s256(&verifier))
            .append_pair("challengeMethod", CHALLENGE_METHOD)
            .append_pair("redirect", &format!("http://127.0.0.1:{port}/signed-in"));
        Ok(SignIn {
            url,
            listener,
            verifier: Secret::new(verifier),
        })
    }

    /// `POST /v1/token` with a refresh credential. The answer's refresh credential replaces it.
    pub async fn refresh(&self, refresh: &Secret) -> Result<Credentials> {
        let request = TokenRequest::Refresh {
            refresh: refresh.clone(),
        };
        self.send(self.http.post(self.url("v1/token")?).json(&request))
            .await
    }

    /// `GET /v1/devices`.
    pub async fn devices(&self, access: &Secret) -> Result<DevicePage> {
        self.send(self.authed(self.http.get(self.url("v1/devices")?), access))
            .await
    }

    /// `POST /v1/devices/{device}/revoke`.
    pub async fn revoke_device(&self, access: &Secret, device: &DeviceId) -> Result<()> {
        let url = self.url(&format!("v1/devices/{device}/revoke"))?;
        self.send_empty(self.authed(self.http.post(url), access))
            .await
    }

    /// `POST /v1/hosts`.
    pub async fn register_host(
        &self,
        access: &Secret,
        registration: &HostRegistration,
    ) -> Result<HostSummary> {
        let request = self.http.post(self.url("v1/hosts")?).json(registration);
        self.send(self.authed(request, access)).await
    }

    /// `GET /v1/hosts`.
    pub async fn hosts(&self, access: &Secret) -> Result<HostPage> {
        self.send(self.authed(self.http.get(self.url("v1/hosts")?), access))
            .await
    }

    /// `POST /v1/hosts/{host}/offers`.
    pub async fn offer(&self, access: &Secret, host: &HostId) -> Result<Offer> {
        let url = self.url(&format!("v1/hosts/{host}/offers"))?;
        self.send(self.authed(self.http.post(url), access)).await
    }

    /// `GET /v1/hosts/{host}/grants`.
    pub async fn host_grants(&self, access: &Secret, host: &HostId) -> Result<GrantPage> {
        let url = self.url(&format!("v1/hosts/{host}/grants"))?;
        self.send(self.authed(self.http.get(url), access)).await
    }

    /// `POST /v1/hosts/{host}/remove`.
    pub async fn remove_host(&self, access: &Secret, host: &HostId) -> Result<()> {
        let url = self.url(&format!("v1/hosts/{host}/remove"))?;
        self.send_empty(self.authed(self.http.post(url), access))
            .await
    }

    /// `POST /v1/grants/{grant}/approve` with a decision, `…/deny` without one.
    pub async fn decide(
        &self,
        access: &Secret,
        grant: &GrantId,
        decision: Option<&GrantDecision>,
    ) -> Result<GrantSummary> {
        let request = match decision {
            Some(decision) => self
                .http
                .post(self.url(&format!("v1/grants/{grant}/approve"))?)
                .json(decision),
            None => self
                .http
                .post(self.url(&format!("v1/grants/{grant}/deny"))?),
        };
        self.send(self.authed(request, access)).await
    }

    /// `POST /v1/pairings`.
    pub async fn pair(&self, access: &Secret, request: &PairingRequest) -> Result<GrantSummary> {
        let request = self.http.post(self.url("v1/pairings")?).json(request);
        self.send(self.authed(request, access)).await
    }

    /// `GET /v1/grants/{grant}`.
    pub async fn grant(&self, access: &Secret, grant: &GrantId) -> Result<GrantSummary> {
        let url = self.url(&format!("v1/grants/{grant}"))?;
        self.send(self.authed(self.http.get(url), access)).await
    }

    /// `GET /v1/grants`.
    pub async fn grants(&self, access: &Secret) -> Result<GrantPage> {
        self.send(self.authed(self.http.get(self.url("v1/grants")?), access))
            .await
    }

    /// `POST /v1/grants/{grant}/revoke`.
    pub async fn revoke_grant(&self, access: &Secret, grant: &GrantId) -> Result<()> {
        let url = self.url(&format!("v1/grants/{grant}/revoke"))?;
        self.send_empty(self.authed(self.http.post(url), access))
            .await
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base
            .join(path)
            .map_err(|e| ClientError::Invalid(e.to_string()))
    }

    fn authed(&self, request: reqwest::RequestBuilder, access: &Secret) -> reqwest::RequestBuilder {
        request.bearer_auth(access.expose())
    }

    async fn send<T: DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> Result<T> {
        let response = self.answer(request).await?;
        response
            .json()
            .await
            .map_err(|e| ClientError::Unexpected(chain(&e)))
    }

    async fn send_empty(&self, request: reqwest::RequestBuilder) -> Result<()> {
        self.answer(request).await.map(drop)
    }

    /// The response, if it is a success; the server's error otherwise.
    async fn answer(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request
            .send()
            .await
            .map_err(|e| ClientError::Unreachable(chain(&e)))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        match response.json::<RemoteError>().await {
            Ok(error) => Err(ClientError::Remote(error)),
            Err(_) => Err(ClientError::Unexpected(format!("HTTP {status}"))),
        }
    }
}

/// A sign-in under way: open [`SignIn::url`] in the browser, then [`SignIn::finish`].
pub struct SignIn {
    url: Url,
    listener: TcpListener,
    verifier: Secret,
}

impl SignIn {
    /// Where to send the browser. It holds no secret: the challenge is a hash.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The loopback address the browser comes back to.
    pub fn redirect_port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Wait for the browser to come back, then redeem its code as `device`.
    pub async fn finish(
        self,
        client: &Client,
        device: NewDevice,
        timeout: Duration,
    ) -> Result<Credentials> {
        let code = tokio::time::timeout(timeout, returned(&self.listener, RETURN_READ_TIMEOUT))
            .await
            .map_err(|_| ClientError::TimedOut)??;
        let request = TokenRequest::SignIn {
            code: Secret::new(code),
            verifier: self.verifier,
            device,
        };
        client
            .send(client.http.post(client.url("v1/token")?).json(&request))
            .await
    }
}

/// A signed-in device's credentials, refreshed when its access is about to lapse.
pub struct Session {
    pub client: Client,
    pub device: DeviceId,
    access: Secret,
    /// When `access` should be refreshed, on this process's clock.
    refresh_at: tokio::time::Instant,
    refresh: Secret,
}

impl Session {
    /// From credentials the server just issued.
    pub fn new(client: Client, credentials: Credentials) -> Self {
        let refresh_at = refresh_at(credentials.access_expires_in_ms);
        Self {
            client,
            device: credentials.device,
            access: credentials.access,
            refresh_at,
            refresh: credentials.refresh,
        }
    }

    /// From a refresh credential kept from before, refreshed at once. The one given is spent
    /// by then: keep [`Session::refresh_credential`] in its place before anything else.
    pub async fn resume(client: Client, refresh: &Secret) -> Result<Self> {
        let credentials = client.refresh(refresh).await?;
        Ok(Self::new(client, credentials))
    }

    /// An access credential that works for at least another minute, refreshing first when
    /// needed. `keep` is given the new refresh credential: the old one is spent, and presenting
    /// it again would sign the device out. The session takes the new one first, so when `keep`
    /// fails ([`ClientError::NotKept`]) this process can carry on with it.
    pub async fn access(
        &mut self,
        keep: impl FnOnce(&Secret) -> std::result::Result<(), String>,
    ) -> Result<Secret> {
        if tokio::time::Instant::now() >= self.refresh_at {
            let credentials = self.client.refresh(&self.refresh).await?;
            self.refresh_at = refresh_at(credentials.access_expires_in_ms);
            self.access = credentials.access;
            self.refresh = credentials.refresh;
            keep(&self.refresh).map_err(ClientError::NotKept)?;
        }
        Ok(self.access.clone())
    }

    /// The refresh credential in force, to keep.
    pub fn refresh_credential(&self) -> &Secret {
        &self.refresh
    }
}

fn refresh_at(expires_in_ms: u64) -> tokio::time::Instant {
    tokio::time::Instant::now()
        + Duration::from_millis(expires_in_ms.saturating_sub(REFRESH_MARGIN_MS))
}

/// HTTPS, or plain HTTP to a loopback address written as one.
fn secure_enough(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        },
        _ => false,
    }
}

/// The one-time code the browser brings back to the loopback listener.
async fn returned(listener: &TcpListener, read_timeout: Duration) -> Result<String> {
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| ClientError::Invalid(e.to_string()))?;
        let mut buffer = vec![0u8; 8192];
        let Ok(Ok(read)) = tokio::time::timeout(read_timeout, socket.read(&mut buffer)).await
        else {
            continue;
        };
        let request = String::from_utf8_lossy(&buffer[..read]);
        let Some(target) = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
        else {
            continue;
        };
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };
        if url.path() != "/signed-in" {
            let _ = socket
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await;
            continue;
        }
        let query = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let (body, outcome) = match (query("code"), query("error")) {
            (Some(code), _) => ("Signed in. You can close this window.", Ok(code)),
            (None, Some(error)) => (
                "Not signed in. You can close this window.",
                Err(ClientError::Remote(RemoteError::new(
                    ErrorCode::Unauthenticated,
                    format!("The sign-in was not completed ({}).", clip(&error)),
                ))),
            ),
            (None, None) => continue,
        };
        let _ = socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        return outcome;
    }
}

/// An error code from the browser, bounded: it came through a URL anyone could have opened.
fn clip(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(32)
        .collect()
}

/// 256 random bits, base64url: a PKCE verifier.
fn random() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system's random source");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// An error and its causes: reqwest's own message is often only "error sending request".
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(&format!(": {next}"));
        cause = next.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_a_loopback_address_is_spoken_to() {
        for ok in [
            "https://remote.example.com",
            "https://example.com/yardsort/",
            "http://127.0.0.1:8787",
            "http://[::1]:8787",
        ] {
            assert!(Client::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://remote.example.com",
            "http://localhost:8787",
            "ftp://example.com",
            "https://example.com/?next=x",
            "https://user@example.com",
            "not an address",
        ] {
            assert!(Client::new(bad).is_err(), "{bad}");
        }
        let client = Client::new("https://example.com/yardsort").unwrap();
        assert_eq!(
            client.url("v1/server").unwrap().as_str(),
            "https://example.com/yardsort/v1/server"
        );
    }

    #[test]
    fn the_challenge_is_the_rfc_7636_one() {
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(random().len(), 43);
    }

    #[tokio::test]
    async fn the_browser_brings_the_code_back_once() {
        let client = Client::new("http://127.0.0.1:9").unwrap();
        let sign_in = client.begin_sign_in().await.unwrap();
        let port = sign_in.redirect_port();
        let challenge = sign_in
            .url()
            .query_pairs()
            .find(|(k, _)| k == "challenge")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(challenge, s256(sign_in.verifier.expose()));
        let browser = tokio::spawn(async move {
            let get = |path: &str| {
                let path = path.to_owned();
                async move {
                    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                        .await
                        .unwrap();
                    socket
                        .write_all(format!("GET {path} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes())
                        .await
                        .unwrap();
                    let mut answer = String::new();
                    socket.read_to_string(&mut answer).await.unwrap();
                    answer
                }
            };
            // Something else on the port first, then the answer.
            assert!(get("/favicon.ico").await.starts_with("HTTP/1.1 404"));
            assert!(get("/signed-in?code=the-code").await.contains("Signed in."));
        });
        let code = returned(&sign_in.listener, RETURN_READ_TIMEOUT)
            .await
            .unwrap();
        browser.await.unwrap();
        assert_eq!(code, "the-code");
    }

    #[tokio::test]
    async fn a_refused_sign_in_says_so_without_echoing_the_page() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            socket
                .write_all(b"GET /signed-in?error=%3Cscript%3Eunauthenticated HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
        });
        let error = returned(&listener, RETURN_READ_TIMEOUT).await.unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Unauthenticated));
        assert!(!error.to_string().contains('<'), "{error}");
    }

    #[tokio::test]
    async fn a_connection_that_sends_nothing_does_not_hold_up_the_browser() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            // Opened first and kept open, saying nothing, as a browser's preconnect does.
            let _quiet = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            socket
                .write_all(b"GET /signed-in?code=the-code HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let code = tokio::time::timeout(
            Duration::from_secs(10),
            returned(&listener, Duration::from_millis(200)),
        )
        .await
        .expect("the quiet connection held it up")
        .unwrap();
        assert_eq!(code, "the-code");
    }

    /// A stand-in for `POST /v1/token` that answers each refresh with the next credentials,
    /// lapsing at once, and reports each refresh credential presented to it.
    async fn token_server() -> (Client, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, presented) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            for issued in 1..=u32::MAX {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                let body = loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request).into_owned();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .and_then(|l| l.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if body.len() >= length {
                        break body.to_owned();
                    }
                };
                let request: serde_json::Value = serde_json::from_str(&body).unwrap();
                seen.send(request["refresh"].as_str().unwrap().to_owned())
                    .unwrap();
                let answer = serde_json::json!({
                    "device": "4f1c2a3b-5d6e-4f70-8192-a3b4c5d6e7f8",
                    "access": format!("ys1.at.{issued}"),
                    "accessExpiresInMs": 0,
                    "refresh": format!("ys1.rt.{issued}"),
                })
                .to_string();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                            answer.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        (
            Client::new(&format!("http://127.0.0.1:{port}")).unwrap(),
            presented,
        )
    }

    #[tokio::test]
    async fn a_refresh_credential_that_could_not_be_kept_is_still_the_one_used_next() {
        let (client, mut presented) = token_server().await;
        let mut session = Session::resume(client, &Secret::new("ys1.rt.0"))
            .await
            .unwrap();
        assert_eq!(presented.recv().await.unwrap(), "ys1.rt.0");
        assert_eq!(session.refresh_credential().expose(), "ys1.rt.1");

        let error = session
            .access(|_| Err("the credential store is locked".into()))
            .await
            .unwrap_err();
        assert!(matches!(error, ClientError::NotKept(_)), "{error}");
        assert_eq!(presented.recv().await.unwrap(), "ys1.rt.1");

        let mut kept = None;
        let access = session
            .access(|refresh| {
                kept = Some(refresh.expose().to_owned());
                Ok(())
            })
            .await
            .unwrap();
        // The one issued with the failed keep, not the spent one before it.
        assert_eq!(presented.recv().await.unwrap(), "ys1.rt.2");
        assert_eq!(access.expose(), "ys1.at.3");
        assert_eq!(kept.as_deref(), Some("ys1.rt.3"));
    }
}
