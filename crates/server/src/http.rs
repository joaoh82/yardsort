//! The control service over HTTP. See [`remote_proto::account`] for the routes and messages.
//!
//! Two routes are for a browser, not an app: `/v1/login` and its callback answer with redirects,
//! and with a short plain-text page when there is nowhere to send the browser. Everything else is
//! JSON, and a failure is a [`RemoteError`] with the HTTP status that matches its code.
//!
//! Starting a sign-in, `/v1/token`, making a pairing offer, redeeming one and asking for a relay
//! ticket are limited per client address, across every instance ([`crate::limit`]). Behind a
//! proxy, the address is the one the proxy names in [`Options::client_ip_header`]; otherwise the
//! connection's own, which needs the router served with `into_make_service_with_connect_info`.
//!
//! Nothing secret is logged: a request is logged by method, path and status, never its query
//! (where sign-in codes travel) or its headers (where access credentials do).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use remote_proto::account::{LoginParams, TokenRequest};
use remote_proto::pairing::{GrantDecision, HostRegistration, PairingRequest};
use remote_proto::relay::{TicketRequest, MAX_MESSAGE};
use remote_proto::{DeviceId, ErrorCode, GrantId, HostId, RemoteError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::limit::Limiter;
use crate::records::{Allowance, Records};
use crate::relay::{Instance, Ticketed};
use crate::{Caller, ControlService, LoginFinish};

/// The answer's header that has Fly.io's proxy send the request to another instance.
const FLY_REPLAY: &str = "fly-replay";

/// The largest request body accepted. The biggest real one, a sign-in redemption, is well under a
/// kilobyte.
pub const MAX_BODY: usize = 16 * 1024;

/// Sign-ins one address may start a minute, after a burst of as many.
pub const LOGINS_PER_MINUTE: u32 = 20;
/// Token requests one address may make a minute. A device refreshes every quarter hour, so this
/// leaves room for a few hundred devices behind one address.
pub const TOKENS_PER_MINUTE: u32 = 60;
/// Pairing offers one address may ask for a minute. A host makes one each time its person shows
/// a code.
pub const OFFERS_PER_MINUTE: u32 = 10;
/// Pairings one address may try a minute, right or wrong. An account's wrong codes are also
/// limited on their own (`MAX_OFFER_MISSES`), wherever they come from.
pub const PAIRINGS_PER_MINUTE: u32 = 10;
/// Relay tickets one address may ask for a minute. A host asks for one each time it connects,
/// and a device each time it opens a host.
pub const TICKETS_PER_MINUTE: u32 = 60;

pub struct Options {
    /// The header a trusted proxy puts the client's address in (`Fly-Client-IP` on Fly.io). Set
    /// it only behind such a proxy: anyone can send the header to a server that is not.
    pub client_ip_header: Option<HeaderName>,
    pub logins_per_minute: u32,
    pub tokens_per_minute: u32,
    pub offers_per_minute: u32,
    pub pairings_per_minute: u32,
    pub tickets_per_minute: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            client_ip_header: None,
            logins_per_minute: LOGINS_PER_MINUTE,
            tokens_per_minute: TOKENS_PER_MINUTE,
            offers_per_minute: OFFERS_PER_MINUTE,
            pairings_per_minute: PAIRINGS_PER_MINUTE,
            tickets_per_minute: TICKETS_PER_MINUTE,
        }
    }
}

type Service<R> = Arc<ControlService<R>>;

