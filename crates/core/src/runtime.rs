//! What any program that keeps a profile's daemon company does to keep its records true: the app,
//! and `yardsort-host` on a computer with no screen.
//!
//! - At start, [`settle_at_start`]: what agents reported and what exited while nobody was
//!   listening, then the sessions that died with the previous run.
//! - On each exit the daemon reports, [`exited`].
//! - While it runs, [`watch_inbox`]: hooks write into the inbox whenever an agent does
//!   something, and this drains it as it fills, so the timeline moves while an agent works rather
//!   than when it exits.
//!
//! Each program says what to do with what moved, which only it knows: the app tells its window.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use pty_host::ExitInfo;

use crate::activity::{self, grok, ExitFacts, Via};
use crate::store::Store;

/// What [`settle_at_start`] leaves for its caller.
#[derive(Debug, Default)]
#[must_use = "pull requests agents were seen creating are tracked only if the caller does it"]
pub struct Settled {
    /// Pull requests agents were seen creating while nothing was running. Their reports are out
    /// of the inbox now, so no later drain hands them on again: the caller tracks them with
    /// [`track_seen`] once it can find git — at start the app has not yet read the login
    /// shell's environment that git is found by.
    pub pull_requests: Vec<crate::tracked::Seen>,
}

/// Settle the records before anything is served: exits and reports kept while nobody was
/// connected, then every session still marked running whose process is not in `alive`.
pub fn settle_at_start(
    store: &Store,
    data_dir: &Path,
    alive: &[String],
) -> Result<Settled, String> {
    // What agents reported while nothing was running.
    let inbox = activity::import_inbox(store, data_dir);
    if inbox.imported > 0 || inbox.unlinked > 0 || inbox.unreadable > 0 || inbox.dropped > 0 {
        eprintln!(
            "activity inbox: {} imported, {} duplicate, {} unlinked, {} unreadable, {} dropped",
            inbox.imported, inbox.duplicates, inbox.unlinked, inbox.unreadable, inbox.dropped
        );
    }
    // Exits the daemon kept meanwhile come first: a conversation that ended cleanly in the
    // meantime must keep its exit code rather than be counted as interrupted below.
    let imported = activity::import_spool(store, data_dir);
    if imported.imported > 0 || imported.unreadable > 0 || imported.dropped > 0 {
        eprintln!(
            "exit spool: {} imported, {} duplicate, {} unmatched, {} unreadable, {} dropped",
            imported.imported,
            imported.duplicates,
            imported.unmatched,
            imported.unreadable,
            imported.dropped
        );
    }
    // Rows still marked running whose process is *not* among these died with the previous
    // run; the rest are conversations that never stopped.
    store
        .end_interrupted_sessions(alive)
        .map_err(|e| format!("cannot tidy session records: {e}"))?;
    activity::end_interrupted(store, alive);
    activity::prune(store);
    Ok(Settled {
        pull_requests: inbox.pull_requests,
    })
}

/// The daemon says the process in terminal `pty_id` exited: settle its record first, so a client
/// reacting to the exit reads the truth. Then drain the inbox, since an agent's last hooks ran
/// just before it exited, which is the caller's to do (it knows whom to tell).
pub fn exited(store: &Store, data_dir: &Path, pty_id: &str, exit: &ExitInfo) {
    let _ = store.end_session_by_pty(pty_id, Some(i64::from(exit.code)));
    activity::record_exit(store, pty_id, &ExitFacts::from(exit), Via::Live);
    // The daemon kept this exit for us too; take it out of the spool now rather than find it
    // as a duplicate at the next start.
    activity::import_spool(store, data_dir);
}

/// What one drain found.
#[derive(Debug, Default)]
pub struct Drained {
    /// Workspaces with new events.
    pub workspaces: BTreeSet<String>,
    /// Entries left for later, and Grok runs still being followed: worth looking at again soon.
    pub pending: usize,
    /// The workspaces that began following a pull request an agent was seen creating.
    pub tracked: BTreeSet<String>,
}

/// [`crate::tracked::track_seen`], with `git` found only when there is something to track.
pub fn track_seen(
    store: &Store,
    seen: &[crate::tracked::Seen],
    git: impl FnOnce() -> Option<crate::git::Git>,
) -> BTreeSet<String> {
    if seen.is_empty() {
        return BTreeSet::new();
    }
    crate::tracked::track_seen(store, git().as_ref(), seen)
}

