//! Remote: this computer as a host on a Yardsort server, and the devices its person lets in.
//! See [`yardsort_core::remote::host`] for what is kept where, and
//! [docs/design/27-remote-r2.md](../../docs/design/27-remote-r2.md) for the design.
//!
//! **A preview, off unless `YARDSORT_REMOTE_PREVIEW=1`.** Without it the Remote settings are not
//! shown, every command here refuses, and nothing connects. With it, a host that is on keeps a
//! link to its server's relay ([`remote_host::Connector`]) from the moment the app starts, and
//! paired devices reach it through that; there is no phone app yet to do so.
//!
//! The host signs in through the system browser, like any device, and keeps its refresh
//! credential in the OS credential store (or `YARDSORT_CREDENTIALS_DIR`, a directory only its
//! owner can read, where there is none). The webview never sees a credential: it sees whether
//! Remote is on, the pairing code and QR code while one is shown, and the devices let in.
//!
//! Only these commands approve a device, and only the desktop calls them. Nothing in `ys`, and
//! nothing an agent can reach, approves one.
//!
//! **A device asking is shown at once.** The relay tells the host on its link
//! ([`HostNotice`]); the app then asks the server, as Settings → Remote does, and tells the window
//! of the request only if the server has it waiting ([`RemotePairingRequested`]). The window
//! offers the answer; it never answers by itself.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use remote_client::{Client, ClientError, Session, SIGN_IN_TIMEOUT};
use remote_host::{Access, AccessError, AccessFuture, Connector, Serving};
use remote_proto::account::{DeviceKind, NewDevice, Secret};
use remote_proto::pairing::{
    GrantDecision, GrantState, GrantSummary, HostRegistration, OFFER_TTL_MS,
};
use remote_proto::relay::HostNotice;
use remote_proto::{ErrorCode, GrantId, HostId, Platform, ProjectGrant, ProjectId, Scope};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tauri_specta::Event;
use yardsort_core::remote::host::{
    self, refresh_entry, CredentialStore, FileCredentials, LocalGrant,
};
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::HostIdentity;
use yardsort_core::store::RemoteHostRow;

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// The switch that shows Remote at all.
pub const PREVIEW_VAR: &str = "YARDSORT_REMOTE_PREVIEW";
/// A directory to keep the host's credential in, instead of the OS credential store.
pub const CREDENTIALS_DIR_VAR: &str = "YARDSORT_CREDENTIALS_DIR";
const KEYRING_SERVICE: &str = "dev.yardsort.app";

/// What viewing gives: everything a device can read, nothing it can type.
const VIEW: [Scope; 4] = [
    Scope::WorkspaceRead,
    Scope::ActivityRead,
    Scope::MemoryRead,
    Scope::TerminalRead,
];

/// The host's session with its server, a sign-in waiting for the browser, and the link to the
/// server's relay.
#[derive(Default)]
pub struct Remote {
    session: tokio::sync::Mutex<Option<Session>>,
    signing_in: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    connector: std::sync::Mutex<Option<Connector>>,
}

/// Keep a link to the relay, when Remote is on: at start, and once it is turned on. Needs a
/// Tokio runtime: call it from async code.
pub fn connect(app: &AppHandle) {
    if !available() {
        return;
    }
    let state = app.state::<AppState>();
    let Ok(Some(row)) = state.store.remote_host() else {
        return;
    };
    let Ok(host) = HostId::new(row.host_id) else {
        return;
    };
    let connector = Connector::start(
        host,
        Arc::new(AppAccess(app.clone())),
        Arc::new(AppServing {
            app: app.clone(),
            git: OnceLock::new(),
        }),
    );
    // An earlier one stops as it is dropped.
    let earlier = match state.remote.connector.lock() {
        Ok(mut slot) => slot.replace(connector),
        Err(_) => None,
    };
    drop(earlier);
}

/// Close the link, and with it every device's channel.
async fn disconnect(state: &AppState) {
    let connector = state
        .remote
        .connector
        .lock()
        .ok()
        .and_then(|mut c| c.take());
    if let Some(connector) = connector {
        connector.stop().await;
    }
}

/// The host's access credential, from its session with the server.
struct AppAccess(AppHandle);

