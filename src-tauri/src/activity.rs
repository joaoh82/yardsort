//! The activity timeline's side of IPC: pages of a workspace's events, the diagnostics, the
//! switches, and clearing — and the inbox watcher that takes in what agents' hooks report.
//! The recording itself happens in the core (`yardsort_core::activity`) wherever something is
//! launched, exits, or is drained from the inbox; nothing here writes an event.

use std::path::Path;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};
use crate::store::{DiagnosticRow, EventRow, Store};
use crate::workspaces::commands::{settings_info, ActivitySettingsDto, SettingsInfo};

/// New events were recorded for these workspaces; a timeline showing one should ask again.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ActivityChanged {
    pub workspace_ids: Vec<String>,
}

/// A pull request an agent was seen creating is now tracked with these projects' workspaces:
/// the window should ask about their pull requests again rather than wait for its next look.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct TrackedPullRequestsChanged {
    pub project_ids: Vec<String>,
}

/// Take what the inbox holds into the store, and tell the timeline which workspaces moved.
/// Anything the drain had to leave — a Codex turn its session file has not finished — is
/// handed to the watcher, which looks again on its own clock: nothing else would.
pub fn drain_inbox(handle: &AppHandle, store: &Store, data_dir: &Path) {
    let report = yardsort_core::activity::import_inbox(store, data_dir);
    // This runs on the daemon's event thread; telling the project's own repository apart runs
    // git, which the next terminal event should not wait for.
    track_on_a_thread(handle, report.pull_requests);
    changed(handle, report.workspaces);
    // The watcher's thread also reads the session directories of harnesses that keep their
    // own — Grok's — and an exit is the moment their last lines land; so it is woken here
    // whatever the inbox held.
    if let Some(state) = handle.try_state::<AppState>() {
        launched(&state);
    }
}

/// Track the pull requests agents were seen creating while the app was closed, which the start
/// took out of the inbox.
pub fn track_settled(handle: &AppHandle, settled: yardsort_core::runtime::Settled) {
    track_on_a_thread(handle, settled.pull_requests);
}

/// Track pull requests agents were seen creating, and say so, on a thread of its own: finding
/// `git` may mean asking the login shell, and each link runs git once.
fn track_on_a_thread(handle: &AppHandle, seen: Vec<yardsort_core::tracked::Seen>) {
    if seen.is_empty() {
        return;
    }
    let handle = handle.clone();
    std::thread::spawn(move || {
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        let gained = yardsort_core::runtime::track_seen(&state.store, &seen, || {
            crate::git::Git::new(&state.env()).ok()
        });
        tracked(&handle, &state.store, gained);
    });
}

/// Say which projects' workspaces began following a pull request.
fn tracked(handle: &AppHandle, store: &Store, workspaces: std::collections::BTreeSet<String>) {
    let projects: std::collections::BTreeSet<String> = workspaces
        .iter()
        .filter_map(|id| store.workspace(id).ok().flatten())
        .map(|workspace| workspace.project_id)
        .collect();
    if !projects.is_empty() {
        let _ = TrackedPullRequestsChanged {
            project_ids: projects.into_iter().collect(),
        }
        .emit(handle);
    }
}

fn changed(handle: &AppHandle, workspaces: std::collections::BTreeSet<String>) {
    if !workspaces.is_empty() {
        let _ = ActivityChanged {
            workspace_ids: workspaces.into_iter().collect(),
        }
        .emit(handle);
    }
}

/// A launch happened: wake the watcher, so a harness whose files are read rather than delivered
/// — Grok — is followed from its first line, not from the next unrelated event.
pub fn launched(state: &AppState) {
    if let Some(watcher) = state.inbox_watcher.lock().unwrap().as_ref() {
        watcher.look_again();
    }
}

pub use yardsort_core::runtime::InboxWatcher;

/// Watch the inbox and drain it after each burst of files, telling the timeline what moved. The
/// drain runs on the watcher's own thread, so a hook landing during a page load never touches the
/// UI thread.
pub fn watch_inbox(handle: AppHandle, data_dir: &Path) -> notify::Result<InboxWatcher> {
    let dir = data_dir.to_path_buf();
    yardsort_core::runtime::watch_inbox(data_dir, move |cursors| {
        let Some(state) = handle.try_state::<AppState>() else {
            return 0;
        };
        // Yardsort's own Grok launches write into session directories, read when asked to.
        let grok_home = state
            .settings
            .get()
            .activity
            .capture_grok
            .then(|| yardsort_core::runtime::grok_home(&state.env()))
            .flatten();
        let drained = yardsort_core::runtime::drain(
            &state.store,
            &dir,
            grok_home.as_deref(),
            cursors,
            || crate::git::Git::new(&state.env()).ok(),
        );
        tracked(&handle, &state.store, drained.tracked);
        changed(&handle, drained.workspaces);
        drained.pending
    })
}

