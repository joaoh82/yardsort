//! The workflow driver's home in the app: a background thread, never the UI's, that moves
//! queued and running workflow runs on. See `yardsort_core::workflow::driver` for what a tick
//! does; this module owns the thread, the wake-ups and what only the app can do.
//!
//! It wakes every second, and at once when a session goes quiet or ends. Runs are picked up
//! from the database whoever queued them: `ys workflow run` writes the row and nothing else.
//!
//! The driver runs only while it holds the profile's app lock (`yardsort_core::presence`). That
//! is also how `ys` knows the app is running, and it keeps two apps on one profile from driving
//! the same run twice.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use pty_host::{HostEvent, SessionInfo, TermSize};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;
use yardsort_core::forge::Gh;
use yardsort_core::launch::{HarnessRequest, Launch};
use yardsort_core::workflow::driver::{self, Conversation, Hands, PrAsk};

use crate::state::AppState;

pub mod commands;

/// How often the driver looks when nothing wakes it sooner: queued runs start within this, and
/// timeouts are this precise.
const TICK: Duration = Duration::from_secs(1);

/// How long one transcript read may take before the step fails. The driver thread does not
/// wait this long: it peeks, and asks again on the next tick, reusing the same read.
const TRANSCRIPT_LIMIT: Duration = Duration::from_secs(20);

/// How long the first peek waits, so a short transcript finishes on the same tick.
const TRANSCRIPT_PEEK: Duration = Duration::from_millis(100);

/// The size an agent is started at. Its tab resizes it when the window shows it.
const SIZE: TermSize = TermSize {
    cols: 120,
    rows: 30,
};

/// A workflow run, a device through Remote, or `ys` started a session in a workspace: the window
/// should show it as a tab, and the workspace if it is new.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionStarted {
    pub workspace_id: String,
    pub session_id: String,
}

/// Managed by Tauri while the driver runs.
pub struct Driver {
    wake: Mutex<Sender<()>>,
    driver: driver::Driver,
}

impl Driver {
    /// Every host event passes through here. A quiet or an exit is a reason to look now.
    pub fn observe(&self, event: &HostEvent) {
        if self.driver.observe(event) {
            self.wake();
        }
    }

    /// Issue a replacement code and remember it, so the next tick does not issue another.
    pub fn issue_approval_code(
        &self,
        store: &yardsort_core::store::Store,
        run_id: &str,
        step_id: &str,
    ) -> yardsort_core::store::StoreResult<Option<(String, bool)>> {
        self.driver
            .issue_approval_code(store, run_id, step_id, true)
    }

    /// Issue a replacement answer code and remember it, so the next tick does not issue another.
    pub fn issue_answer_code(
        &self,
        store: &yardsort_core::store::Store,
        run_id: &str,
        step_id: &str,
    ) -> yardsort_core::store::StoreResult<Option<(String, bool)>> {
        self.driver.issue_answer_code(store, run_id, step_id, true)
    }

    /// The one-time code a signed-in phone may submit. Not logged, and not written.
    pub fn code_for_phone(
        &self,
        store: &yardsort_core::store::Store,
        run_id: &str,
        step_id: &str,
    ) -> yardsort_core::store::StoreResult<Option<String>> {
        self.driver.code_for_phone(store, run_id, step_id)
    }

    /// Look at the runs now rather than at the next tick: one was just queued, say.
    pub fn wake(&self) {
        let _ = self
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(());
    }
}

/// Start driving. Only for the copy holding the profile (`one_copy`), so that one app drives its runs.
pub fn start(app: &AppHandle) {
    let (wake, woken) = mpsc::channel();
    app.manage(Driver {
        wake: Mutex::new(wake),
        driver: driver::Driver::default(),
    });
    // Smoke test runs are carried out under the same lock, from the same thread's ticks.
    crate::smoke::start(app);
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("workflows".into())
        .spawn(move || drive(&handle, &woken));
    if let Err(error) = spawned {
        eprintln!("workflows: cannot start the driver: {error}");
    }
}