impl Access for AppAccess {
    fn access(&self) -> AccessFuture<'_> {
        Box::pin(async move {
            let state = self.0.state::<AppState>();
            let row = state
                .store
                .remote_host()
                .map_err(|e| AccessError::Unavailable(e.to_string()))?
                .ok_or_else(|| AccessError::SignedOut("Remote is off.".into()))?;
            let mut session = state.remote.session.lock().await;
            let session = ensure(&mut session, &row).await.map_err(access_error)?;
            let secret = access(session).await.map_err(access_error)?;
            Ok((session.client.clone(), secret))
        })
    }
}

fn access_error(error: IpcError) -> AccessError {
    if error.code == "remote_signed_out" {
        // The link's own line says what to do.
        let why = error
            .message
            .strip_suffix(AGAIN)
            .map_or(error.message.as_str(), str::trim_end);
        AccessError::SignedOut(why.to_owned())
    } else {
        AccessError::Unavailable(error.message)
    }
}

/// What a device's channel answers from: this app's records and terminals.
struct AppServing {
    app: AppHandle,
    git: OnceLock<Option<yardsort_core::git::Git>>,
}

impl Serving for AppServing {
    fn with(&self, f: &mut dyn FnMut(&Context<'_>)) {
        let state = self.app.state::<AppState>();
        let Some(git) = self
            .git
            .get_or_init(|| yardsort_core::git::Git::new(&state.env()).ok())
        else {
            return;
        };
        let Ok(Some(row)) = state.store.remote_host() else {
            return;
        };
        let Ok(id) = HostId::new(row.host_id) else {
            return;
        };
        f(&Context {
            store: &state.store,
            git,
            terminals: state.host.as_ref(),
            host: &HostIdentity { id, name: row.name },
            interactive: state.host.shares_control(),
        });
    }

    fn told(&self, notice: HostNotice) {
        let HostNotice::PairingRequested { grant } = notice else {
            return;
        };
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = asked(&app, &grant).await {
                eprintln!("remote: a pairing request: {}", error.message);
            }
        });
    }
}

/// A device asks to be let in: shown in the window wherever it is, until answered or lapsed.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingRequested {
    pub request: RemoteRequest,
}

/// The relay said `grant` asks: tell the window, if the server has it waiting. The notice is only
/// a hint; what is shown is what the server answers this host.
async fn asked(app: &AppHandle, grant: &GrantId) -> IpcResult<()> {
    let state = app.state::<AppState>();
    let row = enabled(&state)?;
    let waiting = host_grants(&state, &row)
        .await?
        .into_iter()
        .find(|g| g.id == *grant && g.state == GrantState::Pending);
    if let Some(waiting) = waiting {
        let _ = RemotePairingRequested {
            request: request(waiting),
        }
        .emit(app);
    }
    Ok(())
}

fn request(grant: GrantSummary) -> RemoteRequest {
    RemoteRequest {
        wants_control: grant.requested.contains(&Scope::TerminalControl),
        grant_id: grant.id.to_string(),
        device_name: grant.device_name,
        expires_in_ms: grant
            .expires_in_ms
            .unwrap_or_default()
            .min(OFFER_TTL_MS as u64) as u32,
    }
}

pub fn available() -> bool {
    std::env::var_os(PREVIEW_VAR).is_some_and(|v| v == "1")
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    /// Whether this build shows Remote at all (the preview switch).
    pub available: bool,
    /// This computer as a host, when Remote is on.
    pub host: Option<RemoteHost>,
    /// The devices let in here.
    pub devices: Vec<RemoteDevice>,
    /// A sign-in is waiting for the browser.
    pub signing_in: bool,
    /// Where the host's credential is kept: the OS credential store, or a directory.
    pub credentials: String,
    /// The server and name Remote was last turned on with, to offer again once it is off.
    pub last_server_url: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHost {
    pub server_url: String,
    pub name: String,
    pub since: f64,
    /// The link to the server's relay.
    pub link: RemoteLink,
}

/// Where this computer's link to the relay stands.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RemoteLink {
    /// Not kept: the app has not started it.
    Off,
    Connecting,
    Connected {
        /// Devices connected through it now.
        devices: u32,
    },
    /// Not connected; trying again soon.
    Waiting {
        why: String,
        #[serde(rename = "retryInMs")]
        retry_in_ms: u32,
    },
    /// Given up: signed out at the server, or this host was removed there.
    Stopped {
        why: String,
    },
}

