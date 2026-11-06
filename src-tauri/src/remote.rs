//! Remote: this computer as a host on a Yardsort server, and the devices its person lets in.
//! See [`yardsort_core::remote::host`] for what is kept where, and
//! [docs/design/27-remote-r2.md](../../docs/design/27-remote-r2.md) for the design.
//!
//! **Still under development, and off until the person turns it on and signs in.** Until then
//! nothing connects. A host that is on keeps a link to its server's relay
//! ([`remote_host::Connector`]) from the moment the app starts, and paired devices reach it
//! through that.
//!
//! The host signs in through the system browser, like any device, and keeps its refresh
//! credential in the OS credential store (or `YARDSORT_CREDENTIALS_DIR`, a directory only its
//! owner can read, where there is none). The webview never sees a credential: it sees whether
//! Remote is on, the pairing code and QR code while one is shown, and the devices let in.
//!
//! What a host does at its server is [`remote_host::account`], shared with a headless host; this
//! is the window's side of it. Only these commands approve a device, and only the desktop calls
//! them. Nothing in `ys`, and nothing an agent can reach, approves one.
//!
//! **A device asking is shown at once.** The relay tells the host on its link
//! ([`HostNotice`]); the app then asks the server, as Settings → Remote does, and tells the window
//! of the request only if the server has it waiting ([`RemotePairingRequested`]). The window
//! offers the answer; it never answers by itself.

use std::sync::{Arc, OnceLock};

use pty_host::{SessionInfo, SessionState};
use remote_client::SIGN_IN_TIMEOUT;
use remote_host::account::{self, Answer, Failure, Home, Remote, Request};
use remote_proto::account::{DeviceKind, NewDevice, Secret};
use remote_proto::{Platform, ProjectGrant, Scope};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tauri_specta::Event;
use yardsort_core::remote::host::{self, CredentialStore, FileCredentials, LocalGrant};
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::HostIdentity;
use yardsort_core::settings::SettingsFile;
use yardsort_core::store::Store;

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};
use crate::{quit, updates};

/// A directory to keep the host's credential in, instead of the OS credential store.
pub const CREDENTIALS_DIR_VAR: &str = "YARDSORT_CREDENTIALS_DIR";
const KEYRING_SERVICE: &str = "dev.yardsort.app";

/// The host's session with its server and its sign-in waiting for the browser.
#[derive(Default)]
pub struct RemoteState {
    account: Remote,
    signing_in: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

/// This app as a host: its records and terminals, and the window to tell of a device asking.
struct AppHome {
    app: AppHandle,
    git: OnceLock<Option<yardsort_core::git::Git>>,
}

impl AppHome {
    fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
            git: OnceLock::new(),
        }
    }

    fn state(&self) -> &AppState {
        self.app.state::<AppState>().inner()
    }
}

impl Home for AppHome {
    fn remote(&self) -> &Remote {
        &self.state().remote.account
    }

    fn store(&self) -> &Store {
        &self.state().store
    }

    fn settings(&self) -> &SettingsFile {
        &self.state().settings
    }

    fn credentials(&self) -> account::Result<Box<dyn CredentialStore>> {
        credentials()
    }

