use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use pty_host::TerminalHost;
use tauri::{AppHandle, Manager};

use crate::assist::Assist;
use crate::env::ShellEnv;
use crate::error::{IpcError, IpcResult};
use crate::settings::SettingsFile;
use crate::store::Store;

/// Everything the commands share. Managed by Tauri, created in `setup`.
pub struct AppState {
    /// This profile's data directory: the database, the daemon's socket and log, and the exit
    /// spool all sit in it.
    pub data_dir: PathBuf,
    /// The PTY host: the daemon over a local socket, or an in-process host under
    /// `YARDSORT_NO_DAEMON`. Nothing here knows or cares which.
    pub host: Arc<dyn TerminalHost>,
    /// The same object as `host` when there is a daemon, for what only a daemon can be asked.
    pub daemon_client: Option<Arc<pty_ipc::DaemonClient>>,
    pub daemon: crate::daemon::DaemonStatus,
    pub store: Store,
    pub settings: SettingsFile,
    /// Assist: the API key store and the answers already given. See `crate::assist`.
    pub assist: Assist,
    /// What the forge last said about each project's pull requests. See `crate::publish`.
    pub forge: crate::publish::Forge,
    /// Held while catching up with git, so overlapping project listings reconcile one at a time.
    pub reconciling: Mutex<()>,
    /// The file watcher of the workspace on screen, if any.
    pub watcher: Mutex<Option<crate::changes::watch::WorkspaceWatcher>>,
    env: Mutex<Option<Arc<ShellEnv>>>,
}

impl AppState {
    pub fn new(
        data_dir: PathBuf,
        connected: crate::daemon::Connected,
        store: Store,
        settings: SettingsFile,
    ) -> Self {
        Self {
            data_dir,
            host: connected.host,
            daemon_client: connected.client,
            daemon: connected.status,
            store,
            settings,
            assist: Assist::default(),
            forge: crate::publish::Forge::default(),
            reconciling: Mutex::new(()),
            watcher: Mutex::new(None),
            env: Mutex::new(None),
        }
    }

    /// The environment to launch programs in. The first caller resolves it (which can take a
    /// moment: it runs the user's shell startup files) while the others wait on the lock.
    /// Blocking — call from a blocking context.
    pub fn env(&self) -> Arc<ShellEnv> {
        let mut slot = self.env.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slot.get_or_insert_with(|| Arc::new(ShellEnv::resolve())))
    }

    /// Where worktrees go. See [`yardsort_core::workspaces::worktree_root`].
    pub fn worktree_root(&self) -> IpcResult<PathBuf> {
        crate::workspaces::worktree_root(&self.settings.get().workspaces, &self.env())
    }

    pub fn default_worktree_root(&self) -> IpcResult<PathBuf> {
        crate::workspaces::default_worktree_root(&self.env())
    }

    /// Re-run the login shell, e.g. after the user installed a harness.
    pub fn reload_env(&self) -> Arc<ShellEnv> {
        let mut slot = self.env.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = Arc::new(ShellEnv::resolve());
        *slot = Some(Arc::clone(&fresh));
        fresh
    }
}

/// Run `f` off the async runtime. Nearly every command blocks — on SQLite, on git, on the login
/// shell — and must not stall the threads that serve the webview.
pub async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> IpcResult<T> + Send + 'static,
) -> IpcResult<T> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
}