/// Take what the inbox holds into the store, and, given `grok_home`, what Yardsort's own Grok
/// launches wrote into their session directories. A pull request an agent was seen creating is
/// tracked with its workspace; `git`, asked for only then, tells the project's own apart.
pub fn drain(
    store: &Store,
    data_dir: &Path,
    grok_home: Option<&Path>,
    cursors: &mut grok::Cursors,
    git: impl FnOnce() -> Option<crate::git::Git>,
) -> Drained {
    let report = activity::import_inbox(store, data_dir);
    let mut seen = report.pull_requests;
    let mut drained = Drained {
        workspaces: report.workspaces,
        pending: report.deferred,
        tracked: BTreeSet::new(),
    };
    if let Some(home) = grok_home {
        let read = grok::import(store, home, cursors);
        drained.workspaces.extend(read.workspaces);
        drained.pending += read.watching;
        seen.extend(read.pull_requests);
    }
    drained.tracked = track_seen(store, &seen, git);
    drained
}

/// Where Grok keeps its sessions, by the environment a launch gets: this process's own, then the
/// login shell's.
pub fn grok_home(shell: &crate::env::ShellEnv) -> Option<PathBuf> {
    grok::grok_home(&|name| {
        std::env::var(name)
            .ok()
            .or_else(|| shell.vars.get(name).cloned())
    })
}

/// Wait this long after the last file lands before draining: a tool call is two files in
/// quick succession, and one drain covers both.
const INBOX_QUIET: Duration = Duration::from_millis(200);
/// While a drain has left something for later, look again this often. Codex writes a turn's
/// end to its session file within moments of announcing it; five seconds is patience, not
/// polling, and the core gives up on a turn after five minutes, which ends the looking.
const RETRY_DEFERRED: Duration = Duration::from_secs(5);

/// How long the watcher's thread waits for a signal before draining on its own: forever when
/// nothing was deferred and no session file is being followed, [`RETRY_DEFERRED`] otherwise.
fn wait_for(pending: usize) -> Option<Duration> {
    (pending > 0).then_some(RETRY_DEFERRED)
}

/// Watches until dropped.
pub struct InboxWatcher {
    _watcher: RecommendedWatcher,
    wake: mpsc::Sender<()>,
}

impl InboxWatcher {
    /// Drain now, and keep draining on the retry clock while anything stays deferred. Called on
    /// a launch, so a harness whose files are read rather than delivered (Grok) is followed from
    /// its first line, and on an exit, which is when its last lines land.
    pub fn look_again(&self) {
        let _ = self.wake.send(());
    }
}