fn link(state: &AppState) -> RemoteLink {
    let connector = state.remote.connector.lock();
    let Some(status) = connector
        .ok()
        .and_then(|c| c.as_ref().map(Connector::status))
    else {
        return RemoteLink::Off;
    };
    match status {
        remote_host::Status::Connecting => RemoteLink::Connecting,
        remote_host::Status::Connected { devices } => RemoteLink::Connected {
            devices: u32::try_from(devices).unwrap_or(u32::MAX),
        },
        remote_host::Status::Waiting { why, retry_in } => RemoteLink::Waiting {
            why,
            retry_in_ms: u32::try_from(retry_in.as_millis()).unwrap_or(u32::MAX),
        },
        remote_host::Status::Stopped { why } => RemoteLink::Stopped { why },
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDevice {
    pub grant_id: String,
    pub name: String,
    /// It may type into terminals, not only watch them.
    pub control: bool,
    /// The projects it reaches; `None` for all of them.
    pub projects: Option<Vec<String>>,
    pub approved_at: f64,
}

/// What to show for a device to pair: the code to type, and a QR code holding the secret.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairing {
    pub code: String,
    /// An SVG image of the QR code: the server's address and the offer's secret.
    pub qr_svg: String,
    pub expires_in_ms: u32,
}

/// A device that redeemed this host's code and waits for an answer.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRequest {
    pub grant_id: String,
    pub device_name: String,
    /// It asked to type into terminals, not only watch.
    pub wants_control: bool,
    pub expires_in_ms: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRequests {
    pub requests: Vec<RemoteRequest>,
    /// The devices let in here, after the server's word on which are still in force.
    pub status: RemoteStatus,
}

/// The host's person's answer.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RemoteAnswer {
    Deny,
    Allow {
        /// Typing into terminals too, if the device asked for it.
        control: bool,
        /// Only these projects; all of them when absent.
        projects: Option<Vec<String>>,
    },
}

fn unavailable() -> IpcError {
    IpcError::new(
        "remote_unavailable",
        format!("Remote is a preview; start Yardsort with {PREVIEW_VAR}=1 to try it."),
    )
}

fn failed(error: ClientError) -> IpcError {
    let code = match error.code() {
        Some(ErrorCode::Unauthenticated) => "remote_signed_out",
        Some(ErrorCode::RateLimited) => "remote_rate_limited",
        Some(_) => "remote_refused",
        None => match error {
            ClientError::Invalid(_) => "remote_address",
            ClientError::TimedOut => "remote_timed_out",
            ClientError::NotKept(_) => "remote_credentials",
            _ => "remote_unreachable",
        },
    };
    IpcError::new(code, error.to_string())
}

/// What to do once signed out at the server.
const AGAIN: &str = "Turn Remote off, then on again.";

fn signed_out(why: &str) -> IpcError {
    IpcError::new("remote_signed_out", format!("{why} {AGAIN}"))
}

/// A request made with this computer's own credential. Refused, it was signed out at the server;
/// the server's own words ("Sign in again.") are for any client.
fn refused_here(error: ClientError) -> IpcError {
    if error.code() == Some(ErrorCode::Unauthenticated) {
        signed_out("This computer was signed out at the server.")
    } else {
        failed(error)
    }
}

fn store_failed(error: impl std::fmt::Display) -> IpcError {
    IpcError::new("remote_store", error.to_string())
}

/// Where the host's credential is kept.
fn credentials() -> IpcResult<Box<dyn CredentialStore>> {
    match std::env::var_os(CREDENTIALS_DIR_VAR) {
        Some(dir) => Ok(Box::new(
            FileCredentials::new(dir).map_err(|e| IpcError::new("remote_credentials", e))?,
        )),
        None => Ok(Box::new(Keyring)),
    }
}

fn credentials_label() -> String {
    match std::env::var_os(CREDENTIALS_DIR_VAR) {
        Some(dir) => format!("in {}", std::path::Path::new(&dir).display()),
        None => "in this computer's credential store".into(),
    }
}