/// One recorded fact, as the timeline shows it. Timestamps are epoch milliseconds; `payload`
/// is the event's JSON, whose shape depends on `kind` (see the guide).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    /// Ingestion order: the cursor for paging, and the tie-breaker for display.
    pub seq: f64,
    pub id: String,
    pub schema_version: u32,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub occurred_at: f64,
    pub received_at: f64,
    pub kind: String,
    /// Who said so, how, and how sure to be: `yardsort` / `lifecycle` / `observed` for
    /// everything recorded today.
    pub producer: String,
    pub method: String,
    pub fidelity: String,
    pub privacy_class: String,
    pub payload: String,
}

impl From<EventRow> for ActivityEvent {
    fn from(row: EventRow) -> Self {
        Self {
            // Epoch milliseconds and sequence numbers fit a double exactly for a very long time.
            seq: row.seq as f64,
            id: row.id,
            schema_version: u32::try_from(row.schema_version).unwrap_or(u32::MAX),
            workspace_id: row.workspace_id,
            session_id: row.session_id,
            run_id: row.run_id,
            occurred_at: row.occurred_at as f64,
            received_at: row.received_at as f64,
            kind: row.kind,
            producer: row.producer,
            method: row.method,
            fidelity: row.fidelity,
            privacy_class: row.privacy_class,
            payload: row.payload,
        }
    }
}

/// One page of a workspace's timeline, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub events: Vec<ActivityEvent>,
    /// Whether asking again with the last event's `seq` as `before` would find more.
    pub has_more: bool,
}

/// What one agent run reported about one changed file. See
/// `yardsort_core::activity::provenance`.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WriteReport {
    pub run_id: Option<String>,
    pub harness_id: Option<String>,
    pub producer: String,
    pub method: String,
    pub fidelity: String,
    pub first_at: f64,
    pub last_at: f64,
    /// Reports, not lines: how many times the agent said it wrote the file.
    pub writes: u32,
}

/// One tool call that was executing when the file was last written.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ObservedMatch {
    pub run_id: String,
    pub harness_id: Option<String>,
    pub tool: Option<String>,
    pub from: f64,
    /// `None` while the tool call is still running.
    pub to: Option<f64>,
}

/// The file's last write, by its own clock, fell inside one or more tool calls' windows.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ObservedWrite {
    pub at: f64,
    pub matches: Vec<ObservedMatch>,
}

/// Every report about one workspace-relative path, oldest first, and what was observed of it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileReports {
    pub path: String,
    pub reports: Vec<WriteReport>,
    pub observed: Option<ObservedWrite>,
}

/// One of the workspace's agent runs and how it was asked to report, or `None` if it was not.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunCoverage {
    pub run_id: String,
    pub harness_id: Option<String>,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub capture: Option<String>,
    /// Whether a `None` capture is known to mean the run was not reporting.
    pub capture_known: bool,
}

/// Which of a workspace's files its agents said they wrote, and which runs could have said so.
/// Joined to git's own change list by the Changes panel; nothing here is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub files: Vec<FileReports>,
    pub runs: Vec<RunCoverage>,
}

impl From<yardsort_core::activity::provenance::Provenance> for Provenance {
    fn from(p: yardsort_core::activity::provenance::Provenance) -> Self {
        Self {
            files: p
                .files
                .into_iter()
                .map(|file| FileReports {
                    path: file.path,
                    reports: file
                        .reports
                        .into_iter()
                        .map(|r| WriteReport {
                            run_id: r.run_id,
                            harness_id: r.harness_id,
                            producer: r.producer,
                            method: r.method,
                            fidelity: r.fidelity,
                            first_at: r.first_at as f64,
                            last_at: r.last_at as f64,
                            writes: r.writes,
                        })
                        .collect(),
                    observed: file.observed.map(|observed| ObservedWrite {
                        at: observed.at as f64,
                        matches: observed
                            .matches
                            .into_iter()
                            .map(|m| ObservedMatch {
                                run_id: m.run_id,
                                harness_id: m.harness_id,
                                tool: m.tool,
                                from: m.from as f64,
                                to: m.to.map(|to| to as f64),
                            })
                            .collect(),
                    }),
                })
                .collect(),
            runs: p
                .runs
                .into_iter()
                .map(|run| RunCoverage {
                    run_id: run.run_id,
                    harness_id: run.harness_id,
                    started_at: run.started_at as f64,
                    ended_at: run.ended_at.map(|at| at as f64),
                    capture: run.capture,
                    capture_known: run.capture_known,
                })
                .collect(),
        }
    }
}

