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
use remote_proto::{Platform, TerminalActivity, TerminalPurpose, WorkspaceKind};
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

pub(crate) struct AppComputer(AppHandle);

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

pub(crate) fn home(app: &AppHandle) -> Arc<AppComputer> {
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
    /// Whether that computer lets this one type into its terminals, once it takes control.
    pub may_type: bool,
    /// Whether this computer may start work there: it may type, and that computer can.
    pub may_start: bool,
    /// Whether this computer may resume that computer's ended conversations: it may type, and
    /// that computer can.
    pub may_resume: bool,
    /// Whether a start there may say a name, a model, an effort and a branch, and this computer
    /// may clone a project there, start a shell, and rename, archive and delete a workspace: it
    /// may type, and that computer's Yardsort is new enough.
    pub may_manage: bool,
    /// Whether this computer may read that computer's changes and files: it may type, and that
    /// computer's Yardsort is new enough.
    pub may_look: bool,
    /// Whether this computer may commit, push, and open and merge a pull request there: it may
    /// type, and that computer's Yardsort is new enough.
    pub may_publish: bool,
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
    /// Which repository it is, when that computer said where it fetches from: the same words
    /// [`project_repository`] gives for one of this computer's projects of that repository.
    pub repository: Option<String>,
    pub workspaces: Vec<OtherWorkspace>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherWorkspace {
    pub id: String,
    pub name: String,
    /// `local`, that project's own checkout, or `worktree`.
    pub kind: String,
    pub branch: Option<String>,
    pub archived: bool,
    /// Its folder should be there and is not.
    pub missing: bool,
    /// The tasks it was started from, on their forge.
    pub tasks: Vec<OtherTask>,
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
    /// Its terminal there, to watch; `None` once it has ended.
    pub terminal: Option<String>,
    /// That computer can resume it: it has ended, and its agent can take it up again.
    pub resumable: bool,
}

/// A task a workspace on another computer was started from: what to call it, and its page.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherTask {
    pub key: String,
    pub title: String,
    pub url: String,
}

/// An agent another computer can start.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherHarness {
    pub id: String,
    pub name: String,
    /// Models to suggest for it; any other may be typed.
    pub models: Vec<String>,
    /// The efforts it takes; none when it has no such thing.
    pub efforts: Vec<String>,
}

/// A new workspace on another computer, as its composer here says it.
#[derive(Debug, Clone, serde::Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherNewWorkspace {
    pub project_id: String,
    pub prompt: String,
    pub harness: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The branch the new one starts from. Without it, that project's default.
    pub base_branch: Option<String>,
    /// A branch to open as it is, with no new one made.
    pub existing_branch: Option<String>,
}

/// The branches a new workspace in a project on another computer can start from or open.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherBranches {
    pub branches: Vec<String>,
    pub default: Option<String>,
    /// Those a worktree there has out, which cannot be opened in another.
    pub checked_out: Vec<String>,
    /// That project has more branches than are listed.
    pub more: bool,
}

/// A project another computer cloned and added at this one's asking.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherCloned {
    pub project_id: String,
    pub name: String,
}

/// Which repository one of this computer's projects is, for offering the same one elsewhere.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalRepository {
    /// What an [`OtherProject::repository`] of the same repository says. `None` when the project
    /// has no `origin`, or one that is a folder.
    pub key: Option<String>,
    /// Whether another computer could clone it: its `origin` is an address, not a folder.
    pub clonable: bool,
}

/// Where a repository another device has this computer clone goes.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsDir {
    /// What the settings say, when they say.
    pub set: Option<String>,
    /// The folder used: the setting, or `Projects` in this user's home.
    pub used: Option<String>,
}

/// An agent started on another computer: where it runs, to watch it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherStarted {
    pub workspace_id: String,
    pub terminal: String,
    pub harness: String,
}

