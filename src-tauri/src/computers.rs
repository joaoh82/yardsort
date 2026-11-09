//! Remote, the other way round: this computer reaching other hosts, as a phone does. Settings →
//! Remote → Other computers signs it in as a client and pairs it by a host's code; the sidebar
//! lists the hosts it is paired with, and each opens read-only. See [`remote_host::computer`] for
//! what is kept where, and docs/design/35-remote-r4-r5.md §3, "The desktop as a client".
//!
//! **Apart from this computer as a host.** It is a second device at the server, with its own
//! sign-in, and turning Remote on or off here leaves it be, and the other way round.
//!
//! **Nothing from another computer is taken for local.** What a host shows has no paths, and no
//! command here touches the files, Git or terminals of this computer for it: the window shows it,
//! named by its host, and offers nothing that would act here.

use std::sync::Arc;

use remote_client::SIGN_IN_TIMEOUT;
use remote_host::account::{self, Failure};
use remote_host::computer::{self, Computer, ComputerHome, HostView, Link, Paired};
use remote_proto::account::{DeviceKind, NewDevice};
use remote_proto::pairing::GrantState;
use remote_proto::{Platform, TerminalActivity, TerminalPurpose};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tauri_specta::Event;
use yardsort_core::remote::host::CredentialStore;
use yardsort_core::store::Store;

use crate::error::{IpcError, IpcResult};
use crate::state::AppState;

/// The client's session, its links to other hosts, and its sign-in waiting for the browser.
#[derive(Default)]
pub struct ComputersState {
    computer: Computer,
    signing_in: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

struct AppComputer(AppHandle);

impl ComputerHome for AppComputer {
    fn computer(&self) -> &Computer {
        &self.0.state::<AppState>().inner().computers.computer
    }

    fn store(&self) -> &Store {
        &self.0.state::<AppState>().inner().store
    }

    fn credentials(&self) -> account::Result<Box<dyn CredentialStore>> {
        crate::remote::credentials()
    }

    fn client_name(&self) -> String {
        format!(
            "yardsort-desktop/{} ({})",
            env!("CARGO_PKG_VERSION"),
            platform_name()
        )
    }

    fn changed(&self) {
        let _ = OtherComputersChanged {}.emit(&self.0);
    }
}

fn platform_name() -> &'static str {
    match Platform::current() {
        Platform::Macos => "macos",
        Platform::Windows => "windows",
        _ => "linux",
    }
}

fn home(app: &AppHandle) -> Arc<AppComputer> {
    Arc::new(AppComputer(app.clone()))
}

/// Reach the hosts this computer is paired with, when it is signed in as a client. At start.
pub fn start(app: &AppHandle) {
    computer::start(&home(app));
}

/// Something about the other computers changed: the list, a link, or what one shows.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct OtherComputersChanged {}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherComputers {
    /// This computer as a client, when it is signed in as one.
    pub client: Option<OtherClient>,
    /// A sign-in is waiting for the browser.
    pub signing_in: bool,
    /// The hosts it asked to reach, by the server's word, by name.
    pub hosts: Vec<OtherHost>,
    /// Why they could not be listed just now, in words for a person.
    pub problem: Option<String>,
    /// Signed out at the server: signing in again is the way back.
    pub signed_out: bool,
    /// What to offer when signing in: the server this computer is a host on, and its name.
    pub suggested_server_url: Option<String>,
    pub suggested_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherClient {
    pub server_url: String,
    pub name: String,
    pub since: f64,
}

/// A host this computer asked to reach.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherHost {
    pub grant_id: String,
    pub host_id: String,
    pub name: String,
    pub pairing: OtherPairing,
    /// While let in: how its link stands.
    pub link: Option<OtherLink>,
}

#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OtherPairing {
    /// Waiting for the person at that computer.
    Waiting,
    /// Let in, to watch.
    Paired,
    Refused,
    /// Nobody answered in time.
    Lapsed,
    /// Taken back, there or here.
    Revoked,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OtherLink {
    Connecting,
    Online,
    Offline { since: f64, why: String },
}

