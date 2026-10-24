//! The control service over HTTP. See [`remote_proto::account`] for the routes and messages.
//!
//! Two routes are for a browser, not an app: `/v1/login` and its callback answer with redirects,
//! and with a short plain-text page when there is nowhere to send the browser. Everything else is
//! JSON, and a failure is a [`RemoteError`] with the HTTP status that matches its code.
//!
//! Starting a sign-in and `/v1/token` are limited per client address ([`crate::limit`]). Behind a
//! proxy, the address is the one the proxy names in [`Options::client_ip_header`]; otherwise the
//! connection's own, which needs the router served with `into_make_service_with_connect_info`.
//!
//! Nothing secret is logged: a request is logged by method, path and status, never its query
//! (where sign-in codes travel) or its headers (where access credentials do).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use remote_proto::account::{LoginParams, TokenRequest};
use remote_proto::{DeviceId, ErrorCode, RemoteError};
use serde::Deserialize;

use crate::limit::Limiter;
use crate::records::Records;
use crate::{Caller, ControlService, LoginFinish};

/// The largest request body accepted. The biggest real one, a sign-in redemption, is well under a
/// kilobyte.
pub const MAX_BODY: usize = 16 * 1024;

/// Sign-ins one address may start a minute, after a burst of as many.
pub const LOGINS_PER_MINUTE: u32 = 20;
/// Token requests one address may make a minute. A device refreshes every quarter hour, so this
/// leaves room for a few hundred devices behind one address.
pub const TOKENS_PER_MINUTE: u32 = 60;

pub struct Options {
    /// The header a trusted proxy puts the client's address in (`Fly-Client-IP` on Fly.io). Set
    /// it only behind such a proxy: anyone can send the header to a server that is not.
    pub client_ip_header: Option<HeaderName>,
    pub logins_per_minute: u32,
    pub tokens_per_minute: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            client_ip_header: None,
            logins_per_minute: LOGINS_PER_MINUTE,
            tokens_per_minute: TOKENS_PER_MINUTE,
        }
    }
}

type Service<R> = Arc<ControlService<R>>;

pub fn router<R: Records + 'static>(service: ControlService<R>, options: Options) -> Router {
    let gate = |per_minute, answer| {
        Arc::new(Gate {
            limiter: Limiter::per_minute(per_minute),
            client_ip_header: options.client_ip_header.clone(),
            answer,
        })
    };
    let logins = gate(options.logins_per_minute, Answer::Page);
    let tokens = gate(options.tokens_per_minute, Answer::Json);
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/server", get(describe::<R>))
        .route(
            "/v1/login",
            get(login::<R>).route_layer(middleware::from_fn_with_state(logins, limited)),
        )
        .route("/v1/login/callback", get(callback::<R>))
        .route(
            "/v1/token",
            post(token::<R>).route_layer(middleware::from_fn_with_state(tokens, limited)),
        )
        .route("/v1/devices", get(devices::<R>))
        .route("/v1/devices/{id}/revoke", post(revoke::<R>))
        .fallback(|| async { failure(RemoteError::not_found()) })
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn(guard))
        .with_state(Arc::new(service))
}

struct Gate {
    limiter: Limiter,
    client_ip_header: Option<HeaderName>,
    answer: Answer,
}

/// How a refusal is shown: as a page to a browser, or as JSON to an app.
enum Answer {
    Page,
    Json,
}

async fn limited(State(gate): State<Arc<Gate>>, request: Request, next: Next) -> Response {
    let address = client_address(&request, gate.client_ip_header.as_ref());
    let Err(wait) = gate.limiter.check(address, Instant::now()) else {
        return next.run(request).await;
    };
    let message = "Too many attempts from this address. Try again in a minute.";
    let mut response = match gate.answer {
        Answer::Page => page(StatusCode::TOO_MANY_REQUESTS, message),
        Answer::Json => failure(RemoteError::new(ErrorCode::RateLimited, message)),
    };
    let seconds = (wait.as_secs_f64().ceil() as u64).max(1).to_string();
    if let Ok(value) = HeaderValue::from_str(&seconds) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

/// Who is asking: the trusted proxy's word, else the connection's address.
fn client_address(request: &Request, proxy_header: Option<&HeaderName>) -> IpAddr {
    proxy_header
        .and_then(|name| request.headers().get(name))
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.trim().parse().ok())
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip())
        })
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Milliseconds since the Unix epoch, the protocol's clock.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