impl From<remote_proto::Started> for OtherStarted {
    fn from(started: remote_proto::Started) -> Self {
        Self {
            workspace_id: started.workspace.to_string(),
            terminal: started.terminal.to_string(),
            harness: started.harness,
        }
    }
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

fn view(host_id: &str, name: String, view: HostView, may_type: bool) -> OtherHostView {
    let Some(shown) = view.shown else {
        return OtherHostView {
            host_id: host_id.to_owned(),
            name,
            link: link(&view.link),
            may_type,
            may_start: false,
            may_resume: false,
            may_manage: false,
            may_look: false,
            may_publish: false,
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
            repository: p
                .remote
                .as_deref()
                .and_then(yardsort_core::remote::start::repository_key),
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
                    .as_ref()
                    .map_or("ended", |t| activity_name(t.activity))
                    .to_owned(),
                started_at: s.started_at as f64,
                terminal: s.terminal.map(|t| t.id.to_string()),
                resumable: s.resumable,
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
            kind: match workspace.kind {
                WorkspaceKind::Local => "local",
                WorkspaceKind::Worktree => "worktree",
            }
            .to_owned(),
            branch: workspace.branch.map(|b| b.label),
            archived: workspace.archived,
            missing: workspace.missing,
            tasks: workspace
                .tasks
                .into_iter()
                .map(|task| OtherTask {
                    key: task.key,
                    title: task.title,
                    url: task.url,
                })
                .collect(),
            sessions,
            terminals,
        };
        let project = workspace.project.to_string();
        match projects.iter_mut().find(|p| p.id == project) {
            Some(project) => project.workspaces.push(entry),
            None => projects.push(OtherProject {
                id: project,
                name: "Project".into(),
                repository: None,
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
    let answers = |op: &str| may_type && shown.host.capabilities.iter().any(|c| c == op);
    OtherHostView {
        host_id: host_id.to_owned(),
        name: shown.host.name.clone(),
        may_start: answers(computer::start::START),
        may_resume: answers(computer::start::RESUME),
        may_manage: answers(computer::start::MANAGE),
        may_look: answers(computer::look::LOOK),
        may_publish: answers(computer::publish::PUBLISH),
        link: link(&view.link),
        may_type,
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
    let grant = hosts
        .into_iter()
        .find(|p| p.grant.host.as_str() == host_id)
        .map(|p| p.grant);
    let may_type = grant
        .as_ref()
        .is_some_and(|g| g.scopes.contains(&remote_proto::Scope::TerminalControl));
    let name = grant.map(|g| g.host_name).unwrap_or_default();
    Ok(Some(view(&host_id, name, found, may_type)))
}

/// The agents another computer can start, its default first.
#[tauri::command]
#[specta::specta]
pub async fn other_host_harnesses(app: AppHandle, host_id: String) -> IpcResult<Vec<OtherHarness>> {
    let harnesses = computer::start::harnesses(&home(&app), &host_id)
        .await
        .map_err(ipc)?;
    Ok(harnesses
        .into_iter()
        .map(|h| OtherHarness {
            id: h.id,
            name: h.name,
            models: h.models,
            efforts: h.efforts,
        })
        .collect())
}

/// A new workspace in one of another computer's projects, with an agent given its first message.
/// That computer makes it with its own settings, as its window would. A name, a model, an effort
/// or a branch is sent only to a computer that reads them ([`OtherHostView::may_manage`]).
#[tauri::command]
#[specta::specta]
pub async fn other_host_start_workspace(
    app: AppHandle,
    host_id: String,
    request: OtherNewWorkspace,
) -> IpcResult<OtherStarted> {
    computer::start::workspace(
        &home(&app),
        &host_id,
        &request.project_id,
        &request.prompt,
        computer::start::Said {
            harness: request.harness,
            name: request.name,
            model: request.model,
            effort: request.effort,
            base_branch: request.base_branch,
            existing_branch: request.existing_branch,
        },
    )
    .await
    .map(OtherStarted::from)
    .map_err(ipc)
}

/// The branches a new workspace in one of another computer's projects can start from or open.
#[tauri::command]
#[specta::specta]
pub async fn other_host_branches(
    app: AppHandle,
    host_id: String,
    project_id: String,
) -> IpcResult<OtherBranches> {
    let listed = computer::start::branches(&home(&app), &host_id, &project_id)
        .await
        .map_err(ipc)?;
    Ok(OtherBranches {
        branches: listed.branches,
        default: listed.default,
        checked_out: listed.checked_out,
        more: listed.more,
    })
}

/// Where one of this computer's projects is fetched from, as another computer may be told it:
/// its `origin`, when that is an address and not a folder, with no credentials in it.
fn local_remote(app: &AppHandle, project_id: &str) -> IpcResult<Option<String>> {
    let state = app.state::<AppState>();
    let project = state
        .store
        .project(project_id)?
        .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))?;
    let git = yardsort_core::git::Git::new(&state.env())?;
    Ok(git
        .remote_url_as_set(std::path::Path::new(&project.root_path), "origin")
        .as_deref()
        .and_then(yardsort_core::remote::start::shareable_remote))
}

/// Which repository one of this computer's projects is, read from its `origin`.
#[tauri::command]
#[specta::specta]
pub async fn project_repository(app: AppHandle, project_id: String) -> IpcResult<LocalRepository> {
    let remote = tauri::async_runtime::spawn_blocking(move || local_remote(&app, &project_id))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))??;
    Ok(LocalRepository {
        key: remote
            .as_deref()
            .and_then(yardsort_core::remote::start::repository_key),
        clonable: remote.is_some(),
    })
}

/// Have another computer clone the repository of one of this computer's projects into its own
/// projects folder and add it there. Only the repository's address goes over, with no
/// credentials in it; that computer clones with its own.
#[tauri::command]
#[specta::specta]
pub async fn other_host_clone(
    app: AppHandle,
    host_id: String,
    project_id: String,
) -> IpcResult<OtherCloned> {
    let reader = app.clone();
    let remote = tauri::async_runtime::spawn_blocking(move || local_remote(&reader, &project_id))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))??
        .ok_or_else(|| {
            IpcError::new(
                "no_remote",
                "This project has no remote another computer could clone it from. Add it on \
                 that computer instead.",
            )
        })?;
    let cloned = computer::start::clone(&home(&app), &host_id, &remote)
        .await
        .map_err(ipc)?;
    Ok(OtherCloned {
        project_id: cloned.project.to_string(),
        name: cloned.name,
    })
}