    fn serve(&self, f: &mut dyn FnMut(&Context<'_>)) {
        let state = self.state();
        let Some(git) = self
            .git
            .get_or_init(|| yardsort_core::git::Git::new(&state.env()).ok())
        else {
            return;
        };
        let Ok(Some(row)) = state.store.remote_host() else {
            return;
        };
        let Ok(id) = remote_proto::HostId::new(row.host_id) else {
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

    fn asked(&self, request: Request) {
        let _ = RemotePairingRequested {
            request: request.into(),
        }
        .emit(&self.app);
    }
}

/// Keep a link to the relay, when Remote is on: at start, and once it is turned on. Needs a
/// Tokio runtime: call it from async code.
pub fn connect(app: &AppHandle) {
    account::connect(&Arc::new(AppHome::new(app)));
}

/// A device asks to be let in: shown in the window wherever it is, until answered or lapsed.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingRequested {
    pub request: RemoteRequest,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    /// This computer as a host, when Remote is on.
    pub host: Option<RemoteHost>,
    /// The devices let in here.
    pub devices: Vec<RemoteDevice>,
    /// A sign-in is waiting for the browser.
    pub signing_in: bool,
    /// Where the host's credential is kept: the OS credential store, or a directory.
    pub credentials: String,
    /// Set while this computer's terminals run in a daemon that cannot share them.
    pub earlier_daemon: Option<RemoteEarlierDaemon>,
    /// The server and name Remote was last turned on with, to offer again once it is off.
    pub last_server_url: Option<String>,
    pub last_name: Option<String>,
}

/// A daemon from before leases, kept for the programs running in it, and what replacing it stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEarlierDaemon {
    /// The Yardsort version that started it.
    pub version: Option<String>,
    /// The running agents, as ids the frontend can match to its tabs.
    pub agents: Vec<String>,
    /// How many running sessions are not agents.
    pub shells: u32,
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

fn link(app: &AppHandle) -> RemoteLink {
    let Some(status) = account::link(&AppHome::new(app)) else {
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

/// What to do once signed out at the server.
const AGAIN: &str = "Sign in again in Settings → Remote.";

/// A failure as the window shows it: signed out in the host's own words, it adds where to sign in
/// again. The server's own refusal already ends "Sign in again.", and is shown as it is.
fn ipc(failure: Failure) -> IpcError {
    if failure.needs_where_to_sign_in() {
        IpcError::new(failure.code, format!("{} {AGAIN}", failure.message))
    } else {
        IpcError::new(failure.code, failure.message)
    }
}

fn store_failed(error: impl std::fmt::Display) -> IpcError {
    IpcError::new("remote_store", error.to_string())
}

/// Where the host's credential is kept.
fn credentials() -> account::Result<Box<dyn CredentialStore>> {
    match std::env::var_os(CREDENTIALS_DIR_VAR) {
        Some(dir) => Ok(Box::new(
            FileCredentials::new(dir).map_err(|e| Failure::new("remote_credentials", e))?,
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

fn status(app: &AppHandle) -> IpcResult<RemoteStatus> {
    let state = app.state::<AppState>();
    let state = state.inner();
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
        host: host.map(|h| RemoteHost {
            server_url: h.server_url,
            name: h.name,
            since: h.enabled_at as f64,
            link: link(app),
        }),
        devices,
        signing_in: state
            .remote
            .signing_in
            .lock()
            .map(|s| s.is_some())
            .unwrap_or(false),
        credentials: credentials_label(),
        earlier_daemon: earlier_daemon(state),
        last_server_url: last.server_url,
        last_name: last.name,
    })
}

fn earlier_daemon(state: &AppState) -> Option<RemoteEarlierDaemon> {
    // Without a daemon the terminals are in this process, which shares them.
    if state.daemon_client.is_none() || state.host.shares_control() {
        return None;
    }
    Some(earlier_daemon_of(
        state.daemon.version.clone(),
        &state.host.list(),
    ))
}

fn earlier_daemon_of(version: Option<String>, sessions: &[SessionInfo]) -> RemoteEarlierDaemon {
    let agents = quit::agents_among(sessions);
    let running = sessions
        .iter()
        .filter(|session| matches!(session.state, SessionState::Running))
        .count();
    RemoteEarlierDaemon {
        version,
        shells: u32::try_from(running - agents.len()).unwrap_or(u32::MAX),
        agents,
    }
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

impl From<Request> for RemoteRequest {
    fn from(request: Request) -> Self {
        Self {
            grant_id: request.grant_id,
            device_name: request.device_name,
            wants_control: request.wants_control,
            expires_in_ms: request.expires_in_ms,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn remote_status(app: AppHandle) -> IpcResult<RemoteStatus> {
    status_of(app).await
}

async fn status_of(app: AppHandle) -> IpcResult<RemoteStatus> {
    let handle = app.clone();
    blocking(app, move |_| status(&handle)).await
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
    let name = name.trim().to_owned();
    let (client, server) = account::prepare(&server_url, &name).await.map_err(ipc)?;
    let sign_in = client
        .begin_sign_in()
        .await
        .map_err(|e| ipc(account::failed(e)))?;
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
            credentials = sign_in.finish(&client, device, SIGN_IN_TIMEOUT) => credentials.map_err(|e| ipc(account::failed(e))),
            _ = cancelled => Err(IpcError::new("remote_cancelled", "Signing in was cancelled.")),
        }
    }
    .await;
    if let Ok(mut signing_in) = app.state::<AppState>().remote.signing_in.lock() {
        signing_in.take();
    }
    let signed_in = finished?;
    account::enable(
        &Arc::new(AppHome::new(&app)),
        client,
        &server,
        &name,
        signed_in,
    )
    .await
    .map_err(ipc)?;
    status_of(app).await
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
    status_of(app).await
}

/// Turn Remote off: remove this host at the server and sign its device out there, then forget
/// both here. Done here even when the server cannot be reached, which the error then says.
#[tauri::command]
#[specta::specta]
pub async fn remote_disable(app: AppHandle) -> IpcResult<RemoteStatus> {
    account::disable(&AppHome::new(&app)).await.map_err(ipc)?;
    status_of(app).await
}

/// Make a pairing offer: a code to type, and a QR code, good once for five minutes.
#[tauri::command]
#[specta::specta]
pub async fn remote_pair(app: AppHandle) -> IpcResult<RemotePairing> {
    let offer = account::pair(&AppHome::new(&app)).await.map_err(ipc)?;
    let qr = qrcode::QrCode::new(offer.link.as_bytes())
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let qr_svg = qr
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build();
    Ok(RemotePairing {
        code: offer.code,
        qr_svg,
        expires_in_ms: offer.expires_in_ms,
    })
}

/// Devices waiting for an answer, and the devices let in here once the server has said which
/// are still in force. The server can take a device off this list, never put one on it.
#[tauri::command]
#[specta::specta]
pub async fn remote_requests(app: AppHandle) -> IpcResult<RemoteRequests> {
    let requests = account::requests(&AppHome::new(&app))
        .await
        .map_err(ipc)?
        .into_iter()
        .map(RemoteRequest::from)
        .collect();
    Ok(RemoteRequests {
        requests,
        status: status_of(app).await?,
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
    let answer = match answer {
        RemoteAnswer::Deny => Answer::Deny,
        RemoteAnswer::Allow { control, projects } => Answer::Allow { control, projects },
    };
    account::answer(&AppHome::new(&app), grant_id, answer)
        .await
        .map_err(ipc)?;
    status_of(app).await
}

/// Stop everything in a daemon that cannot share its terminals, and start Yardsort again, so that
/// a current daemon takes over. Only ever on the person's word, once told what it stops. A daemon
/// that does share them is left alone, whatever asked.
#[tauri::command]
#[specta::specta]
pub async fn remote_replace_daemon(app: AppHandle) -> IpcResult<()> {
    let restarting = app.clone();
    blocking(app, |state| {
        if earlier_daemon(state).is_none() {
            return Err(IpcError::new(
                "remote_daemon_current",
                "This computer's terminals can already be shared.",
            ));
        }
        quit::stop_everything(state);
        Ok(())
    })
    .await?;
    quit::past_the_question();
    updates::restart(&restarting)
}

/// Take a device's grant away. Refused here at once; the server is told too.
#[tauri::command]
#[specta::specta]
pub async fn remote_revoke(app: AppHandle, grant_id: String) -> IpcResult<RemoteStatus> {
    account::revoke(&AppHome::new(&app), grant_id)
        .await
        .map_err(ipc)?;
    status_of(app).await
}

#[cfg(test)]
mod tests {
    use remote_client::ClientError;
    use remote_proto::{ErrorCode, RemoteError};

    use super::*;

    fn session(id: &str, harness: bool, running: bool) -> SessionInfo {
        let mut labels = serde_json::Map::new();
        if harness {
            labels.insert(crate::terminal::HARNESS_LABEL.to_owned(), "claude".into());
        }
        let state = if running {
            serde_json::json!({ "status": "running" })
        } else {
            serde_json::json!({ "status": "exited", "exit": { "code": 0, "success": true } })
        };
        serde_json::from_value(serde_json::json!({
            "id": id, "program": "p", "args": [], "cwd": null, "pid": null,
            "size": { "cols": 80, "rows": 24 }, "labels": labels, "state": state,
            "hasOutput": false, "busy": false, "idleMs": 0,
        }))
        .unwrap()
    }

    /// What the person is told replacing the daemon stops: everything still running in it, a
    /// shell at a prompt too, and nothing that has already ended.
    #[test]
    fn replacing_an_earlier_daemon_counts_all_that_it_stops() {
        let sessions = [
            session("agent", true, true),
            session("finished-agent", true, false),
            session("shell", false, true),
            session("finished-shell", false, false),
        ];
        assert_eq!(
            earlier_daemon_of(Some("0.20.0".into()), &sessions),
            RemoteEarlierDaemon {
                version: Some("0.20.0".into()),
                agents: vec!["agent".into()],
                shells: 1,
            }
        );
    }

    /// Signed out at the server, the window adds where to sign in again to the host's own words;
    /// the link's own line, which the account module makes, says only why.
    #[test]
    fn a_refusal_of_this_computers_credential_says_it_was_signed_out() {
        let refused = |code| ClientError::Remote(RemoteError::new(code, "Sign in again."));
        let error = ipc(account::refused_here(refused(ErrorCode::Unauthenticated)));
        assert_eq!(error.code, "remote_signed_out");
        assert_eq!(
            error.message,
            "This computer was signed out at the server. Sign in again in Settings → Remote."
        );
        assert_eq!(
            ipc(account::refused_here(refused(ErrorCode::NotFound))).code,
            "remote_refused"
        );
        // Refused while signing in, or registering: the server's words, said once.
        let theirs = ipc(account::failed(refused(ErrorCode::Unauthenticated)));
        assert_eq!(theirs.code, "remote_signed_out");
        assert_eq!(theirs.message, "Sign in again.");
    }
}
