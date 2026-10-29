//! The server on a real port, with the stand-in identity provider, for tests that go through
//! HTTP the way a device does.

use std::collections::HashMap;

use remote_proto::account::{Credentials, DeviceKind, NewDevice, Secret, TokenRequest};
use remote_proto::{Platform, RemoteError, ServerId};
use reqwest::header::LOCATION;
use url::Url;
use yardsort_server::oidc::{self, ProviderConfig};
use yardsort_server::records::Records;
use yardsort_server::relay::Relay;
use yardsort_server::{http, tokens, ControlService, ServerConfig};

use super::idp::{Idp, APP, CLIENT_ID};

pub const SECRET: &str = "the client secret";

pub struct Served {
    pub idp: Idp,
    pub base: Url,
    pub client: reqwest::Client,
    /// The relay's sockets, to fail them all as a restart would.
    #[allow(dead_code)]
    pub relay: yardsort_server::relay::Relay,
    serving: tokio::task::JoinHandle<()>,
}

impl Served {
    pub async fn start(records: impl Records + 'static) -> Self {
        Self::start_with(records, http::Options::default()).await
    }

    pub async fn start_with(records: impl Records + 'static, options: http::Options) -> Self {
        Self::start_as(records, options, Relay::default()).await
    }

    /// As one of several instances of a server: `relay` names this one, and how requests reach
    /// the others.
    pub async fn start_as(
        records: impl Records + 'static,
        options: http::Options,
        relay: Relay,
    ) -> Self {
        let idp = Idp::start(Some(SECRET)).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let client = oidc::http_client_builder().no_proxy().build().unwrap();
        let service = ControlService::new(
            ServerConfig {
                server: ServerId::new("srv-test").unwrap(),
                name: "Test server".into(),
                public_url: base.clone(),
                provider: ProviderConfig {
                    issuer: idp.issuer(),
                    client_id: CLIENT_ID.into(),
                    client_secret: Some(Secret::new(SECRET)),
                },
                device_redirects: vec![APP.into()],
            },
            records,
            client.clone(),
        )
        .unwrap()
        .with_relay(relay);
        let relay = service.relay().clone();
        let app = http::router(service, options)
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            idp,
            base,
            client,
            relay,
            serving,
        }
    }

    /// Stop the server as a crash would: no new connections, every relay socket closed.
    #[allow(dead_code)]
    pub fn fail(&self) {
        self.serving.abort();
        self.relay.shutdown();
    }

    pub fn url(&self, path: &str) -> Url {
        self.base.join(path).unwrap()
    }

    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.client.get(self.url(path)).send().await.unwrap()
    }

    pub async fn token(&self, request: &TokenRequest) -> reqwest::Response {
        self.client
            .post(self.url("v1/token"))
            .json(request)
            .send()
            .await
            .unwrap()
    }

    pub async fn devices(&self, access: Option<&Secret>) -> reqwest::Response {
        let mut request = self.client.get(self.url("v1/devices"));
        if let Some(access) = access {
            request = request.bearer_auth(access.expose());
        }
        request.send().await.unwrap()
    }
}

pub fn location(response: &reqwest::Response) -> Url {
    Url::parse(response.headers()[LOCATION].to_str().unwrap()).unwrap()
}

pub fn query(url: &Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}

pub async fn error(response: reqwest::Response) -> RemoteError {
    response.json().await.unwrap()
}

impl Served {
    /// Through the browser and back, as `subject`, then redeemed: a device of `kind` signed in.
    pub async fn sign_in(&self, subject: &str, name: &str, kind: DeviceKind) -> Credentials {
        let verifier = tokens::random();
        let mut start = self.url("v1/login");
        start
            .query_pairs_mut()
            .append_pair("challenge", &tokens::s256(&verifier))
            .append_pair("challengeMethod", "S256")
            .append_pair("redirect", APP);
        let to_provider = self.client.get(start).send().await.unwrap();
        let (code, state) = self.idp.approve(&location(&to_provider), subject);
        let mut callback = self.url("v1/login/callback");
        callback
            .query_pairs_mut()
            .append_pair("code", &code)
            .append_pair("state", &state);
        let back = location(&self.client.get(callback).send().await.unwrap());
        let signed_in = self
            .token(&TokenRequest::SignIn {
                code: Secret::new(query(&back)["code"].clone()),
                verifier: Secret::new(verifier),
                device: NewDevice {
                    name: name.into(),
                    kind,
                    platform: Platform::Linux,
                },
            })
            .await;
        assert!(signed_in.status().is_success(), "{}", signed_in.status());
        signed_in.json().await.unwrap()
    }

    /// `GET` as the device holding `access`.
    pub async fn get_as(&self, access: &Secret, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .bearer_auth(access.expose())
            .send()
            .await
            .unwrap()
    }

    /// `POST` as the device holding `access`, with `body` as JSON when there is one.
    pub async fn post_as(
        &self,
        access: &Secret,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> reqwest::Response {
        let mut request = self
            .client
            .post(self.url(path))
            .bearer_auth(access.expose());
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().await.unwrap()
    }
}