/// That computer's shell in one of its workspaces.
#[tauri::command]
#[specta::specta]
pub async fn other_host_start_shell(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
) -> IpcResult<OtherStarted> {
    computer::start::shell(&home(&app), &host_id, &workspace_id)
        .await
        .map(OtherStarted::from)
        .map_err(ipc)
}

/// Change what a workspace on another computer is called. Its folder and branch keep their names.
#[tauri::command]
#[specta::specta]
pub async fn other_host_rename_workspace(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    name: String,
) -> IpcResult<()> {
    computer::start::rename(&home(&app), &host_id, &workspace_id, &name)
        .await
        .map_err(ipc)
}

/// Archive a workspace on another computer, or with `remove` delete it. Both keep its branch.
/// Fails with `remote_uncommitted` unless `force` when it has uncommitted work there.
#[tauri::command]
#[specta::specta]
pub async fn other_host_remove_workspace(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    remove: bool,
    force: bool,
) -> IpcResult<()> {
    computer::start::remove(&home(&app), &host_id, &workspace_id, remove, force)
        .await
        .map_err(ipc)
}

/// What changed in a workspace on another computer, as the local panel is given a local one's.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherChanges {
    pub changes: yardsort_core::changes::ChangeSet,
    /// That computer listed fewer files than changed.
    pub more: bool,
}

/// One folder of a workspace's file tree on another computer.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherFiles {
    pub entries: Vec<crate::changes::files::FileEntry>,
    /// That folder holds more than is listed.
    pub more: bool,
}

