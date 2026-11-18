//! A stand-in entitlements service served over real HTTP: it answers each action as told, or
//! fails as an unreachable one would, and keeps every question it was asked with the credential
//! that came with it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use url::Url;
use yardsort_server::entitlements::Entitlements;

pub const TOKEN: &str = "the entitlements token";

/// How the stand-in answers one action.
#[derive(Debug, Clone)]
pub enum Plan {
    Allow,
    Refuse(Option<&'static str>),
    /// A `503`, as a service that is down answers.
    Down,
}

#[derive(Default)]
struct Book {
    plans: HashMap<String, Plan>,
    asked: Vec<(Option<String>, Value)>,
}

pub struct Plans {
    pub url: Url,
    book: Arc<Mutex<Book>>,
    serving: tokio::task::JoinHandle<()>,
}

impl Plans {
    /// Allowing everything until told otherwise.
    pub async fn start() -> Self {
        let book = Arc::new(Mutex::new(Book::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/ask", listener.local_addr().unwrap())).unwrap();
        let app = Router::new()
            .route("/ask", post(ask))
            .with_state(Arc::clone(&book));
        let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, book, serving }
    }

    /// The server's side of it: this service, with [`TOKEN`].
    pub fn entitlements(&self) -> Entitlements {
        Entitlements::service(
            self.url.clone(),
            Some(remote_proto::account::Secret::new(TOKEN)),
            yardsort_server::oidc::http_client_builder()
                .no_proxy()
                .build()
                .unwrap(),
        )
    }

    /// Answer `action` (`device`, `stream` or `sync`) this way from now on.
    pub fn set(&self, action: &str, plan: Plan) {
        self.book.lock().unwrap().plans.insert(action.into(), plan);
    }

    /// Answer every action this way from now on.
    pub fn set_all(&self, plan: Plan) {
        for action in ["device", "stream", "sync"] {
            self.set(action, plan.clone());
        }
    }

    /// Every question so far, with the `Authorization` header it came with.
    pub fn asked(&self) -> Vec<(Option<String>, Value)> {
        self.book.lock().unwrap().asked.clone()
    }

    /// The questions about `action` so far.
    pub fn asked_about(&self, action: &str) -> usize {
        self.asked()
            .iter()
            .filter(|(_, q)| q["action"] == action)
            .count()
    }
}

impl Drop for Plans {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

async fn ask(
    State(book): State<Arc<Mutex<Book>>>,
    headers: HeaderMap,
    Json(question): Json<Value>,
) -> Response {
    let authorization = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut book = book.lock().unwrap();
    let action = question["action"].as_str().unwrap_or_default().to_owned();
    book.asked.push((authorization, question));
    match book.plans.get(&action).cloned().unwrap_or(Plan::Allow) {
        Plan::Allow => Json(json!({"allowed": true})).into_response(),
        Plan::Refuse(Some(message)) => {
            Json(json!({"allowed": false, "message": message})).into_response()
        }
        Plan::Refuse(None) => Json(json!({"allowed": false})).into_response(),
        Plan::Down => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