/// What a host shows, as this computer last read it. Read-only: nothing here acts on it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherHostView {
    pub host_id: String,
    pub name: String,
    pub link: OtherLink,
    /// When it was last read; `None` until it has been.
    pub read_at: Option<f64>,
    pub platform: Option<String>,
    pub app_version: Option<String>,
    pub projects: Vec<OtherProject>,
    /// The newest first.
    pub activity: Vec<OtherActivity>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherProject {
    pub id: String,
    pub name: String,
    pub workspaces: Vec<OtherWorkspace>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherWorkspace {
    pub id: String,
    pub name: String,
    pub branch: Option<String>,
    pub archived: bool,
    pub sessions: Vec<OtherSession>,
    pub terminals: Vec<OtherTerminal>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherSession {
    pub id: String,
    /// What its tab is called there. It may quote the first message: shown, never logged.
    pub title: String,
    pub harness: String,
    /// `starting`, `busy`, `quiet` or `exited`, while its terminal is there; `ended` otherwise.
    pub state: String,
    pub started_at: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherTerminal {
    pub id: String,
    pub program: String,
    /// `shell`, `run`, `agent` or `program`.
    pub purpose: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherActivity {
    pub id: String,
    pub kind: String,
    pub occurred_at: f64,
    pub workspace: String,
}

fn ipc(failure: Failure) -> IpcError {
    IpcError::new(failure.code, failure.message)
}

fn link(link: &Link) -> OtherLink {
    match link {
        Link::Connecting => OtherLink::Connecting,
        Link::Online => OtherLink::Online,
        Link::Offline { since_ms, why } => OtherLink::Offline {
            since: *since_ms as f64,
            why: why.clone(),
        },
    }
}

fn host(paired: Paired) -> OtherHost {
    OtherHost {
        grant_id: paired.grant.id.to_string(),
        host_id: paired.grant.host.to_string(),
        name: paired.grant.host_name,
        pairing: match paired.grant.state {
            GrantState::Pending => OtherPairing::Waiting,
            GrantState::Approved => OtherPairing::Paired,
            GrantState::Denied => OtherPairing::Refused,
            GrantState::Expired => OtherPairing::Lapsed,
            _ => OtherPairing::Revoked,
        },
        link: paired.view.map(|view| link(&view.link)),
    }
}

fn activity_name(activity: TerminalActivity) -> &'static str {
    match activity {
        TerminalActivity::Starting => "starting",
        TerminalActivity::Busy => "busy",
        TerminalActivity::Quiet => "quiet",
        TerminalActivity::Exited => "exited",
    }
}

fn view(host_id: &str, name: String, view: HostView) -> OtherHostView {
    let Some(shown) = view.shown else {
        return OtherHostView {
            host_id: host_id.to_owned(),
            name,
            link: link(&view.link),
            read_at: None,
            platform: None,
            app_version: None,
            projects: vec![],
            activity: vec![],
        };
    };
    let mut names = std::collections::HashMap::new();
    let mut projects: Vec<OtherProject> = shown
        .projects
        .iter()
        .map(|p| OtherProject {
            id: p.id.to_string(),
            name: p.name.clone(),
            workspaces: vec![],
        })
        .collect();
    for (workspace, page) in shown.workspaces {
        names.insert(workspace.id.to_string(), workspace.name.clone());
        let sessions = page
            .sessions
            .into_iter()
            .map(|s| OtherSession {
                id: s.id.to_string(),
                title: s.title,
                harness: s.harness,
                state: s
                    .terminal
                    .map_or("ended", |t| activity_name(t.activity))
                    .to_owned(),
                started_at: s.started_at as f64,
            })
            .collect();
        let terminals = page
            .terminals
            .into_iter()
            .map(|t| OtherTerminal {
                id: t.terminal.id.to_string(),
                program: t.program,
                purpose: match t.purpose {
                    TerminalPurpose::Shell => "shell",
                    TerminalPurpose::RunCommand => "run",
                    TerminalPurpose::Agent => "agent",
                    _ => "program",
                }
                .to_owned(),
                state: activity_name(t.terminal.activity).to_owned(),
            })
            .collect();
        let entry = OtherWorkspace {
            id: workspace.id.to_string(),
            name: workspace.name,
            branch: workspace.branch.map(|b| b.label),
            archived: workspace.archived,
            sessions,
            terminals,
        };
        let project = workspace.project.to_string();
        match projects.iter_mut().find(|p| p.id == project) {
            Some(project) => project.workspaces.push(entry),
            None => projects.push(OtherProject {
                id: project,
                name: "Project".into(),
                workspaces: vec![entry],
            }),
        }
    }
    let activity = shown
        .activity
        .into_iter()
        .map(|event| OtherActivity {
            id: event.id.to_string(),
            workspace: names
                .get(event.workspace.as_str())
                .cloned()
                .unwrap_or_default(),
            kind: event.kind,
            occurred_at: event.occurred_at as f64,
        })
        .collect();
    OtherHostView {
        host_id: host_id.to_owned(),
        name: shown.host.name.clone(),
        link: link(&view.link),
        read_at: Some(shown.read_at_ms as f64),
        platform: Some(platform_label(shown.host.platform).to_owned()),
        app_version: Some(shown.host.app_version),
        projects,
        activity,
    }
}

fn platform_label(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macOS",
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
        _ => "another system",
    }
}

fn status(app: &AppHandle) -> IpcResult<OtherComputers> {
    let state = app.state::<AppState>();
    let state = state.inner();
    let row = state
        .store
        .remote_computer()
        .map_err(|e| IpcError::new("remote_store", e.to_string()))?;
    let (hosts, problem) = if row.is_some() {
        computer::paired(&home(app))
    } else {
        (vec![], None)
    };
    let host_row = state.store.remote_host().ok().flatten();
    let last = state.settings.get().remote;
    let signing_in = state
        .computers
        .signing_in
        .lock()
        .map(|s| s.is_some())
        .unwrap_or(false);
    Ok(OtherComputers {
        client: row.map(|row| OtherClient {
            server_url: row.server_url,
            name: row.name,
            since: row.signed_in_at as f64,
        }),
        signing_in,
        hosts: hosts.into_iter().map(host).collect(),
        signed_out: problem.as_ref().is_some_and(Failure::is_signed_out),
        problem: problem.map(|p| p.message),
        suggested_server_url: host_row
            .as_ref()
            .map(|h| h.server_url.clone())
            .or(last.server_url),
        suggested_name: host_row.map(|h| h.name).or(last.name),
    })
}

/// This computer as a client of other hosts, and the hosts it asked to reach.
#[tauri::command]
#[specta::specta]
pub async fn other_computers(app: AppHandle) -> IpcResult<OtherComputers> {
    status(&app)
}

/// Sign in as a client: a device apart from this computer's host device, through the browser.
#[tauri::command]
#[specta::specta]
pub async fn other_computers_sign_in(
    app: AppHandle,
    server_url: String,
    name: String,
) -> IpcResult<OtherComputers> {
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
            .computers
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
            kind: DeviceKind::Computer,
            platform: Platform::current(),
        };
        tokio::select! {
            credentials = sign_in.finish(&client, device, SIGN_IN_TIMEOUT) => credentials.map_err(|e| ipc(account::failed(e))),
            _ = cancelled => Err(IpcError::new("remote_cancelled", "Signing in was cancelled.")),
        }
    }
    .await;
    if let Ok(mut signing_in) = app.state::<AppState>().computers.signing_in.lock() {
        signing_in.take();
    }
    let signed_in = finished?;
    computer::sign_in(&home(&app), client, &server, &name, signed_in)
        .await
        .map_err(ipc)?;
    status(&app)
}