fn change_there(file: remote_proto::ChangedFile) -> yardsort_core::changes::FileChange {
    use remote_proto::ChangeKind as There;
    use yardsort_core::changes::ChangeKind as Here;
    yardsort_core::changes::FileChange {
        path: file.path,
        old_path: file.old_path,
        kind: match file.kind {
            There::Added => Here::Added,
            There::Deleted => Here::Deleted,
            There::Renamed => Here::Renamed,
            There::Untracked => Here::Untracked,
            There::Conflicted => Here::Conflicted,
            // A kind from a newer computer: something about the file changed.
            There::Modified | There::Unknown => Here::Modified,
        },
        additions: file.additions,
        deletions: file.deletions,
    }
}

/// What a file read from another computer is, as the viewer takes a local one.
fn content_there(file: remote_client::channel::WholeFile) -> yardsort_core::changes::Content {
    use remote_proto::FileState;
    use yardsort_core::changes::Content;
    match file.state {
        FileState::Text => match String::from_utf8(file.bytes) {
            Ok(text) => Content::Text { text },
            Err(_) => Content::Binary,
        },
        FileState::Absent => Content::Absent,
        FileState::Binary => Content::Binary,
        FileState::TooLarge => Content::TooLarge {
            bytes: u32::try_from(file.size).unwrap_or(u32::MAX),
        },
        FileState::NotShown | FileState::Unknown => Content::NotEditable {
            reason: "That computer does not show this kind of file.".into(),
        },
    }
}

/// What changed in one of another computer's workspaces: uncommitted, and on its branch.
#[tauri::command]
#[specta::specta]
pub async fn other_host_changes(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
) -> IpcResult<OtherChanges> {
    let listed = computer::look::changes(&home(&app), &host_id, &workspace_id)
        .await
        .map_err(ipc)?;
    Ok(OtherChanges {
        changes: yardsort_core::changes::ChangeSet {
            uncommitted: listed.uncommitted.into_iter().map(change_there).collect(),
            committed: listed.committed.into_iter().map(change_there).collect(),
            base: listed.base,
        },
        more: listed.more,
    })
}

/// One folder of the file tree of a workspace on another computer (`dir` is inside it; empty for
/// its root). What Git ignores there is not listed.
#[tauri::command]
#[specta::specta]
pub async fn other_host_files(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    dir: String,
) -> IpcResult<OtherFiles> {
    let listed = computer::look::files(&home(&app), &host_id, &workspace_id, &dir)
        .await
        .map_err(ipc)?;
    Ok(OtherFiles {
        entries: listed
            .entries
            .into_iter()
            .map(|entry| crate::changes::files::FileEntry {
                path: if dir.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{dir}/{}", entry.name)
                },
                name: entry.name,
                is_dir: entry.dir,
                ignored: false,
            })
            .collect(),
        more: listed.more,
    })
}

/// Both sides of a changed file in a workspace on another computer.
#[tauri::command]
#[specta::specta]
pub async fn other_host_diff(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    path: String,
    old_path: Option<String>,
    scope: yardsort_core::changes::Scope,
) -> IpcResult<yardsort_core::changes::FileDiff> {
    use remote_proto::FileOf;
    use yardsort_core::changes::Scope;
    let (old, new) = match scope {
        Scope::Uncommitted => (FileOf::UncommittedOld, FileOf::Working),
        Scope::Committed => (FileOf::CommittedOld, FileOf::CommittedNew),
    };
    let home = home(&app);
    let side = |of| {
        computer::look::file(
            &home,
            &host_id,
            &workspace_id,
            &path,
            old_path.as_deref(),
            of,
        )
    };
    let (old, new) = tokio::try_join!(side(old), side(new)).map_err(ipc)?;
    Ok(yardsort_core::changes::FileDiff {
        old: content_there(old),
        new: content_there(new),
    })
}

/// A file of a workspace on another computer, as it is on its disk now.
#[tauri::command]
#[specta::specta]
pub async fn other_host_file(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    path: String,
) -> IpcResult<yardsort_core::changes::Content> {
    computer::look::file(
        &home(&app),
        &host_id,
        &workspace_id,
        &path,
        None,
        remote_proto::FileOf::Working,
    )
    .await
    .map(content_there)
    .map_err(ipc)
}

