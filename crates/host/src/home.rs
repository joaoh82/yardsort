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
use yardsort_core::remote::serve::Context;
use yardsort_core::remote::start::{Starter, Starting};
use yardsort_core::remote::HostIdentity;
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

    pub fn asking(&self) -> tokio::sync::broadcast::Receiver<Request> {
        self.asked.subscribe()
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
        let Some(git) = self.git.get_or_init(|| Git::new(self.env()).ok()) else {
            return;
        };
        let Ok(Some(row)) = self.store.remote_host() else {
            return;
        };
        let Ok(id) = remote_proto::HostId::new(row.host_id) else {
            return;
        };
        f(&Context {
            store: &self.store,
            git,
            terminals: terminals.host.as_ref(),
            host: &HostIdentity { id, name: row.name },
            interactive: terminals.host.shares_control(),
            starting: Some(Arc::clone(&self.starting) as Arc<dyn Starting>),
        });
    }

    fn asked(&self, request: Request) {
        // Nobody waiting is fine: the device's request waits at the server for the next `pair`.
        let _ = self.asked.send(request);
    }
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
        project: &str,
        prompt: &str,
        harness: &str,
    ) -> IpcResult<(String, SessionInfo)> {
        self.with(|starter| starter.workspace(project, prompt, harness))
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