/// Stop waiting for the browser.
#[tauri::command]
#[specta::specta]
pub async fn other_computers_cancel(app: AppHandle) -> IpcResult<OtherComputers> {
    if let Ok(mut signing_in) = app.state::<AppState>().computers.signing_in.lock() {
        if let Some(cancel) = signing_in.take() {
            let _ = cancel.send(());
        }
    }
    status(&app)
}

/// Sign the client out, here and at the server. This computer as a host is left as it is.
#[tauri::command]
#[specta::specta]
pub async fn other_computers_sign_out(app: AppHandle) -> IpcResult<OtherComputers> {
    computer::sign_out(home(&app).as_ref()).await.map_err(ipc)?;
    status(&app)
}

/// Ask the host showing `code` to let this computer watch it. Its person answers there.
#[tauri::command]
#[specta::specta]
pub async fn other_computers_pair(app: AppHandle, code: String) -> IpcResult<OtherComputers> {
    computer::pair(&home(&app), &code).await.map_err(ipc)?;
    status(&app)
}

/// Stop reaching a host: the pairing is taken back at the server.
#[tauri::command]
#[specta::specta]
pub async fn other_computers_forget(app: AppHandle, grant_id: String) -> IpcResult<OtherComputers> {
    computer::forget(&home(&app), &grant_id)
        .await
        .map_err(ipc)?;
    status(&app)
}

/// What a host shows, as this computer last read it; `None` for a host it may not reach.
#[tauri::command]
#[specta::specta]
pub async fn other_host(app: AppHandle, host_id: String) -> IpcResult<Option<OtherHostView>> {
    let home = home(&app);
    let Some(found) = computer::view(home.as_ref(), &host_id) else {
        return Ok(None);
    };
    let (hosts, _) = computer::paired(&home);
    let name = hosts
        .into_iter()
        .find(|p| p.grant.host.as_str() == host_id)
        .map(|p| p.grant.host_name)
        .unwrap_or_default();
    Ok(Some(view(&host_id, name, found)))
}