/// The pull request of a workspace's branch on another computer.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherPullRequest {
    pub number: u32,
    /// Its page on the forge, when that is an `https` address.
    pub url: Option<String>,
    pub title: String,
    /// `open`, `merged`, `closed` or `unknown`.
    pub state: String,
    pub draft: bool,
    /// `none`, `running`, `passing`, `failing` or `unknown`.
    pub checks: String,
    /// The commit at its head, to name when merging it.
    pub head_oid: Option<String>,
    pub conflicting: bool,
}

/// Where a workspace on another computer stands with its remote.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherPublishState {
    pub branch: Option<String>,
    pub base: Option<String>,
    /// Who commits there would be by.
    pub identity: Option<String>,
    pub remote: bool,
    pub pushed: bool,
    pub ahead: u32,
    pub behind: u32,
    /// The subjects of the commits its remote does not have, newest first.
    pub unpushed: Vec<String>,
    /// Its pull request, when that computer was asked to ask the forge and there is one.
    pub pull_request: Option<OtherPullRequest>,
    pub can_open: bool,
    /// That computer can open and merge a pull request without a browser.
    pub gh: bool,
    pub problem: Option<String>,
}

impl From<remote_proto::publish::PublishState> for OtherPublishState {
    fn from(state: remote_proto::publish::PublishState) -> Self {
        let word = |value: serde_json::Result<serde_json::Value>| {
            value
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".into())
        };
        Self {
            branch: state.branch,
            base: state.base,
            identity: state.identity,
            remote: state.remote,
            pushed: state.pushed,
            ahead: state.ahead,
            behind: state.behind,
            unpushed: state.unpushed,
            pull_request: state.pull_request.map(|pr| OtherPullRequest {
                number: pr.number,
                // Its address came from that computer: only a web page is ever opened.
                url: Some(pr.url).filter(|url| url.starts_with("https://")),
                title: pr.title,
                state: word(serde_json::to_value(pr.state)),
                draft: pr.draft,
                checks: word(serde_json::to_value(pr.checks)),
                head_oid: pr.head_oid,
                conflicting: pr.conflicting,
            }),
            can_open: state.can_open,
            gh: state.gh,
            problem: state.problem,
        }
    }
}

/// A pull request another computer opened, or the forge's form to finish in a browser.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherPullRequestOpened {
    pub url: String,
    /// That computer opened it. When false the address is the forge's own form.
    pub created: bool,
}

/// How to merge a pull request on another computer.
#[derive(Debug, Clone, Copy, serde::Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OtherMergeMethod {
    Squash,
    Merge,
    Rebase,
}

/// Where a workspace on another computer stands with its remote. With `forge`, that computer
/// asks the forge about its pull request, which takes it a network round trip.
#[tauri::command]
#[specta::specta]
pub async fn other_host_publish_state(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    forge: bool,
) -> IpcResult<OtherPublishState> {
    computer::publish::state(&home(&app), &host_id, &workspace_id, forge)
        .await
        .map(OtherPublishState::from)
        .map_err(ipc)
}

/// Commit everything changed in a workspace on another computer, with that computer's own Git
/// identity. Only ever at the person's press.
#[tauri::command]
#[specta::specta]
pub async fn other_host_commit(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    message: String,
) -> IpcResult<OtherPublishState> {
    computer::publish::commit(&home(&app), &host_id, &workspace_id, &message)
        .await
        .map(OtherPublishState::from)
        .map_err(ipc)
}

/// Push a workspace's branch on another computer to its project's remote, with that computer's
/// own credentials. Only ever at the person's press.
#[tauri::command]
#[specta::specta]
pub async fn other_host_push(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
) -> IpcResult<OtherPublishState> {
    computer::publish::push(&home(&app), &host_id, &workspace_id)
        .await
        .map(OtherPublishState::from)
        .map_err(ipc)
}

