//! This program as a host: its records and terminals, and the operator's `pair` to tell of a
//! device asking.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use pty_host::SessionInfo;

use remote_host::account::{self, Failure, Home, Remote, Request};
use yardsort_core::daemon::Connected;
use yardsort_core::env::ShellEnv;
use yardsort_core::error::IpcResult;
use yardsort_core::git::Git;
use yardsort_core::remote::host::{CredentialStore, FileCredentials};
use yardsort_core::remote::pull::polled::{self, Polled};
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::start::{Starter, Starting};
use yardsort_core::remote::HostIdentity;
use yardsort_core::remote::PullRequests;
use yardsort_core::runtime::InboxWatcher;
use yardsort_core::settings::SettingsFile;
use yardsort_core::store::Store;

use crate::profile::Profile;

pub struct HostHome {
    pub profile: Profile,
    remote: Remote,
    pub store: Store,
    pub settings: SettingsFile,
    /// The daemon, while serving. Signing in needs none.
    pub terminals: Option<Connected>,
    env: OnceLock<Arc<ShellEnv>>,
    git: OnceLock<Option<Git>>,
    /// Devices asking to be let in, for whichever `pair` is waiting.
    asked: tokio::sync::broadcast::Sender<Request>,
    /// The activity inbox's watcher, while serving.
    pub watcher: OnceLock<InboxWatcher>,
    /// What a paired device that may type starts agents with.
    starting: Arc<HostStarting>,
    /// The dashboard, while it is served (`dashboard.rs`).
    pub dashboard: std::sync::Mutex<Option<host_dashboard::Dashboard>>,
    /// The projects' pull requests, read here when a device asks about them.
    pulls: Arc<yardsort_core::remote::pull::polled::Polled>,
}

impl HostHome {
    pub fn open(profile: Profile, terminals: Option<Connected>) -> Result<Arc<Self>, String> {
        let database = profile.database();
        let store = Store::open(&database)
            .map_err(|e| format!("Cannot open {}: {e}", database.display()))?;
        let settings = SettingsFile::load(profile.settings());
        Ok(Arc::new_cyclic(|home| Self {
            profile,
            remote: Remote::default(),
            store,
            settings,
            terminals,
            env: OnceLock::new(),
            git: OnceLock::new(),
            asked: tokio::sync::broadcast::channel(8).0,
            watcher: OnceLock::new(),
            starting: Arc::new(HostStarting(home.clone())),
            dashboard: std::sync::Mutex::new(None),
            pulls: Arc::new(pulls(home.clone())),
        }))
    }

    /// Something was launched or exited: drain the inbox now, and follow a harness whose files are
    /// read rather than delivered (Grok) from its first line to its last.
    pub fn look_again(&self) {
        if let Some(watcher) = self.watcher.get() {
            watcher.look_again();
        }
    }

    /// The environment programs are launched in. Blocking on first use: it runs the login shell.
    pub fn env(&self) -> &Arc<ShellEnv> {
        self.env.get_or_init(|| Arc::new(ShellEnv::resolve()))
    }

    /// Git, as programs here run it. Blocking on first use: it resolves the environment.
    pub fn git(&self) -> Option<&Git> {
        self.git.get_or_init(|| Git::new(self.env()).ok()).as_ref()
    }

    pub fn asking(&self) -> tokio::sync::broadcast::Receiver<Request> {
        self.asked.subscribe()
    }

    /// Clone a repository into the folder for clones and add it as a project, as a paired
    /// device's asking does: its id and its name.
    pub fn clone_project(&self, remote: &str) -> IpcResult<(String, String)> {
        self.starting.clone_project(remote, None)
    }
}

impl Home for HostHome {
    fn remote(&self) -> &Remote {
        &self.remote
    }

    fn store(&self) -> &Store {
        &self.store
    }

    fn settings(&self) -> &SettingsFile {
        &self.settings
    }

    fn credentials(&self) -> account::Result<Box<dyn CredentialStore>> {
        Ok(Box::new(
            FileCredentials::new(&self.profile.credentials_dir)
                .map_err(|e| Failure::new("remote_credentials", e))?,
        ))
    }