fn drive(handle: &AppHandle, woken: &Receiver<()>) {
    loop {
        match woken.recv_timeout(TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // One pass answers every wake that arrived meanwhile.
        while woken.try_recv().is_ok() {}
        let (Some(state), Some(driver)) =
            (handle.try_state::<AppState>(), handle.try_state::<Driver>())
        else {
            continue;
        };
        let hands = AppHands {
            handle,
            state: &state,
        };
        // Git answers what a workspace row does not know: the branch of the project's own
        // checkout. Without git nothing here could have started anyway.
        let git = match crate::git::Git::new(&state.env()) {
            Ok(git) => git,
            Err(error) => {
                eprintln!("workflows: {error}");
                continue;
            }
        };
        match driver
            .driver
            .tick(&state.store, state.host.as_ref(), &hands, &git)
        {
            Ok(true) => {
                let _ = commands::WorkflowRunsChanged {}.emit(handle);
            }
            Ok(false) => {}
            Err(error) => eprintln!("workflows: cannot read runs: {}", error.message),
        }
        crate::smoke::tick(handle, &state);
    }
}

struct TranscriptJob {
    started: Instant,
    rx: Receiver<Result<Option<String>, String>>,
}

/// One read belongs to one step of one run. A later step does not pick up an earlier result.
type TranscriptKey = (String, String, String, bool);

fn transcript_jobs() -> &'static Mutex<HashMap<TranscriptKey, TranscriptJob>> {
    static JOBS: OnceLock<Mutex<HashMap<TranscriptKey, TranscriptJob>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn approval_codes() -> &'static Mutex<HashMap<(String, String), String>> {
    static CODES: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    CODES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn remember_approval_code(run_id: &str, step_id: &str, code: &str) {
    approval_codes()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((run_id.to_owned(), step_id.to_owned()), code.to_owned());
}

pub(crate) fn approval_code_for(run_id: &str, step_id: &str) -> Option<String> {
    approval_codes()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&(run_id.to_owned(), step_id.to_owned()))
        .cloned()
}

/// Peek at the read for this step. The first call starts one thread. Later calls reuse it.
/// A job older than [`TRANSCRIPT_LIMIT`] is dropped and the step fails. A new thread is not
/// started in its place, or the step would wait forever. Finished jobs of other steps stay
/// out of this key.
fn poll_transcript(
    handle: &AppHandle,
    run_id: &str,
    step_id: &str,
    workspace_id: &str,
    record: &str,
    raw: bool,
) -> Result<Conversation, String> {
    let key = (
        run_id.to_owned(),
        step_id.to_owned(),
        record.to_owned(),
        raw,
    );
    let mut jobs = transcript_jobs()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let stale: Vec<TranscriptKey> = jobs
        .iter()
        .filter(|(_, job)| job.started.elapsed() >= TRANSCRIPT_LIMIT)
        .map(|(key, _)| key.clone())
        .collect();
    let this_was_stale = stale.iter().any(|old| old == &key);
    for old in stale {
        jobs.remove(&old);
    }
    if this_was_stale {
        return Err("Reading that conversation took too long, so it was not copied.".into());
    }
    if !jobs.contains_key(&key) {
        let (tx, rx) = mpsc::channel();
        let handle = handle.clone();
        let workspace_id = workspace_id.to_owned();
        let record = record.to_owned();
        let spawned = std::thread::Builder::new()
            .name("artifact-transcript".into())
            .spawn(move || {
                let result = read_transcript(&handle, &workspace_id, &record, raw)
                    .map(|text| text.map(|text| yardsort_core::workflow::artifact::cap(&text).0));
                let _ = tx.send(result);
            });
        if let Err(error) = spawned {
            return Err(format!(
                "Could not read that session's conversation: {error}"
            ));
        }
        jobs.insert(
            key.clone(),
            TranscriptJob {
                started: Instant::now(),
                rx,
            },
        );
    }
    let job = jobs.remove(&key).expect("the job was just inserted");
    let started = job.started;
    drop(jobs);
    let wait = TRANSCRIPT_PEEK.saturating_sub(started.elapsed());
    match job.rx.recv_timeout(wait) {
        Ok(_) if started.elapsed() >= TRANSCRIPT_LIMIT => {
            Err("Reading that conversation took too long, so it was not copied.".into())
        }
        Ok(result) => result.map(Conversation::Ready),
        Err(RecvTimeoutError::Timeout) if started.elapsed() >= TRANSCRIPT_LIMIT => {
            Err("Reading that conversation took too long, so it was not copied.".into())
        }
        Err(RecvTimeoutError::Timeout) => {
            transcript_jobs()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, job);
            Ok(Conversation::Later)
        }
        Err(RecvTimeoutError::Disconnected) => {
            Err("Could not read that session's conversation.".into())
        }
    }
}