pub fn router<R: Records + 'static>(service: ControlService<R>, options: Options) -> Router {
    let service = Arc::new(service);
    let gate = |name, per_minute, answer| {
        Arc::new(Gate {
            name,
            per_minute,
            limiter: Limiter::per_minute(per_minute),
            service: Arc::clone(&service),
            client_ip_header: options.client_ip_header.clone(),
            answer,
        })
    };
    let logins = gate("logins", options.logins_per_minute, Answer::Page);
    let tokens = gate("tokens", options.tokens_per_minute, Answer::Json);
    let offers = gate("offers", options.offers_per_minute, Answer::Json);
    let pairings = gate("pairings", options.pairings_per_minute, Answer::Json);
    let tickets = gate("tickets", options.tickets_per_minute, Answer::Json);
    crate::relay::start_rechecks(Arc::clone(&service));
    crate::limit::start_sweeping(Arc::downgrade(&service));
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/server", get(describe::<R>))
        .route(
            "/v1/login",
            get(login::<R>).route_layer(middleware::from_fn_with_state(logins, limited::<R>)),
        )
        .route("/v1/login/callback", get(callback::<R>))
        .route(
            "/v1/token",
            post(token::<R>).route_layer(middleware::from_fn_with_state(tokens, limited::<R>)),
        )
        .route("/v1/devices", get(devices::<R>))
        .route("/v1/devices/{id}/revoke", post(revoke::<R>))
        .route("/v1/hosts", get(hosts::<R>).post(register_host::<R>))
        .route(
            "/v1/hosts/{id}/offers",
            post(offer::<R>).route_layer(middleware::from_fn_with_state(offers, limited::<R>)),
        )
        .route("/v1/hosts/{id}/grants", get(host_grants::<R>))
        .route("/v1/hosts/{id}/remove", post(remove_host::<R>))
        .route(
            "/v1/pairings",
            post(pair::<R>).route_layer(middleware::from_fn_with_state(pairings, limited::<R>)),
        )
        .route("/v1/grants", get(device_grants::<R>))
        .route("/v1/grants/{id}", get(grant::<R>))
        .route("/v1/grants/{id}/approve", post(approve::<R>))
        .route("/v1/grants/{id}/deny", post(deny::<R>))
        .route("/v1/grants/{id}/revoke", post(revoke_grant::<R>))
        .route(
            "/v1/relay/tickets",
            post(ticket::<R>).route_layer(middleware::from_fn_with_state(tickets, limited::<R>)),
        )
        .route("/v1/relay", get(relay::<R>))
        .fallback(|| async { failure(RemoteError::not_found()) })
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn(guard))
        .with_state(service)
}

/// One rate limit: counted here, then in the records every instance shares ([`crate::limit`]).
struct Gate<R> {
    /// Its name in the records.
    name: &'static str,
    per_minute: u32,
    limiter: Limiter,
    service: Service<R>,
    client_ip_header: Option<HeaderName>,
    answer: Answer,
}

/// How a refusal is shown: as a page to a browser, or as JSON to an app.
enum Answer {
    Page,
    Json,
}

async fn limited<R: Records>(
    State(gate): State<Arc<Gate<R>>>,
    request: Request,
    next: Next,
) -> Response {
    let address = client_address(&request, gate.client_ip_header.as_ref());
    let records = &gate.service.records;
    let shared = match gate.limiter.check(address, Instant::now()) {
        Ok(()) => crate::limit::take_shared(records, gate.name, address, gate.per_minute).await,
        Err(wait) => Some(Allowance::Wait(wait)),
    };
    let wait = match shared {
        Some(Allowance::Wait(wait)) => wait,
        taken => {
            let response = next.run(request).await;
            // Sent on to the instance holding the link, which counts it: given back before the
            // proxy sends it there.
            if taken == Some(Allowance::Taken) && response.headers().contains_key(FLY_REPLAY) {
                crate::limit::return_shared(records, gate.name, address, gate.per_minute).await;
            }
            return response;
        }
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

async fn hosts<R: Records>(State(service): State<Service<R>>, headers: HeaderMap) -> Response {
    match caller(&service, &headers).await {
        Ok(caller) => answer(service.hosts(&caller).await),
        Err(error) => failure(error),
    }
}

async fn register_host<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    body: Result<Json<HostRegistration>, JsonRejection>,
) -> Response {
    let caller = match caller(&service, &headers).await {
        Ok(caller) => caller,
        Err(error) => return failure(error),
    };
    match body_of(body, "The request is not a host registration.") {
        Ok(registration) => answer(
            service
                .register_host(&caller, &registration, now_ms())
                .await,
        ),
        Err(refused) => refused.into_response(),
    }
}

async fn offer<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (caller, host) = match host_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    answer(service.offer(&caller, &host, now_ms()).await)
}

async fn host_grants<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (caller, host) = match host_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    answer(service.host_grants(&caller, &host, now_ms()).await)
}

async fn remove_host<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (caller, host) = match host_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    match service.remove_host(&caller, &host, now_ms()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}

async fn pair<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    body: Result<Json<PairingRequest>, JsonRejection>,
) -> Response {
    let caller = match caller(&service, &headers).await {
        Ok(caller) => caller,
        Err(error) => return failure(error),
    };
    match body_of(body, "The request is not a pairing request.") {
        Ok(request) => answer(service.pair(&caller, &request, now_ms()).await),
        Err(refused) => refused.into_response(),
    }
}

async fn ticket<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    body: Result<Json<TicketRequest>, JsonRejection>,
) -> Response {
    let caller = match caller(&service, &headers).await {
        Ok(caller) => caller,
        Err(error) => return failure(error),
    };
    let request = match body_of(body, "The request is not a ticket request.") {
        Ok(request) => request,
        Err(refused) => return refused.into_response(),
    };
    let replayed = service.relay.routing().replayed(&headers);
    match service.ticket(&caller, &request, replayed, now_ms()).await {
        Ok(Ticketed::Here(ticket)) => Json(ticket).into_response(),
        Ok(Ticketed::Elsewhere(instance)) => replay_to(&instance),
        Err(error) => failure(error),
    }
}