/// Open a pull request for a workspace's branch on another computer, with that computer's own
/// forge sign-in, pushing first when it needs it.
#[tauri::command]
#[specta::specta]
pub async fn other_host_open_pull_request(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    title: String,
    body: String,
    draft: bool,
) -> IpcResult<OtherPullRequestOpened> {
    let opened = computer::publish::open_pull_request(
        &home(&app),
        &host_id,
        &workspace_id,
        &title,
        &body,
        draft,
    )
    .await
    .map_err(ipc)?;
    if !opened.url.starts_with("https://") {
        return Err(IpcError::new(
            "remote_refused",
            "That computer answered with an address this computer will not open.",
        ));
    }
    Ok(OtherPullRequestOpened {
        url: opened.url,
        created: opened.created,
    })
}

/// Merge the pull request of a workspace on another computer, if it is still open and at the
/// commit the person saw.
#[tauri::command]
#[specta::specta]
pub async fn other_host_merge_pull_request(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    number: u32,
    head_oid: String,
    method: OtherMergeMethod,
) -> IpcResult<()> {
    let method = match method {
        OtherMergeMethod::Squash => remote_proto::publish::MergeMethod::Squash,
        OtherMergeMethod::Merge => remote_proto::publish::MergeMethod::Merge,
        OtherMergeMethod::Rebase => remote_proto::publish::MergeMethod::Rebase,
    };
    computer::publish::merge_pull_request(
        &home(&app),
        &host_id,
        &workspace_id,
        number,
        &head_oid,
        method,
    )
    .await
    .map_err(ipc)
}

fn projects_dir(state: &AppState) -> ProjectsDir {
    let settings = state.settings.get().remote;
    ProjectsDir {
        used: yardsort_core::remote::start::projects_dir(&settings, &state.env())
            .map(|dir| dir.to_string_lossy().into_owned()),
        set: settings.projects_dir,
    }
}

/// Where a repository another device has this computer clone goes.
#[tauri::command]
#[specta::specta]
pub async fn remote_projects_dir(app: AppHandle) -> IpcResult<ProjectsDir> {
    Ok(projects_dir(app.state::<AppState>().inner()))
}

/// Choose where such a clone goes, or with nothing go back to `Projects` in the home folder. The
/// folder is made when the first clone needs it, not here.
#[tauri::command]
#[specta::specta]
pub async fn remote_set_projects_dir(
    app: AppHandle,
    dir: Option<String>,
) -> IpcResult<ProjectsDir> {
    let state = app.state::<AppState>();
    let dir = dir.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty());
    if let Some(dir) = &dir {
        let absolute = dir.starts_with('~') || std::path::Path::new(dir).is_absolute();
        if !absolute || dir.chars().any(char::is_control) {
            return Err(IpcError::new(
                "invalid_folder",
                "Give the whole path of a folder, or one that starts with ~ for your home.",
            ));
        }
    }
    state
        .settings
        .update(|settings| settings.remote.projects_dir = dir)
        .map_err(|e| IpcError::new("settings", e.to_string()))?;
    Ok(projects_dir(state.inner()))
}

/// Another agent in one of another computer's workspaces, with a first message or none.
#[tauri::command]
#[specta::specta]
pub async fn other_host_start_session(
    app: AppHandle,
    host_id: String,
    workspace_id: String,
    harness: Option<String>,
    prompt: Option<String>,
) -> IpcResult<OtherStarted> {
    computer::start::session(
        &home(&app),
        &host_id,
        &workspace_id,
        harness.as_deref(),
        prompt.as_deref(),
    )
    .await
    .map(OtherStarted::from)
    .map_err(ipc)
}

/// One of another computer's ended conversations again, with a message to go on with or none.
#[tauri::command]
#[specta::specta]
pub async fn other_host_resume(
    app: AppHandle,
    host_id: String,
    session_id: String,
    prompt: Option<String>,
) -> IpcResult<OtherStarted> {
    computer::start::resume(&home(&app), &host_id, &session_id, prompt.as_deref())
        .await
        .map(OtherStarted::from)
        .map_err(ipc)
}