/// A handoff packet: what Yardsort recorded about a workspace, as the opening prompt for the
/// next agent there. See `yardsort_core::activity::handoff`.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HandoffPacket {
    /// Markdown, for the composer's message box: the user reads and edits it before sending.
    pub text: String,
    /// How many agent runs and events stand behind it, for the line above the box.
    pub runs: u32,
    pub events: u32,
}

/// Assemble the packet for a workspace: the core's facts — the store's and git's — and, when
/// Assist is reviewing changes, its judgment of each file from the same review the Changes
/// list shows. Nothing new is sent for the packet: a review that is switched off, has no
/// key, or fails leaves the packet without a ranking, and the packet reads the same.
#[tauri::command]
#[specta::specta]
pub async fn workspace_handoff(app: AppHandle, workspace_id: String) -> IpcResult<HandoffPacket> {
    use yardsort_core::activity::handoff;
    let facts = handoff_facts(app, workspace_id).await?;
    Ok(HandoffPacket {
        runs: facts.runs.len() as u32,
        events: facts.events as u32,
        text: handoff::render(&facts),
    })
}

/// A fork packet: what Yardsort recorded about a workspace, for an agent starting a new
/// workspace on a new branch made from this one's. See `yardsort_core::activity::handoff`.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForkPacket {
    /// Markdown, sent after the user's own message, which is the new workspace's task.
    pub text: String,
    /// The branch to start the new one from: the workspace's own, as git has it now. `None`
    /// when it is not on a branch, and there is nothing to fork.
    pub branch: Option<String>,
    /// Files changed and not committed there: a fork starts from the commits, so these stay.
    pub uncommitted: u32,
}

/// Assemble the fork packet for a workspace, from the same facts as a handoff.
#[tauri::command]
#[specta::specta]
pub async fn workspace_fork(app: AppHandle, workspace_id: String) -> IpcResult<ForkPacket> {
    use yardsort_core::activity::handoff::{self, Purpose};
    let facts = handoff_facts(app, workspace_id).await?;
    Ok(ForkPacket {
        text: handoff::render_for(&facts, Purpose::Fork),
        branch: facts.branch.clone(),
        uncommitted: facts.uncommitted.len() as u32,
    })
}

async fn handoff_facts(
    app: AppHandle,
    workspace_id: String,
) -> IpcResult<yardsort_core::activity::handoff::Facts> {
    use yardsort_core::activity::handoff::{self, Ranking};
    let (mut facts, prepared) = blocking(app.clone(), {
        let workspace_id = workspace_id.clone();
        move |state| {
            let (row, root) = crate::changes::commands::workspace(state, &workspace_id)?;
            let git = crate::git::Git::new(&state.env())?;
            let mut facts = handoff::facts(
                &state.store,
                &git,
                &root,
                &workspace_id,
                row.base_branch.as_deref(),
            )?;
            facts.said = yardsort_core::activity::digest::packet_section(
                &state.store,
                &row,
                &crate::conversations::sources(state),
                crate::conversations::now(),
            )?;
            // A review that cannot run is no ranking, not a failed packet.
            let prepared = crate::assist::commands::prepare_review(state, &workspace_id).ok();
            Ok((facts, prepared))
        }
    })
    .await?;
    if let Some(prepared) = prepared {
        if let Ok(jev) = crate::assist::jev::Jev::new(&prepared.key) {
            let state = app.state::<AppState>();
            if let Ok(review) = crate::assist::review::review(
                std::sync::Arc::new(jev),
                &state.assist.reviews,
                prepared.task,
                prepared.inputs,
                prepared.thresholds,
            )
            .await
            {
                facts.rankings = review
                    .files
                    .into_iter()
                    .filter(|file| file.not_checked.is_none())
                    .map(|file| {
                        (
                            (file.scope, file.path.clone()),
                            Ranking {
                                relevance: file.relevance.as_ref().and_then(camel_word),
                                flags: file.flags.iter().filter_map(camel_word).collect(),
                            },
                        )
                    })
                    .collect();
            }
        }
    }
    Ok(facts)
}

/// A serde-named enum value as the string its `rename_all` gives it — the same word the
/// bindings and the badges use.
fn camel_word<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
}

/// The reported writes for a workspace, as they stand now, and for each of `paths` — the
/// change list's files — whether its last write fell inside a tool call.
#[tauri::command]
#[specta::specta]
pub async fn workspace_provenance(
    app: AppHandle,
    workspace_id: String,
    paths: Vec<String>,
) -> IpcResult<Provenance> {
    blocking(app, move |state| {
        let (_, root) = crate::changes::commands::workspace(state, &workspace_id)?;
        let files = yardsort_core::activity::provenance::last_written(&root, &paths);
        Ok(yardsort_core::activity::provenance::of(&state.store, &workspace_id, &files)?.into())
    })
    .await
}