/// Watch the inbox directory and call `drain` after each burst of files, and once at the start.
/// `drain` returns how much is pending ([`Drained::pending`]), which sets when it is called again
/// with nothing new. The directory is created first, since a watch needs something to watch, and
/// `drain` runs on a thread of its own.
pub fn watch_inbox(
    data_dir: &Path,
    mut drain: impl FnMut(&mut grok::Cursors) -> usize + Send + 'static,
) -> notify::Result<InboxWatcher> {
    let dir = activity::inbox_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let (tx, rx) = mpsc::channel::<()>();
    let wake = tx.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| brings_entries(&event.kind)) {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    std::thread::spawn(move || {
        let mut pending = 0;
        let mut cursors = grok::Cursors::new();
        // The first pass does not wait: a Grok run left going when the program last stopped has
        // a directory to read now, and nothing else would say so.
        let mut first = true;
        loop {
            let woken = if first {
                first = false;
                false
            } else {
                match wait_for(pending) {
                    Some(wait) => match rx.recv_timeout(wait) {
                        Ok(()) => true,
                        Err(RecvTimeoutError::Timeout) => false,
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                    None => match rx.recv() {
                        Ok(()) => true,
                        Err(_) => break,
                    },
                }
            };
            if woken {
                // Let the burst finish; whatever else arrives meanwhile is one drain.
                std::thread::sleep(INBOX_QUIET);
                while rx.try_recv().is_ok() {}
            }
            pending = drain(&mut cursors);
        }
    });
    Ok(InboxWatcher {
        _watcher: watcher,
        wake,
    })
}

/// Whether an event on the inbox directory can mean a new entry: a file created or renamed
/// into place. A drain *reads* the directory, which on Linux is an `Access` event of its own,
/// and a drain that woke the next drain would never let the worker sleep. Removals are the
/// drain's too.
fn brings_entries(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Create(_) | EventKind::Modify(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind, RenameMode};

    /// Claude opened a pull request elsewhere while nothing was running: the start takes its
    /// report out of the inbox and hands the link on, since no drain after will see it again.
    #[test]
    fn a_pull_request_seen_while_nothing_ran_is_tracked_from_the_start() {
        use crate::activity::inbox::{Inbox, InboxEntry, INBOX_VERSION};
        use crate::git::testing::git;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("parent");
        std::fs::create_dir_all(&root).unwrap();
        git().run(&root, &["init", "-q"]).unwrap();
        git()
            .run(
                &root,
                &["remote", "add", "origin", "git@github.com:o/parent.git"],
            )
            .unwrap();
        let store = Store::in_memory();
        let project = store
            .add_project("parent", &root.to_string_lossy())
            .unwrap();
        let ws = store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap();
        let data_dir = dir.path().join("profile");
        let created = InboxEntry {
            version: INBOX_VERSION,
            id: "e-pr".into(),
            at_ms: 1_790_000_000_000,
            producer: activity::claude::PRODUCER.into(),
            method: activity::claude::METHOD.into(),
            fidelity: activity::claude::FIDELITY.into(),
            run_id: None,
            workspace_id: Some(ws.id.clone()),
            session_record_id: None,
            native_session_id: None,
            kind: "tool.completed".into(),
            privacy_class: activity::PRIVACY_METADATA.into(),
            payload: serde_json::json!({
                "tool": "Bash",
                "pullRequestUrl": "https://github.com/o/lib/pull/7",
            }),
        };
        let inbox = Inbox::new(activity::inbox_dir(&data_dir));
        inbox.record(&created).unwrap();

        let settled = settle_at_start(&store, &data_dir, &[]).unwrap();
        assert_eq!(settled.pull_requests.len(), 1);
        let gained = track_seen(&store, &settled.pull_requests, || Some(git()));
        assert_eq!(gained, BTreeSet::from([ws.id.clone()]));
        let rows = store.tracked_pull_requests(&ws.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://github.com/o/lib/pull/7");
        assert_eq!(rows[0].tracked_by, "hook");

        // The drain that follows finds the inbox empty, and the same report written again is a
        // duplicate: nothing tracked twice, nothing lost.
        let mut cursors = grok::Cursors::default();
        assert!(drain(&store, &data_dir, None, &mut cursors, || Some(git()))
            .tracked
            .is_empty());
        inbox.record(&created).unwrap();
        assert!(drain(&store, &data_dir, None, &mut cursors, || Some(git()))
            .tracked
            .is_empty());
        assert_eq!(store.tracked_pull_requests(&ws.id).unwrap().len(), 1);
    }

    #[test]
    fn only_a_file_arriving_wakes_the_drain_never_the_drain_itself() {
        assert!(brings_entries(&EventKind::Create(CreateKind::File)));
        assert!(brings_entries(&EventKind::Modify(ModifyKind::Name(
            RenameMode::To
        ))));
        assert!(brings_entries(&EventKind::Modify(ModifyKind::Any)));
        // What a drain does: lists the directory, then removes what it took.
        assert!(!brings_entries(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!brings_entries(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!brings_entries(&EventKind::Remove(RemoveKind::File)));
        assert!(!brings_entries(&EventKind::Any));
        assert!(!brings_entries(&EventKind::Other));
    }

    /// A deferred entry — a Codex turn whose session file is still being written — gets a
    /// retry on a timer, because nothing else will wake the drain for it: the session file is
    /// not watched, and the agent may then sit waiting for the user with no exit in sight.
    #[test]
    fn the_watcher_looks_again_on_its_own_only_while_something_is_deferred() {
        assert_eq!(
            wait_for(0),
            None,
            "nothing waiting: sleep until a file lands"
        );
        assert_eq!(wait_for(1), Some(RETRY_DEFERRED));
        assert_eq!(wait_for(7), Some(RETRY_DEFERRED));
        assert!(RETRY_DEFERRED.as_millis() < activity::codex::NOT_READY_GRACE_MS as u128);
    }
}