/// The OS credential store: the Keychain, Credential Manager, or the Secret Service.
struct Keyring;

impl CredentialStore for Keyring {
    fn get(&self, name: &str) -> Result<Option<Secret>, String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, name).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(Secret::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, name: &str, secret: &Secret) -> Result<(), String> {
        keyring::Entry::new(KEYRING_SERVICE, name)
            .and_then(|entry| entry.set_password(secret.expose()))
            .map_err(|e| e.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, name).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

fn keep_refresh(device: &str, refresh: &Secret) -> Result<(), String> {
    credentials()
        .map_err(|e| e.message)?
        .set(&refresh_entry(device), refresh)
}

fn forget_refresh(device: &str) -> Result<(), String> {
    credentials()
        .map_err(|e| e.message)?
        .delete(&refresh_entry(device))
}

/// An access credential, keeping any new refresh credential under the session's device.
async fn access(session: &mut Session) -> IpcResult<Secret> {
    let device = session.device.clone();
    session
        .access(|refresh| keep_refresh(device.as_str(), refresh))
        .await
        .map_err(refused_here)
}

fn status(state: &AppState) -> IpcResult<RemoteStatus> {
    let last = state.settings.get().remote;
    let host = state.store.remote_host().map_err(store_failed)?;
    let devices = match &host {
        Some(_) => host::local_grants(&state.store)
            .map_err(store_failed)?
            .into_iter()
            .map(device)
            .collect(),
        None => Vec::new(),
    };
    Ok(RemoteStatus {
        available: available(),
        host: host.map(|h| RemoteHost {
            server_url: h.server_url,
            name: h.name,
            since: h.enabled_at as f64,
            link: link(state),
        }),
        devices,
        signing_in: state
            .remote
            .signing_in
            .lock()
            .map(|s| s.is_some())
            .unwrap_or(false),
        credentials: credentials_label(),
        last_server_url: last.server_url,
        last_name: last.name,
    })
}

fn device(grant: LocalGrant) -> RemoteDevice {
    RemoteDevice {
        grant_id: grant.id.to_string(),
        name: grant.device_name,
        control: grant.grant.allows(Scope::TerminalControl),
        projects: match grant.grant.projects {
            ProjectGrant::All => None,
            ProjectGrant::Only(projects) => Some(projects.iter().map(|p| p.to_string()).collect()),
        },
        approved_at: grant.approved_at as f64,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn remote_status(app: AppHandle) -> IpcResult<RemoteStatus> {
    blocking(app, status).await
}

/// Turn Remote on with the server at `server_url`, as `name`: sign in through the browser, then
/// register this computer as a host. Waits for the browser, for up to ten minutes, unless
/// [`remote_cancel`] is called.
#[tauri::command]
#[specta::specta]
pub async fn remote_enable(
    app: AppHandle,
    server_url: String,
    name: String,
) -> IpcResult<RemoteStatus> {
    if !available() {
        return Err(unavailable());
    }
    let name = name.trim().to_owned();
    // Checked before signing in, so a name the server would refuse leaves no device behind.
    HostRegistration {
        host: None,
        name: name.clone(),
    }
    .validate()
    .map_err(|e| IpcError::new("remote_name", e.message))?;
    let client = Client::new(&server_url).map_err(failed)?;
    let server = client.describe().await.map_err(failed)?;
    let sign_in = client.begin_sign_in().await.map_err(failed)?;
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    {
        let state = app.state::<AppState>();
        let mut signing_in = state
            .remote
            .signing_in
            .lock()
            .map_err(|_| IpcError::internal("poisoned"))?;
        if signing_in.is_some() {
            return Err(IpcError::new(
                "remote_busy",
                "A sign-in is already waiting for the browser.",
            ));
        }
        *signing_in = Some(cancel);
    }
    let finished = async {
        app.opener()
            .open_url(sign_in.url().as_str(), None::<&str>)
            .map_err(|e| IpcError::new("remote_browser", e.to_string()))?;
        let device = NewDevice {
            name: name.clone(),
            kind: DeviceKind::Host,
            platform: Platform::current(),
        };
        tokio::select! {
            credentials = sign_in.finish(&client, device, SIGN_IN_TIMEOUT) => credentials.map_err(failed),
            _ = cancelled => Err(IpcError::new("remote_cancelled", "Signing in was cancelled.")),
        }
    }
    .await;
    if let Ok(mut signing_in) = app.state::<AppState>().remote.signing_in.lock() {
        signing_in.take();
    }
    let signed_in = finished?;
    // Kept before anything else: from here on, it is the only way back in.
    keep_refresh(signed_in.device.as_str(), &signed_in.refresh)
        .map_err(|e| IpcError::new("remote_credentials", e))?;
    let mut session = Session::new(client.clone(), signed_in);
    let access = access(&mut session).await?;

    let state = app.state::<AppState>();
    let earlier = state.store.remote_host().map_err(store_failed)?;
    let again = earlier
        .as_ref()
        .filter(|h| h.server_id == server.server.as_str())
        .and_then(|h| HostId::new(h.host_id.clone()).ok());
    let registration = |host| HostRegistration {
        host,
        name: name.clone(),
    };
    let registered = match client
        .register_host(&access, &registration(again.clone()))
        .await
    {
        // The host was removed meanwhile, from here or elsewhere: start over as a new one.
        Err(e) if again.is_some() && e.code() == Some(ErrorCode::NotFound) => {
            client.register_host(&access, &registration(None)).await
        }
        other => other,
    };
    let registered = match registered {
        Ok(registered) => registered,
        Err(error) => {
            // Signed in, but not a host: sign that device out again rather than leave it.
            let _ = client.revoke_device(&access, &session.device).await;
            let _ = forget_refresh(session.device.as_str());
            return Err(failed(error));
        }
    };
    // The device this computer was signed in as before is signed out: it has no use now.
    if let Some(earlier) = earlier
        .as_ref()
        .filter(|h| h.server_id == server.server.as_str())
    {
        if let Ok(old) = remote_proto::DeviceId::new(earlier.device_id.clone()) {
            if old != session.device {
                let _ = client.revoke_device(&access, &old).await;
            }
        }
    }
    let row = RemoteHostRow {
        server_url: client.address().as_str().trim_end_matches('/').to_owned(),
        server_id: server.server.to_string(),
        host_id: registered.id.to_string(),
        device_id: session.device.to_string(),
        name: registered.name,
        enabled_at: now_ms(),
    };
    state.store.set_remote_host(&row).map_err(store_failed)?;
    if let Some(earlier) = earlier.filter(|h| h.device_id != row.device_id) {
        let _ = forget_refresh(&earlier.device_id);
    }
    // Only a convenience for next time: Remote is on whether or not it could be written.
    let _ = state.settings.update(|s| {
        s.remote = crate::settings::RemoteSettings {
            server_url: Some(row.server_url.clone()),
            name: Some(row.name.clone()),
        }
    });
    *state.remote.session.lock().await = Some(session);
    connect(&app);
    blocking(app, status).await
}

/// Stop waiting for the browser.
#[tauri::command]
#[specta::specta]
pub async fn remote_cancel(app: AppHandle) -> IpcResult<RemoteStatus> {
    if let Ok(mut signing_in) = app.state::<AppState>().remote.signing_in.lock() {
        if let Some(cancel) = signing_in.take() {
            let _ = cancel.send(());
        }
    }
    blocking(app, status).await
}

/// Turn Remote off: remove this host at the server and sign its device out there, then forget
/// both here. Done here even when the server cannot be reached, which the error then says.
#[tauri::command]
#[specta::specta]
pub async fn remote_disable(app: AppHandle) -> IpcResult<RemoteStatus> {
    let state = app.state::<AppState>();
    let Some(row) = state.store.remote_host().map_err(store_failed)? else {
        return blocking(app, status).await;
    };
    // Every device is let go of first: nothing comes through while Remote goes off.
    disconnect(&state).await;
    let at_server = async {
        let mut session = state.remote.session.lock().await;
        let session = ensure(&mut session, &row).await?;
        let access = access(session).await?;
        let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
        session
            .client
            .remove_host(&access, &host)
            .await
            .map_err(refused_here)?;
        session
            .client
            .revoke_device(&access, &session.device)
            .await
            .map_err(refused_here)
    }
    .await;
    *state.remote.session.lock().await = None;
    state.store.forget_remote_host().map_err(store_failed)?;
    forget_refresh(&row.device_id).map_err(|e| IpcError::new("remote_credentials", e))?;
    if let Err(error) = at_server {
        // Signed out at the server already is as good as removed.
        if error.code != "remote_signed_out" {
            return Err(IpcError::new(
                "remote_off_here_only",
                format!(
                    "Remote is off on this computer, but the server could not be told: {}. \
                     Sign this computer out from another device if it still shows there.",
                    error.message
                ),
            ));
        }
    }
    blocking(app, status).await
}

/// Make a pairing offer: a code to type, and a QR code, good once for five minutes.
#[tauri::command]
#[specta::specta]
pub async fn remote_pair(app: AppHandle) -> IpcResult<RemotePairing> {
    let state = app.state::<AppState>();
    let row = enabled(&state)?;
    let mut session = state.remote.session.lock().await;
    let session = ensure(&mut session, &row).await?;
    let access = access(session).await?;
    let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
    let offer = session
        .client
        .offer(&access, &host)
        .await
        .map_err(refused_here)?;
    let mut link = url::Url::parse("yardsort://pair").map_err(store_failed)?;
    link.query_pairs_mut()
        .append_pair("server", &row.server_url)
        .append_pair("secret", offer.secret.expose());
    let qr = qrcode::QrCode::new(link.as_str().as_bytes())
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let qr_svg = qr
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build();
    Ok(RemotePairing {
        code: offer.code.expose().to_owned(),
        qr_svg,
        expires_in_ms: offer.expires_in_ms.min(OFFER_TTL_MS as u64) as u32,
    })
}

/// Devices waiting for an answer, and the devices let in here once the server has said which
/// are still in force. The server can take a device off this list, never put one on it.
#[tauri::command]
#[specta::specta]
pub async fn remote_requests(app: AppHandle) -> IpcResult<RemoteRequests> {
    let state = app.state::<AppState>();
    let row = enabled(&state)?;
    let grants = host_grants(&state, &row).await?;
    let in_force: Vec<String> = grants
        .iter()
        .filter(|g| g.state == GrantState::Approved)
        .map(|g| g.id.to_string())
        .collect();
    state
        .store
        .retain_remote_grants(&in_force)
        .map_err(store_failed)?;
    let requests = grants
        .into_iter()
        .filter(|g| g.state == GrantState::Pending)
        .map(request)
        .collect();
    Ok(RemoteRequests {
        requests,
        status: blocking(app.clone(), status).await?,
    })
}

/// Answer a device that asked to be let in. Allowing gives it what it asked for that viewing
/// (and, with `control`, typing) covers, on the projects chosen; never more than it asked for.
#[tauri::command]
#[specta::specta]
pub async fn remote_answer(
    app: AppHandle,
    grant_id: String,
    answer: RemoteAnswer,
) -> IpcResult<RemoteStatus> {
    let state = app.state::<AppState>();
    let row = enabled(&state)?;
    let id =
        GrantId::new(grant_id).map_err(|_| IpcError::new("remote_refused", "No such request."))?;
    let asked = host_grants(&state, &row)
        .await?
        .into_iter()
        .find(|g| g.id == id && g.state == GrantState::Pending)
        .ok_or_else(|| {
            IpcError::new(
                "remote_refused",
                "That device is no longer waiting. Show a new code to pair it.",
            )
        })?;
    let decision = match answer {
        RemoteAnswer::Deny => None,
        RemoteAnswer::Allow { control, projects } => {
            let mut offered: BTreeSet<Scope> = VIEW.into();
            if control {
                offered.insert(Scope::TerminalControl);
            }
            let scopes: BTreeSet<Scope> = asked.requested.intersection(&offered).copied().collect();
            if scopes.is_empty() {
                return Err(IpcError::new(
                    "remote_refused",
                    "That device asked for nothing this computer gives.",
                ));
            }
            let projects = match projects {
                None => ProjectGrant::All,
                Some(ids) => ProjectGrant::Only(
                    ids.into_iter()
                        .map(ProjectId::new)
                        .collect::<Result<_, _>>()
                        .map_err(|_| IpcError::new("remote_refused", "Not a project."))?,
                ),
            };
            Some(GrantDecision { scopes, projects })
        }
    };
    let decided = {
        let mut session = state.remote.session.lock().await;
        let session = ensure(&mut session, &row).await?;
        let access = access(session).await?;
        session
            .client
            .decide(&access, &id, decision.as_ref())
            .await
            .map_err(refused_here)?
    };
    if let (Some(decision), GrantState::Approved) = (&decision, decided.state) {
        // What this host's person chose, as they chose it: the server's copy is not trusted for
        // what it says was given.
        host::keep_approved(
            &state.store,
            &decided.id,
            &decided.device,
            &decided.device_name,
            &decision.scopes,
            &decision.projects,
            now_ms(),
        )
        .map_err(store_failed)?;
    }
    blocking(app, status).await
}

/// Take a device's grant away. Refused here at once; the server is told too.
#[tauri::command]
#[specta::specta]
pub async fn remote_revoke(app: AppHandle, grant_id: String) -> IpcResult<RemoteStatus> {
    let state = app.state::<AppState>();
    let row = enabled(&state)?;
    state
        .store
        .drop_remote_grant(&grant_id)
        .map_err(store_failed)?;
    // Its channel closes now, not at the next check.
    if let Some(connector) = state
        .remote
        .connector
        .lock()
        .ok()
        .as_deref()
        .and_then(Option::as_ref)
    {
        connector.recheck();
    }
    let id =
        GrantId::new(grant_id).map_err(|_| IpcError::new("remote_refused", "No such device."))?;
    {
        let mut session = state.remote.session.lock().await;
        let session = ensure(&mut session, &row).await?;
        let access = access(session).await?;
        session
            .client
            .revoke_grant(&access, &id)
            .await
            .map_err(refused_here)?;
    }
    blocking(app, status).await
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn enabled(state: &AppState) -> IpcResult<RemoteHostRow> {
    if !available() {
        return Err(unavailable());
    }
    state
        .store
        .remote_host()
        .map_err(store_failed)?
        .ok_or_else(|| IpcError::new("remote_off", "Remote is off on this computer."))
}

async fn host_grants(state: &AppState, row: &RemoteHostRow) -> IpcResult<Vec<GrantSummary>> {
    let mut session = state.remote.session.lock().await;
    let session = ensure(&mut session, row).await?;
    let access = access(session).await?;
    let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
    Ok(session
        .client
        .host_grants(&access, &host)
        .await
        .map_err(refused_here)?
        .grants)
}

/// The session, resumed from the kept refresh credential after a restart.
async fn ensure<'a>(
    slot: &'a mut Option<Session>,
    row: &RemoteHostRow,
) -> IpcResult<&'a mut Session> {
    if slot.is_none() {
        let refresh = credentials()?
            .get(&refresh_entry(&row.device_id))
            .map_err(|e| IpcError::new("remote_credentials", e))?
            .ok_or_else(|| signed_out("This computer's sign-in is missing."))?;
        let client = Client::new(&row.server_url).map_err(failed)?;
        let session = Session::resume(client, &refresh)
            .await
            .map_err(refused_here)?;
        let session = slot.insert(session);
        // The kept one is spent now. Should this fail, the session carries on with the new one
        // until the app quits, and the error says why the next start will be signed out.
        keep_refresh(&row.device_id, session.refresh_credential())
            .map_err(|e| IpcError::new("remote_credentials", e))?;
        return Ok(session);
    }
    Ok(slot.as_mut().expect("just filled"))
}

#[cfg(test)]
mod tests {
    use remote_proto::RemoteError;

    use super::*;

    #[test]
    fn a_refusal_of_this_computers_credential_says_it_was_signed_out() {
        let refused = |code| ClientError::Remote(RemoteError::new(code, "Sign in again."));
        let error = refused_here(refused(ErrorCode::Unauthenticated));
        assert_eq!(error.code, "remote_signed_out");
        assert_eq!(
            error.message,
            "This computer was signed out at the server. Turn Remote off, then on again."
        );
        // The link's own line adds what to do.
        assert!(matches!(
            access_error(error),
            AccessError::SignedOut(why) if why == "This computer was signed out at the server."
        ));
        assert_eq!(
            refused_here(refused(ErrorCode::NotFound)).code,
            "remote_refused"
        );
    }
}