/// One session's conversation, read from the harness files. Called off the driver thread.
fn read_transcript(
    handle: &AppHandle,
    workspace_id: &str,
    record: &str,
    raw: bool,
) -> Result<Option<String>, String> {
    use yardsort_core::activity::digest;
    use yardsort_core::workflow::artifact;
    let Some(state) = handle.try_state::<AppState>() else {
        return Err("Yardsort is not ready to read that conversation.".into());
    };
    let workspace = state
        .store
        .workspace(workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The workspace is gone.".to_owned())?;
    let conversation = digest::one(
        &state.store,
        &workspace,
        record,
        &crate::conversations::sources(&state),
        crate::conversations::now(),
    )
    .map_err(|error| error.to_string())?;
    let Some(conversation) = conversation else {
        return Ok(None);
    };
    artifact::conversation_text(std::slice::from_ref(&conversation), record, raw)
}

struct AppHands<'a> {
    handle: &'a AppHandle,
    state: &'a AppState,
}

impl Hands for AppHands<'_> {
    fn start(&self, workspace_id: &str, request: HarnessRequest) -> Result<SessionInfo, String> {
        let session = crate::terminal::spawn_in_workspace(
            self.state,
            workspace_id,
            Launch::Harness(request),
            SIZE,
        )
        .map_err(|error| error.message)?;
        let _ = SessionStarted {
            workspace_id: workspace_id.to_owned(),
            session_id: session.id.0.clone(),
        }
        .emit(self.handle);
        Ok(session)
    }

    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String> {
        let mut notification = self.handle.notification().builder().title(title);
        if let Some(body) = body {
            notification = notification.body(body);
        }
        notification
            .show()
            .map_err(|error| format!("Could not show the notification: {error}"))
    }

    fn handoff(&self, workspace_id: &str) -> Option<String> {
        use yardsort_core::activity::handoff;
        let state = self.state;
        let (row, root) = crate::changes::commands::workspace(state, workspace_id).ok()?;
        let git = crate::git::Git::new(&state.env()).ok()?;
        let mut facts = handoff::facts(
            &state.store,
            &git,
            &root,
            &row.id,
            row.base_branch.as_deref(),
        )
        .ok()?;
        facts.said = yardsort_core::activity::digest::packet_section(
            &state.store,
            &row,
            &crate::conversations::sources(state),
            crate::conversations::now(),
        )
        .unwrap_or(None);
        Some(handoff::render(&facts))
    }

    fn approval_code(&self, run_id: &str, step_id: &str, code: &str) {
        remember_approval_code(run_id, step_id, code);
    }

    fn include_approval_code(&self) -> bool {
        self.state.settings.get().workflows.show_approval_code()
    }

    fn conversation(
        &self,
        workspace_id: &str,
        pty_id: &str,
        raw: bool,
        run_id: &str,
        step_id: &str,
    ) -> Result<Conversation, String> {
        use yardsort_core::launch::RECORD_LABEL;
        // The terminal id is not the session record. The record is what the transcript is filed
        // under. Tool output is not in those turns, and this does not read the terminal scrollback.
        let record = self
            .state
            .host
            .list()
            .into_iter()
            .find(|session| session.id.0 == pty_id)
            .and_then(|session| session.labels.get(RECORD_LABEL).cloned());
        let Some(record) = record else {
            return Ok(Conversation::Ready(None));
        };
        // Reading a transcript walks the harness's files. That stays off this thread, which is
        // also what moves every other run on. One read per session is reused until it finishes
        // or it has had TRANSCRIPT_LIMIT. A second call does not start a second thread.
        poll_transcript(self.handle, run_id, step_id, workspace_id, &record, raw)
    }

    fn pr_posts(&self, workspace_id: &str, pr: &yardsort_core::workflow::runs::PrFacts) -> PrAsk {
        // Found here, quickly; `gh` itself runs in the job, on the driver's forge thread. A link
        // needs no repository around it: one elsewhere is asked about from anywhere, should the
        // workspace's folder have gone.
        let folder = crate::changes::commands::workspace(self.state, workspace_id)
            .map(|(_, root)| root)
            .or_else(|error| match pr.elsewhere {
                Some(_) => Ok(std::env::temp_dir()),
                None => Err(error),
            });
        let found = folder.map_err(|error| error.message).and_then(|root| {
            Gh::find(&self.state.env())
                .map(|gh| (gh, root))
                .ok_or_else(|| "The GitHub CLI (`gh`) is not on your PATH.".to_owned())
        });
        // The workspace's own by its number, one elsewhere by its link.
        let selector = pr.selector();
        Box::new(move || {
            let (gh, root) = found?;
            gh.pr_posts_of(&root, &selector, driver::FORGE_LIMIT)
                .map_err(|error| error.to_string())
        })
    }
}