/// How many events a page holds at most, whatever is asked for.
const MAX_PAGE: u32 = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCounter {
    pub name: String,
    pub count: f64,
    pub last_at: f64,
    pub last_detail: Option<String>,
}

impl From<DiagnosticRow> for ActivityCounter {
    fn from(row: DiagnosticRow) -> Self {
        Self {
            name: row.name,
            count: row.count as f64,
            last_at: row.last_at as f64,
            last_detail: row.last_detail,
        }
    }
}

/// What there is, and what went wrong recording it. For Settings → General and bug reports.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDiagnostics {
    pub events: f64,
    pub runs: f64,
    /// Where the daemon keeps exits for a closed window, and how many are waiting there.
    pub spool_dir: String,
    pub spool_pending: u32,
    /// Where agents' hooks leave what they report, and how many entries are waiting there.
    pub inbox_dir: String,
    pub inbox_pending: u32,
    /// The settings file Claude Code launches are given when capture is on.
    pub claude_hooks_file: String,
    pub counters: Vec<ActivityCounter>,
}

/// A page of a workspace's events, newest first: those before `before_seq` (or the newest,
/// when `None`), at most `limit` of them.
#[tauri::command]
#[specta::specta]
pub async fn activity_timeline(
    app: AppHandle,
    workspace_id: String,
    before_seq: Option<f64>,
    limit: u32,
) -> IpcResult<ActivityPage> {
    blocking(app, move |state| {
        let limit = limit.clamp(1, MAX_PAGE) as usize;
        // One more than asked for says whether there is another page, without a count query.
        let mut rows =
            state
                .store
                .events(&workspace_id, before_seq.map(|seq| seq as i64), limit + 1)?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok(ActivityPage {
            events: rows.into_iter().map(ActivityEvent::from).collect(),
            has_more,
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn activity_diagnostics(app: AppHandle) -> IpcResult<ActivityDiagnostics> {
    blocking(app, move |state| {
        let (events, runs) = state.store.activity_counts()?;
        let spool = pty_ipc::spool::Spool::new(yardsort_core::activity::spool_dir(&state.data_dir));
        let inbox = yardsort_core::activity::inbox::Inbox::new(yardsort_core::activity::inbox_dir(
            &state.data_dir,
        ));
        let pending = |entries: std::io::Result<Vec<std::path::PathBuf>>| {
            entries
                .map(|entries| u32::try_from(entries.len()).unwrap_or(u32::MAX))
                .unwrap_or(0)
        };
        Ok(ActivityDiagnostics {
            events: events as f64,
            runs: runs as f64,
            spool_dir: spool.dir().display().to_string(),
            spool_pending: pending(spool.entries()),
            inbox_dir: inbox.dir().display().to_string(),
            inbox_pending: pending(inbox.entries()),
            claude_hooks_file: yardsort_core::activity::claude::settings_path(&state.data_dir)
                .display()
                .to_string(),
            counters: state
                .store
                .diagnostics()?
                .into_iter()
                .map(ActivityCounter::from)
                .collect(),
        })
    })
    .await
}

/// Forget recorded activity — one workspace's, or all of it. Runs still going keep their row,
/// so their exit can still be matched; nothing about the processes themselves is touched.
#[tauri::command]
#[specta::specta]
pub async fn activity_clear(app: AppHandle, workspace_id: Option<String>) -> IpcResult<()> {
    blocking(app, move |state| {
        state.store.clear_activity(workspace_id.as_deref())?;
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_save_activity(
    app: AppHandle,
    activity: ActivitySettingsDto,
) -> IpcResult<SettingsInfo> {
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| {
                settings.activity.record_lifecycle = activity.record_lifecycle;
                settings.activity.show_timeline = activity.show_timeline;
                settings.activity.capture_claude = activity.capture_claude;
                settings.activity.capture_codex = activity.capture_codex;
                settings.activity.capture_opencode = activity.capture_opencode;
                settings.activity.capture_grok = activity.capture_grok;
                settings.activity.capture_omp = activity.capture_omp;
                settings.activity.capture_pi = activity.capture_pi;
                settings.activity.capture_cursor = activity.capture_cursor;
            })
            .map_err(|error| {
                IpcError::new(
                    "settings_write",
                    format!("Could not save settings: {error}"),
                )
            })?;
        settings_info(state)
    })
    .await
}