    fn serve(&self, f: &mut dyn FnMut(&Context<'_>)) {
        let Some(terminals) = &self.terminals else {
            return;
        };
        let Some(git) = self.git() else {
            return;
        };
        let Ok(Some(row)) = self.store.remote_host() else {
            return;
        };
        let Ok(id) = remote_proto::HostId::new(row.host_id) else {
            return;
        };
        let env = self.env();
        let conversations = yardsort_core::remote::Conversations {
            sources: yardsort_core::usage::logs::Sources::from_env(&|name| {
                env.vars.get(name).cloned()
            }),
            data_dir: self.profile.data_dir.clone(),
        };
        f(&Context {
            store: &self.store,
            git,
            terminals: terminals.host.as_ref(),
            host: &HostIdentity { id, name: row.name },
            interactive: terminals.host.shares_control(),
            starting: Some(Arc::clone(&self.starting) as Arc<dyn Starting>),
            workflow: None,
            conversations: Some(&conversations),
            tests: None,
            pulls: Some(Arc::clone(&self.pulls) as Arc<dyn PullRequests>),
        });
    }

    fn asked(&self, request: Request) {
        // Nobody waiting is fine: the device's request waits at the server for the next `pair`.
        let _ = self.asked.send(request);
    }
}

/// Pull requests read with this computer's `gh` and its sign-in, in each project's checkout, as
/// the desktop app's window reads them.
fn pulls(home: Weak<HostHome>) -> Polled {
    Polled::new(Arc::new(move |project_id: &str, every_open: bool| {
        let home = home.upgrade()?;
        let git = home.git()?;
        let gh = yardsort_core::forge::Gh::find(home.env());
        polled::read(&home.store, git, gh.as_ref(), project_id, every_open)
    }))
}

/// A device's channel starting agents here, with this host's settings, worktree folder and
/// terminals, as `ys workspace new` uses them.
struct HostStarting(Weak<HostHome>);

impl HostStarting {
    fn with<T>(&self, f: impl FnOnce(&Starter<'_>) -> IpcResult<T>) -> IpcResult<T> {
        let gone = || yardsort_core::error::IpcError::new("host_stopping", "The host is stopping.");
        let home = self.0.upgrade().ok_or_else(gone)?;
        let terminals = home.terminals.as_ref().ok_or_else(gone)?;
        let env = home.env();
        let settings = home.settings.get();
        let git = Git::new(env)?;
        let worktree_root = yardsort_core::workspaces::worktree_root(&settings.workspaces, env)?;
        let done = f(&Starter {
            store: &home.store,
            git: &git,
            terminals: terminals.host.as_ref(),
            env,
            settings: &settings,
            worktree_root: &worktree_root,
            data_dir: &home.profile.data_dir,
        });
        if done.is_ok() {
            home.look_again();
        }
        done
    }
}

impl Starting for HostStarting {
    fn harnesses(&self) -> Vec<remote_proto::HarnessSummary> {
        self.with(|starter| Ok(starter.harnesses()))
            .unwrap_or_default()
    }

    fn workspace(
        &self,
        new: &yardsort_core::remote::start::NewWorkspace,
    ) -> IpcResult<(String, SessionInfo)> {
        self.with(|starter| starter.workspace(new))
    }

    fn shell(&self, workspace: &str) -> IpcResult<SessionInfo> {
        self.with(|starter| starter.shell(workspace))
    }

    fn branches(&self, project: &str) -> IpcResult<remote_proto::ProjectBranches> {
        self.with(|starter| starter.branches(project))
    }

    fn clone_project(&self, remote: &str, name: Option<&str>) -> IpcResult<(String, String)> {
        self.with(|starter| starter.clone_project(remote, name))
    }

    fn rename(&self, workspace: &str, name: &str) -> IpcResult<()> {
        self.with(|starter| starter.rename(workspace, name))
    }

    fn archive(&self, workspace: &str, force: bool) -> IpcResult<()> {
        self.with(|starter| starter.archive(workspace, force))
    }

    fn delete(&self, workspace: &str, force: bool) -> IpcResult<()> {
        self.with(|starter| starter.delete(workspace, force))
    }

    fn changes(&self, workspace: &str) -> IpcResult<yardsort_core::changes::ChangeSet> {
        self.with(|starter| starter.changes(workspace))
    }

    fn files(
        &self,
        workspace: &str,
        dir: &str,
    ) -> IpcResult<Vec<yardsort_core::file_tree::FileEntry>> {
        self.with(|starter| starter.files(workspace, dir))
    }

    fn content(
        &self,
        workspace: &str,
        path: &str,
        old_path: Option<&str>,
        of: remote_proto::FileOf,
    ) -> IpcResult<yardsort_core::changes::Content> {
        self.with(|starter| starter.content(workspace, path, old_path, of))
    }

    fn publish(
        &self,
        workspace: &str,
        forge: bool,
    ) -> IpcResult<remote_proto::publish::PublishState> {
        self.with(|starter| starter.publish(workspace, forge))
    }
    fn commit(
        &self,
        workspace: &str,
        message: &str,
    ) -> IpcResult<remote_proto::publish::PublishState> {
        self.with(|starter| starter.commit(workspace, message))
    }
    fn push(&self, workspace: &str) -> IpcResult<remote_proto::publish::PublishState> {
        self.with(|starter| starter.push(workspace))
    }
    fn open_pull_request(
        &self,
        workspace: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> IpcResult<remote_proto::publish::PullRequestOpened> {
        self.with(|starter| starter.open_pull_request(workspace, title, body, draft))
    }
    fn merge_pull_request(
        &self,
        workspace: &str,
        number: u32,
        head_oid: &str,
        method: remote_proto::publish::MergeMethod,
    ) -> IpcResult<()> {
        self.with(|starter| starter.merge_pull_request(workspace, number, head_oid, method))
    }

    fn session(
        &self,
        workspace: &str,
        harness: &str,
        prompt: Option<&str>,
    ) -> IpcResult<SessionInfo> {
        self.with(|starter| starter.session(workspace, harness, prompt))
    }

    fn resume(&self, session: &str, prompt: Option<&str>) -> IpcResult<SessionInfo> {
        self.with(|starter| starter.resume(session, prompt))
    }

    fn definitions(&self) -> Vec<yardsort_core::harness::HarnessDef> {
        self.0.upgrade().map_or_else(Vec::new, |home| {
            yardsort_core::remote::start::definitions(&home.settings.get().harnesses, home.env())
        })
    }

    fn data_dir(&self) -> PathBuf {
        self.0
            .upgrade()
            .map(|home| home.profile.data_dir.clone())
            .unwrap_or_default()
    }
}