async fn describe<R: Records>(State(service): State<Service<R>>) -> Response {
    Json(service.describe()).into_response()
}

async fn login<R: Records>(
    State(service): State<Service<R>>,
    params: Result<Query<LoginParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return page(
            StatusCode::BAD_REQUEST,
            "This sign-in link is incomplete. Start again from the app.",
        );
    };
    match service.begin_login(&params, now_ms()).await {
        Ok(to) => Redirect::to(to.as_str()).into_response(),
        Err(error) => page(status(error.code), &error.message),
    }
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback<R: Records>(
    State(service): State<Service<R>>,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    let Ok(Query(answer)) = query else {
        return page(StatusCode::BAD_REQUEST, "This sign-in answer is malformed.");
    };
    let finish = service
        .finish_login(
            answer.code.as_deref(),
            answer.state.as_deref(),
            answer.error.as_deref(),
            now_ms(),
        )
        .await;
    match finish {
        LoginFinish::SignedIn(to) => Redirect::to(to.as_str()).into_response(),
        LoginFinish::Refused { to, why } => {
            tracing::info!(%why, "sign-in refused");
            Redirect::to(to.as_str()).into_response()
        }
        LoginFinish::Unknown(error) => page(status(error.code), &error.message),
    }
}

async fn token<R: Records>(
    State(service): State<Service<R>>,
    request: Result<Json<TokenRequest>, JsonRejection>,
) -> Response {
    let request = match request {
        Ok(Json(request)) => request,
        Err(rejection) => {
            let mut response = failure(RemoteError::new(
                ErrorCode::InvalidRequest,
                "The request is not a token request.",
            ));
            // Too large stays too large, rather than looking like any other malformed body.
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                *response.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
            }
            return response;
        }
    };
    match service.token(&request, now_ms()).await {
        Ok(credentials) => Json(credentials).into_response(),
        Err(error) => failure(error),
    }
}

async fn devices<R: Records>(State(service): State<Service<R>>, headers: HeaderMap) -> Response {
    let caller = match caller(&service, &headers).await {
        Ok(caller) => caller,
        Err(error) => return failure(error),
    };
    match service.devices(&caller).await {
        Ok(page) => Json(page).into_response(),
        Err(error) => failure(error),
    }
}

async fn revoke<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let caller = match caller(&service, &headers).await {
        Ok(caller) => caller,
        Err(error) => return failure(error),
    };
    let Ok(id) = DeviceId::new(id) else {
        return failure(RemoteError::not_found());
    };
    match service.revoke(&caller, &id, now_ms()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}

/// The device behind `Authorization: Bearer …`.
async fn caller<R: Records>(
    service: &ControlService<R>,
    headers: &HeaderMap,
) -> Result<Caller, RemoteError> {
    let access = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| RemoteError::new(ErrorCode::Unauthenticated, "Sign in first."))?;
    service.authenticate(access.trim(), now_ms()).await
}

fn status(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::Forbidden => StatusCode::FORBIDDEN,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::InvalidRequest | ErrorCode::ProtocolIncompatible => StatusCode::BAD_REQUEST,
        ErrorCode::Unsupported => StatusCode::NOT_IMPLEMENTED,
        ErrorCode::HostOffline => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::ControlConflict | ErrorCode::LeaseLost | ErrorCode::SessionEnded => {
            StatusCode::CONFLICT
        }
        ErrorCode::ResnapshotRequired | ErrorCode::CursorExpired => StatusCode::GONE,
        ErrorCode::Internal | ErrorCode::Unknown => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn failure(error: RemoteError) -> Response {
    let mut response = (status(error.code), Json(&error)).into_response();
    if error.code == ErrorCode::Unauthenticated {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

/// A page for a browser that has nowhere else to go.
fn page(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("{message}\n\nYou can close this window.\n"),
    )
        .into_response()
}

/// On every response: nothing is cached (credentials and codes pass through here), and a page the
/// browser lands on sends no `Referer` onward. Then one log line, without the query.
async fn guard(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    tracing::info!(
        %method,
        path = %logged_path(&path),
        status = response.status().as_u16(),
        ms = started.elapsed().as_millis() as u64,
    );
    response
}

/// A path as logged: bounded, since anyone can send any path.
fn logged_path(path: &str) -> &str {
    match path.char_indices().nth(128) {
        Some((at, _)) => &path[..at],
        None => path,
    }
}
