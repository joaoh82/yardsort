//! Remote: this computer as a host on a Yardsort server, and the devices its person lets in.
//! See [`yardsort_core::remote::host`] for what is kept where, and
//! [docs/design/27-remote-r2.md](../../docs/design/27-remote-r2.md) for the design.
//!
//! **A preview, off unless `YARDSORT_REMOTE_PREVIEW=1`.** Pairing works, but nothing can reach a
//! paired host yet: the relay that will carry a device's requests comes in a later slice. Until
//! then the Remote settings are not shown, and every command here refuses.
//!
//! The host signs in through the system browser, like any device, and keeps its refresh
//! credential in the OS credential store (or `YARDSORT_CREDENTIALS_DIR`, a directory only its
//! owner can read, where there is none). The webview never sees a credential: it sees whether
//! Remote is on, the pairing code and QR code while one is shown, and the devices let in.
//!
//! Only these commands approve a device, and only the desktop calls them. Nothing in `ys`, and
//! nothing an agent can reach, approves one.

use std::collections::BTreeSet;

use remote_client::{Client, ClientError, Session, SIGN_IN_TIMEOUT};
use remote_proto::account::{DeviceKind, NewDevice, Secret};
use remote_proto::pairing::{
    GrantDecision, GrantState, GrantSummary, HostRegistration, OFFER_TTL_MS,
};
use remote_proto::{ErrorCode, GrantId, HostId, Platform, ProjectGrant, ProjectId, Scope};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use yardsort_core::remote::host::{
    self, CredentialStore, FileCredentials, LocalGrant, REFRESH_ENTRY,
};
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

/// The host's session with its server, and a sign-in waiting for the browser.
#[derive(Default)]
pub struct Remote {
    session: tokio::sync::Mutex<Option<Session>>,
    signing_in: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
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
            _ => "remote_unreachable",
        },
    };
    IpcError::new(code, error.to_string())
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

fn keep_refresh(refresh: &Secret) -> Result<(), String> {
    credentials()
        .map_err(|e| e.message)?
        .set(REFRESH_ENTRY, refresh)
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
    keep_refresh(&signed_in.refresh).map_err(|e| IpcError::new("remote_credentials", e))?;
    let mut session = Session::new(client.clone(), signed_in);
    let access = session.access(keep_refresh).await.map_err(failed)?;

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
            let _ = credentials().map(|c| c.delete(REFRESH_ENTRY));
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
    // Only a convenience for next time: Remote is on whether or not it could be written.
    let _ = state.settings.update(|s| {
        s.remote = crate::settings::RemoteSettings {
            server_url: Some(row.server_url.clone()),
            name: Some(row.name.clone()),
        }
    });
    *state.remote.session.lock().await = Some(session);
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
    let at_server = async {
        let mut session = state.remote.session.lock().await;
        let session = ensure(&mut session, &row).await?;
        let access = session.access(keep_refresh).await.map_err(failed)?;
        let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
        session
            .client
            .remove_host(&access, &host)
            .await
            .map_err(failed)?;
        session
            .client
            .revoke_device(&access, &session.device)
            .await
            .map_err(failed)
    }
    .await;
    *state.remote.session.lock().await = None;
    state.store.forget_remote_host().map_err(store_failed)?;
    credentials()?
        .delete(REFRESH_ENTRY)
        .map_err(|e| IpcError::new("remote_credentials", e))?;
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
    let access = session.access(keep_refresh).await.map_err(failed)?;
    let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
    let offer = session.client.offer(&access, &host).await.map_err(failed)?;
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
        .map(|g| RemoteRequest {
            wants_control: g.requested.contains(&Scope::TerminalControl),
            grant_id: g.id.to_string(),
            device_name: g.device_name,
            expires_in_ms: g.expires_in_ms.unwrap_or_default().min(OFFER_TTL_MS as u64) as u32,
        })
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
        let access = session.access(keep_refresh).await.map_err(failed)?;
        session
            .client
            .decide(&access, &id, decision.as_ref())
            .await
            .map_err(failed)?
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
    let id =
        GrantId::new(grant_id).map_err(|_| IpcError::new("remote_refused", "No such device."))?;
    {
        let mut session = state.remote.session.lock().await;
        let session = ensure(&mut session, &row).await?;
        let access = session.access(keep_refresh).await.map_err(failed)?;
        session
            .client
            .revoke_grant(&access, &id)
            .await
            .map_err(failed)?;
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
    let access = session.access(keep_refresh).await.map_err(failed)?;
    let host = HostId::new(row.host_id.clone()).map_err(store_failed)?;
    Ok(session
        .client
        .host_grants(&access, &host)
        .await
        .map_err(failed)?
        .grants)
}

/// The session, resumed from the kept refresh credential after a restart.
async fn ensure<'a>(
    slot: &'a mut Option<Session>,
    row: &RemoteHostRow,
) -> IpcResult<&'a mut Session> {
    if slot.is_none() {
        let refresh = credentials()?
            .get(REFRESH_ENTRY)
            .map_err(|e| IpcError::new("remote_credentials", e))?
            .ok_or_else(|| {
                IpcError::new(
                    "remote_signed_out",
                    "This computer's sign-in is missing. Turn Remote off, then on again.",
                )
            })?;
        let client = Client::new(&row.server_url).map_err(failed)?;
        let session = Session::resume(client, &refresh, keep_refresh)
            .await
            .map_err(|e| match e.code() {
                Some(ErrorCode::Unauthenticated) => IpcError::new(
                    "remote_signed_out",
                    "This computer was signed out at the server. Turn Remote off, then on again.",
                ),
                _ => failed(e),
            })?;
        *slot = Some(session);
    }
    Ok(slot.as_mut().expect("just filled"))
}
