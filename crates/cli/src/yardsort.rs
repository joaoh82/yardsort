//! Opening the same things the app opens: the database, the settings, the daemon.

use std::path::PathBuf;
use std::sync::Arc;

use yardsort_core::daemon;
use yardsort_core::env::ShellEnv;
use yardsort_core::git::Git;
use yardsort_core::paths;
use yardsort_core::settings::{Settings, SettingsFile};
use yardsort_core::store::{ProjectRow, Store};

use crate::Failure;

/// Everything a command might need, opened when it is first asked for.
///
/// Resolving the environment runs the user's login shell and connecting to the daemon can start
/// a process, so neither happens for a command that only reads the database.
pub struct Yardsort {
    pub data_dir: PathBuf,
    pub store: Store,
    pub settings: Settings,
    env: std::cell::OnceCell<Arc<ShellEnv>>,
}

impl Yardsort {
    /// `--data-dir` wins, then `YARDSORT_DATA_DIR`, then the platform's own place.
    ///
    /// Deliberately refuses to create a database. If this resolved the wrong directory — the one
    /// risk of working the paths out without Tauri to ask — creating one would answer every
    /// question with a convincing, empty "no projects". An error naming the path is honest.
    pub fn open(data_dir: Option<PathBuf>) -> Result<Self, Failure> {
        let explicit = data_dir.is_some();
        let data_dir = match data_dir {
            Some(dir) => dir,
            None => paths::data_dir().ok_or_else(|| {
                Failure::new("Cannot work out where Yardsort keeps its data. Pass --data-dir.")
            })?,
        };
        let database = data_dir.join("yardsort.db");
        if !database.is_file() {
            return Err(Failure::new(format!(
                "No Yardsort database at {}.\n\
                 Open the app once to create one, or point at another profile with --data-dir.",
                database.display()
            )));
        }
        // A profile directory keeps its settings beside its database, exactly as the app does
        // when `YARDSORT_DATA_DIR` is set.
        let config_dir = if explicit {
            data_dir.clone()
        } else {
            paths::config_dir().unwrap_or_else(|| data_dir.clone())
        };
        let store = Store::open(&database)
            .map_err(|e| Failure::new(format!("Cannot open {}: {e}", database.display())))?;
        let settings = SettingsFile::load(config_dir.join("settings.toml")).get();
        Ok(Self {
            data_dir,
            store,
            settings,
            env: std::cell::OnceCell::new(),
        })
    }

    /// The environment programs are launched in. Blocking on first use: it runs the login shell.
    pub fn env(&self) -> &Arc<ShellEnv> {
        self.env.get_or_init(|| Arc::new(ShellEnv::resolve()))
    }

    pub fn git(&self) -> Result<Git, Failure> {
        Git::new(self.env()).map_err(|e| Failure::new(format!("git: {e}")))
    }

    /// Where worktrees go, by the same rules the app follows.
    pub fn worktree_root(&self) -> Result<PathBuf, Failure> {
        Ok(yardsort_core::workspaces::worktree_root(
            &self.settings.workspaces,
            self.env(),
        )?)
    }

    /// Attach to this profile's daemon, without starting one.
    ///
    /// For commands that only report: a `ys session list` that started a daemon would answer its
    /// own question, and would leave a process behind on a machine that had none.
    pub fn daemon(&self) -> Option<Arc<pty_ipc::DaemonClient>> {
        self.daemon_watching(Arc::new(|_| {}))
    }

    /// Take the exits the daemon kept while nobody was connected into the database, so what
    /// the records say and what happened agree. Cheap when there is nothing there, which is
    /// nearly always; done whenever a command is about to ask the daemon anything.
    pub fn drain_spool(&self) -> yardsort_core::activity::ImportReport {
        // What agents' hooks reported since anyone last looked comes in at the same time, and
        // so does what Grok wrote into its own session directories, when that is switched on.
        yardsort_core::activity::import_inbox(&self.store, &self.data_dir);
        if self.settings.activity.capture_grok {
            let home = yardsort_core::activity::grok::grok_home(&|name| {
                std::env::var(name)
                    .ok()
                    .or_else(|| self.env().vars.get(name).cloned())
            });
            if let Some(home) = home {
                yardsort_core::activity::grok::import(
                    &self.store,
                    &home,
                    &mut yardsort_core::activity::grok::Cursors::new(),
                );
            }
        }
        yardsort_core::activity::import_spool(&self.store, &self.data_dir)
    }

    /// The same, but hearing the daemon's events — which is how `attach` learns that the session
    /// it is showing has exited.
    pub fn daemon_watching(
        &self,
        events: pty_host::EventSink,
    ) -> Option<Arc<pty_ipc::DaemonClient>> {
        self.drain_spool();
        daemon::connect_existing(&self.data_dir, events).map(Arc::new)
    }

    /// Attach to this profile's daemon, starting one if there is none.
    pub fn daemon_or_start(&self) -> daemon::Connected {
        self.drain_spool();
        daemon::connect(&self.data_dir, Arc::new(|_| {}))
    }

    /// Find a project by id, or by name if that is unambiguous.
    pub fn project(&self, wanted: &str) -> Result<ProjectRow, Failure> {
        let projects = self.store.projects()?;
        if let Some(exact) = projects.iter().find(|p| p.id == wanted) {
            return Ok(exact.clone());
        }
        let matches: Vec<_> = projects
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(wanted))
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(Failure::new(format!(
                "No project called {wanted:?}.{}",
                match projects.as_slice() {
                    [] => " There are no projects yet — add one in the app.".to_owned(),
                    some => format!(
                        " Known: {}.",
                        some.iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            ))),
            many => Err(Failure::new(format!(
                "{} projects are called {wanted:?}. Use the id instead: {}.",
                many.len(),
                many.iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }
}