#[derive(Deserialize)]
struct RelayQuery {
    route: Option<String>,
}

/// Send this request on to `instance`, through the proxy in front of every instance
/// ([`crate::relay::Routing::FlyReplay`]). If the instance cannot be reached, the proxy gives the request to
/// another, which answers it as best it can; a client that sees this answer itself has no such
/// proxy in front of the server.
fn replay_to(instance: &Instance) -> Response {
    let mut response = failure(RemoteError::new(
        ErrorCode::HostOffline,
        "That host is connected to another instance of this server, and nothing sent this \
         request there.",
    ));
    *response.status_mut() = StatusCode::CONFLICT;
    if let Ok(value) = HeaderValue::from_str(&format!("prefer_instance={instance}")) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(FLY_REPLAY), value);
    }
    response
}

/// The relay's WebSocket. Anyone may open one; what it may do is the ticket it presents first.
/// One that names another instance's route is sent there before it is upgraded.
async fn relay<R: Records + 'static>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    query: Result<Query<RelayQuery>, QueryRejection>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let route = query.ok().and_then(|Query(q)| q.route);
    let replayed = service.relay.routing().replayed(&headers);
    match service.relay.socket_route(route.as_deref(), replayed) {
        Ok(None) => {}
        Ok(Some(instance)) => return replay_to(&instance),
        Err(error) => return failure(error),
    }
    let Ok(upgrade) = upgrade else {
        return failure(RemoteError::new(
            ErrorCode::InvalidRequest,
            "The relay speaks WebSocket only.",
        ));
    };
    upgrade
        .max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .on_upgrade(move |socket| service.serve_socket(socket))
}

async fn device_grants<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
) -> Response {
    match caller(&service, &headers).await {
        Ok(caller) => answer(service.device_grants(&caller, now_ms()).await),
        Err(error) => failure(error),
    }
}

async fn grant<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (caller, id) = match grant_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    answer(service.grant(&caller, &id, now_ms()).await)
}

async fn approve<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<GrantDecision>, JsonRejection>,
) -> Response {
    let (caller, id) = match grant_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    match body_of(body, "The request is not an approval.") {
        Ok(decision) => answer(
            service
                .decide(&caller, &id, Some(&decision), now_ms())
                .await,
        ),
        Err(refused) => refused.into_response(),
    }
}

async fn deny<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (caller, id) = match grant_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    answer(service.decide(&caller, &id, None, now_ms()).await)
}

async fn revoke_grant<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (caller, id) = match grant_call(&service, &headers, id).await {
        Ok(call) => call,
        Err(error) => return failure(error),
    };
    match service.revoke_grant(&caller, &id, now_ms()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure(error),
    }
}

/// The caller, and the host its path names. A path that is not an id is `not_found`.
async fn host_call<R: Records>(
    service: &ControlService<R>,
    headers: &HeaderMap,
    id: String,
) -> Result<(Caller, HostId), RemoteError> {
    let caller = caller(service, headers).await?;
    let host = HostId::new(id).map_err(|_| RemoteError::not_found())?;
    Ok((caller, host))
}

/// The caller, and the grant its path names.
async fn grant_call<R: Records>(
    service: &ControlService<R>,
    headers: &HeaderMap,
    id: String,
) -> Result<(Caller, GrantId), RemoteError> {
    let caller = caller(service, headers).await?;
    let grant = GrantId::new(id).map_err(|_| RemoteError::not_found())?;
    Ok((caller, grant))
}

/// A JSON body, or the answer to one that is not what the route takes.
fn body_of<T: DeserializeOwned>(
    body: Result<Json<T>, JsonRejection>,
    not_it: &'static str,
) -> Result<T, NotTheBody> {
    body.map(|Json(body)| body).map_err(|rejection| NotTheBody {
        too_large: rejection.status() == StatusCode::PAYLOAD_TOO_LARGE,
        not_it,
    })
}

/// Why a body was refused: too large stays too large, rather than looking like any other
/// malformed body.
struct NotTheBody {
    too_large: bool,
    not_it: &'static str,
}

impl IntoResponse for NotTheBody {
    fn into_response(self) -> Response {
        let mut response = failure(RemoteError::new(ErrorCode::InvalidRequest, self.not_it));
        if self.too_large {
            *response.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
        }
        response
    }
}

fn answer<T: Serialize>(result: Result<T, RemoteError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
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
