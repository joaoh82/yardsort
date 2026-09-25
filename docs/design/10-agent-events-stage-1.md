# 10 — Agent events: fit report and the Stage 0–1 slice

Status: **implemented (Stage 0–1)** · 24 September 2026 · Companion to
[09-agent-events-and-memory](09-agent-events-and-memory.md), which is the proposal. This document
is the record the proposal asked for before any code: what the repository looked like when the
work started, where the proposal's premises had drifted, the decisions taken, and what shipped.

## 1 · Current state at the start of the work

| Fact               | Value                                                                                                                        |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| Branch / HEAD      | `ys/write-concise-fit-report` at `b502b1f` ("added design doc for new agent event and memory feature (#42)"), clean tree     |
| Relation to `main` | `origin/main` fetched; identical to HEAD (0 ahead / 0 behind). `origin/design/agents-events-and-memory` carries the same doc |
| Since the snapshot | The proposal was written against `cb55ee1` (v0.9.2). `cb55ee1..HEAD` is 113 files, +9 228 / −336, seven pull requests        |

What changed between the snapshot and HEAD that this feature has to respect:

- **Four more migrations.** `0005_forgotten_workspaces`, `0006_removed_projects`,
  `0007_project_automation`, `0008_workspace_preparation`. The proposal says "add migrations after
  0004"; the slice adds **0009**. Workspaces can now be _forgotten_ (row hidden, sessions kept)
  and projects _removed with history_ (row hidden, everything kept), so any new table hanging off
  `workspaces` inherits that behaviour through `ON DELETE CASCADE` and needs no special case.
- **Seven built-in harnesses, not four.** `claude`, `codex`, `grok`, `opencode` plus `omp`,
  `cursor` (`cursor-agent`) and `pi` (`crates/core/src/harness.rs:203–360`). The user's note
  confirms these arrived after the proposal. The Stage 2 coverage matrix in §6 covers all seven.
- **Non-harness workspace processes.** `Launcher::project_run` (`crates/core/src/launch.rs:62`)
  starts the project's run command in a PTY labelled `projectRun`; shells and plain programs go
  through `Launcher::in_workspace` too. The proposal only considered harness conversations.
- **`write_args` drafting** (`crates/core/src/draft.rs`, `src-tauri/src/draft/`) runs an agent
  non-interactively through pipes to write commit messages. It is not a PTY session and gets no
  run row; it is out of scope for lifecycle events.
- **`ys workspace delete`, `ys attach`, `ys logs`** exist. `ys attach` already listens for the
  daemon's `Exited` event (`crates/cli/src/commands/attach.rs:54–60`) but settles nothing.
- **No telemetry, OTLP, hook, session-file, task or attempt code exists.** A repository-wide
  search for those terms finds only the proposal, the README's "no telemetry" promise, git's use
  of the word "hooks", and a retry counter in `assist/jev.rs`. The proposal's "do not assume they
  are absent" was right to ask and the answer is: absent.
- **The Quiet-screen judgment** (open question 16, decided 2026-09-21) is still a decision, not
  code. Nothing in `src-tauri/src/assist/` reads a screen; `review.rs` and `suggest.rs` send
  diffs, paths, first messages and composer text only. Its four design questions (what is sent,
  secrets on screen, opt-in scope, what a notification may repeat) remain unanswered.

## 2 · How launch, exit and recovery actually work today

Traced, with the lines that matter:

- **App and CLI share one launcher.** `Launcher::in_workspace` (`launch.rs:112–158`) resolves
  the launch, labels the PTY (`workspace`, `harness`, `harnessSession`, `record`), spawns, and
  only _then_ inserts the `sessions` row and calls `settle_record` in case the process already
  exited. Resume and fork do **not** go through it: `src-tauri/src/sessions.rs:119–218` builds
  labels and calls `terminal::start` directly, then `mark_session_running` or `add_session`.
- **The child environment is the whole login-shell environment** (`launch.rs:307–313`,
  `clear_env` on Unix). No Yardsort variable is set on a session. [03-architecture](03-architecture.md#environment-resolution-important)
  says `YARDSORT_WORKSPACE` / `YARDSORT_PROJECT` are set; only project setup scripts get them
  (`project_automation.rs:191`). Corrected in this pass.
- **Exit ownership.** With a window open, the daemon's `Exited` reaches the app's event sink
  (`src-tauri/src/lib.rs:222–233`), which calls `end_session_by_pty` before emitting to the
  webview. With no window, the daemon keeps the exited session in its list (`ys session list`
  shows `gone`), then **exits itself 10 s after it has no client and no running session**
  (`pty-ipc/src/server.rs:118–140`). The exit code is lost with it; the next app start marks the
  row _interrupted_ (`end_interrupted_sessions`, `lib.rs:236–243`). This is the one real gap in
  "reliable window-closed exit delivery", and it is small: exits are already announced, they are
  just not kept.
- **The daemon has no storage and knows only its socket** (`daemon.rs:35–50`); it is this same
  binary re-run with `--yardsort-daemon <socket>`. The wire protocol is `PROTOCOL = 1`
  (`proto.rs:19`), JSON with `serde` defaults, so a struct can gain an optional field without a
  bump; adding an op or changing a frame kind would need one.
- **Busy/Quiet** are activity heuristics (`pty-host`, 3 s quiet) that drive dots, badges and
  notifications (`src/features/terminal/useTerminalSessions.ts`). They are not persisted anywhere
  and the proposal is right that they must not be re-labelled as semantic state.

## 3 · Where the proposal's assumptions no longer fit

| Proposal said                                           | Reality                                                             | Decision                                                                                                                                            |
| ------------------------------------------------------- | ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Migrations after 0004                                   | 0005–0008 shipped                                                   | Migration **0009**. Never touch the earlier ones.                                                                                                   |
| Four harnesses; Grok "and custom CLI"                   | Seven built-ins                                                     | Matrix covers all seven; OMP, Cursor and Pi start lifecycle-only like Grok.                                                                         |
| `project_id` on every event                             | `workspaces.project_id` exists; hidden projects keep rows           | Events carry `workspace_id` only; the project is a join. Cascade follows the workspace.                                                             |
| Session record ≠ PTY ≠ native id                        | True, and PTY ids are labelled `record`                             | Kept. A **run** is a fourth id, created before spawn; the PTY id is attached once the spawn returns.                                                |
| "Anonymous shell run": decide                           | Every workspace launch is labelled `workspace`                      | Every workspace launch gets a run (`harness`, `shell` or `program`); spawns with no workspace (Settings → Test launch) get none.                    |
| Daemon spool preferred over daemon writing SQLite       | Daemon is storage-free by design, and one writer per file is a rule | **Spool.** One JSON file per exit under `<data-dir>/activity/spool/`, atomic rename, imported and deleted by whichever client connects next.        |
| Pass ids in child env                                   | No Yardsort variable set today                                      | `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID`, `YARDSORT_SESSION_RECORD_ID` appended to the resolved environment for workspace launches. Nothing else. |
| Quiet-screen may "supply an explicit, labeled judgment" | Not built; four design questions open                               | Nothing here depends on it. If it ships it emits `evaluation.recorded` from producer `assist`, method `screen`, fidelity `inferred`, own opt-in.    |
| Timeline "behind an experimental setting"               | Settings live in `settings.toml`, General tab                       | `[activity] record_lifecycle` (default on) and `show_timeline` (default off, labelled experimental).                                                |

The `AGENTS.md` rule — status, readiness and notifications derive from PTY _activity_, never from
parsed output — is untouched by Stage 1. Nothing in this slice reads a byte of terminal output;
the only facts recorded are the ones Yardsort already owned (who launched what, where, when it
exited, with what code). The Quiet-screen decision is the one sanctioned exception to that rule
and it stays a separate Assist feature with its own privacy design; the event store gives it a
place to _put_ a labelled judgment later, and takes nothing from it now.

## 4 · Decisions

| Question          | Decision                                                                                                                                                                                                                                                                  |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Event write owner | The core `Store`, from whichever client is launching or reconciling: the app or `ys`. The daemon writes only the spool. Same single-writer-per-connection discipline as every other table (immediate transactions, 5 s busy timeout).                                     |
| Store             | SQLite, tables `agent_runs`, `agent_events`, `agent_event_diagnostics` (migration 0009). NDJSON is `ys activity export`, nothing else.                                                                                                                                    |
| Correlation       | A run id is generated **before** spawn and written as a pending row; the PTY id is attached after; the session record is linked after it is inserted (it is inserted after spawn today, and a spawn failure must not leave a session row, so that order stays).           |
| Dedupe            | `agent_events(producer, source_key)` is unique. Stage 1 keys: `run:<id>:started`, `pty:<id>:exited`, `run:<id>:resumed`, `run:<id>:forked`. Inserts are `INSERT OR IGNORE`; the second delivery of an exit is a counted duplicate, not an error.                          |
| Spool             | Daemon side: `pty_ipc::Spool`, one `<at>-<pty>.json` per `Exited`, written to `.tmp` then renamed. Cap 2 000 files; past it the daemon drops and counts in `dropped`. Client side: `activity::import_spool` settles the run, the session row and the event, then deletes. |
| Recovery          | At app start: connect → import spool → end runs and sessions whose PTY is not alive as _interrupted_. A clean exit spooled while the window was closed therefore keeps its code.                                                                                          |
| Failure isolation | Every activity write is best effort behind `Recorder`: an error is printed, counted under `write_failed`, and the launch proceeds. A missing or broken table cannot stop a harness (tested).                                                                              |
| Retention         | 20 000 events or 90 days, pruned on app start and after imports; **Clear** in Settings and `activity_clear` per workspace. Rows go with their workspace when it is deleted or forgotten without history, exactly as sessions do.                                          |
| Privacy           | Payloads are metadata: harness id, model, effort, program basename, continuation, exit code/signal, who launched, how the exit arrived. No prompt, no argv, no paths beyond the workspace id, no output. `sessions.prompt` is unchanged and separately reviewed later.    |
| Daemon protocol   | Unchanged (`PROTOCOL = 1`). The spool directory is a third argument on the daemon's command line; an older daemon simply never spools and the app degrades to today's _interrupted_ behaviour.                                                                            |
| Stage order       | Unchanged. Stage 2 starts with Claude Code hooks (fixtures first), see §6.                                                                                                                                                                                                |

## 5 · Stage 1 event contract (schema 1)

Producer `yardsort`, method `lifecycle`, fidelity `observed`, privacy class `metadata`.

| Kind                   | When                                                   | Payload                                                                                                                                                  |
| ---------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `process.started`      | A workspace launch returned a PTY                      | `kind` (harness/shell/program), `harnessId`, `model`, `effort`, `program`, `continuation` (fresh/resumed/forked), `launchedBy` (app/cli), `ptySessionId` |
| `process.spawn_failed` | The host refused to start the program                  | `kind`, `harnessId`, `program`, `reason`                                                                                                                 |
| `process.exited`       | The process ended, however that was learned            | `exitCode`, `success`, `signal`, `reason` (exited/interrupted), `via` (live/settle/spool/reconcile)                                                      |
| `session.resumed`      | A recorded conversation was continued in a new process | `sessionId`                                                                                                                                              |
| `session.forked`       | A recorded conversation was copied into a new record   | `sessionId`, `fromSessionId`                                                                                                                             |

Not captured in Stage 1, and the timeline says so: prompts, tool calls, file edits, commands,
usage, approvals, sub-agents. Those need Stage 2 adapters with fixtures. `workspace.changed` is
deferred to Stage 3 alongside the review join, because on its own it would only restate the
Changes panel.

## 6 · Stage 0 baseline and the Stage 2 coverage matrix

Versions installed on the development machine on 2026-09-24, and what each CLI's own `--help`
says about native telemetry surfaces. **Nothing was configured**; this is a reading of the help
text and public documentation, not a fixture suite. Rows marked _unverified_ are unknowns to be
settled with recorded samples before an adapter is written.

| Harness     | Version            | Native surface seen in `--help`                                                                                     | First Stage 2 path                                                            | Status                                                                                          |
| ----------- | ------------------ | ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Claude Code | 2.1.280            | Hooks defined in settings and plugins (`--bare` skips them); `--include-hook-events`; plugin directories            | Opt-in hook commands for prompt / tool / permission / session lifecycle       | **Documented publicly; no fixtures recorded here.** Schema and ordering per version unverified. |
| Codex       | 0.156.1            | `plugin` subcommand; hook trust (`--dangerously-bypass-hook-trust`) — a hook mechanism exists and is gated by trust | Hooks if their events cover tools; otherwise its session JSONL under fixtures | **Unverified.** Which events hooks deliver, and the session-file layout, unknown here.          |
| Grok        | 1.0.41             | `plugin` subcommand only                                                                                            | Lifecycle-only until a stable hook or log surface is found                    | Unknown.                                                                                        |
| OpenCode    | 1.18.31            | `plugin <module>` install; `--pure` disables plugins                                                                | A managed plugin emitting tool lifecycle and usage                            | **Unverified** plugin API; version drift expected.                                              |
| OMP         | 18.2.11            | `--hook=<file>` loads a hook/extension file; plugin install/link                                                    | Possibly a hook file; needs the extension API read                            | Unknown.                                                                                        |
| Cursor      | 2026.09.23-86fc751 | `--plugin-dir`; `plugin` subcommand                                                                                 | Lifecycle-only                                                                | Unknown.                                                                                        |
| Pi          | 0.87.1             | `PI_TELEMETRY` (its **own** install telemetry, unrelated to us — leave it alone); plugins                           | Lifecycle-only                                                                | Unknown.                                                                                        |

Every harness gets Stage 1 lifecycle coverage regardless of the table above, including custom
ones. The proposal's Beacon-derived expectations for Claude, Codex and OpenCode stand as
_expectations_ only until fixtures exist in this repository.

## 7 · What shipped in this slice

- `crates/core/migrations/0009_agent_runs_and_events.sql` — the three tables and indexes.
- `crates/core/src/activity.rs` — types, the `Recorder`, spool import, retention, export rows.
- `crates/core/src/store.rs` — run/event/diagnostic queries.
- `crates/core/src/launch.rs` — run created before spawn; labels and child env carry the ids;
  `start_recorded` shared by fresh, resumed and forked launches; `settle_record` also settles the
  run.
- `crates/pty-ipc/src/spool.rs`, `server.rs`, `bin/pty-daemon.rs`; `crates/core/src/daemon.rs` —
  the daemon spools exits when given a directory.
- `src-tauri/src/lib.rs`, `sessions.rs`, `terminal.rs`, `activity.rs`, `workspaces/commands.rs` —
  app wiring, IPC commands, the activity settings.
- `src/features/activity/`, `src/stores/activity.ts`, `src/features/settings/ActivitySettings.tsx`,
  `src/features/workspace/WorkspacePanel.tsx` — the experimental timeline and its switch.
- `crates/cli/src/commands/activity.rs`, `doctor.rs`, `yardsort.rs` — `ys activity list|export`,
  spool draining, doctor rows.
- Docs: [guide/activity](../guide/activity.md), settings, terminals & sessions, CLI, README,
  changelog, 03/05/06/08/09 design docs, website nav.

## 8 · Verification

Run on 2026-09-24, Linux (Arch, kernel 7.2, Rust 1.98, bun 1.4). Every suite is the repository's
own gate: `just check` (formatting, clippy with warnings as errors, eslint, tsc, all tests) and
`just bindings-check`.

| What                                                                                | Where                                                                                                         | Result |
| ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ------ |
| Run written before spawn, PTY attached after, record linked after                   | `core/src/activity.rs`, `core/src/launch.rs` (plan-catching host)                                             | pass   |
| Ids in the child environment, appended not replacing                                | `launch.rs::a_workspace_launch_is_a_recorded_run…`                                                            | pass   |
| App and CLI launches (`launched_by`), shell and program runs                        | `launch.rs`, `cli/tests/cli.rs`                                                                               | pass   |
| Resume and fork through the same recorded path                                      | `activity.rs::resumes_and_forks…`, `src-tauri/src/sessions.rs` wiring                                         | pass   |
| Two workspaces at once keep runs and exits apart (real PTYs)                        | `launch.rs::two_workspaces_keep_their_runs_and_exits_apart`                                                   | pass   |
| Immediate exit settled for record and run, once (real PTY)                          | `launch.rs::a_process_that_exits_at_once…`                                                                    | pass   |
| Spawn failure recorded and still an error                                           | `launch.rs::a_program_that_cannot_start…`                                                                     | pass   |
| Duplicate delivery: live, settle, spool, reconcile → one event                      | `activity.rs::an_exit_is_recorded_once…`                                                                      | pass   |
| Closed window: daemon spools the exit after its only client left                    | `pty-ipc/tests/daemon.rs` (real daemon process, real socket)                                                  | pass   |
| Spool import settles run and session, deletes files, counts junk                    | `activity.rs::a_spooled_exit_settles…`                                                                        | pass   |
| Spool cap drops and counts; oversized, corrupt, future entries                      | `pty-ipc/src/spool.rs` tests                                                                                  | pass   |
| Reconciliation at start: gone → interrupted, young pending left alone               | `activity.rs::runs_whose_process_is_gone…`                                                                    | pass   |
| Broken table: launch proceeds, failure counted                                      | `activity.rs`, `launch.rs::a_broken_activity_table…`                                                          | pass   |
| Retention, clear, cascade with workspace, forgotten record                          | `activity.rs` (4 tests)                                                                                       | pass   |
| `ys activity list/export`, `ys session list` drains the spool, doctor               | `cli/tests/cli.rs` (2 tests, real binary)                                                                     | pass   |
| Timeline paging, clearing, load error; settings; footer toggle                      | `src/features/activity/*.test.tsx`, `settings/ActivitySettings.test.tsx`, `workspace/WorkspacePanel.test.tsx` | pass   |
| By hand: `ys workspace new` → exit unattended → daemon idles out → next `ys` drains | throwaway profile, shell as agent; `exit 5, via spool`, record `ended`, spool empty                           | pass   |

Not exercised here: macOS and Windows by hand. CI runs every Rust suite above, including the
real-daemon spool test, on both; the hands-on rows are [08 §11](08-manual-checklist.md#11--activity).
No screenshot was retaken: the timeline is behind an experimental switch and the existing shots
do not show the footer button.

## 9 · Remaining risks and the next slice

- **Spool on a full disk** degrades to today's behaviour (the exit is announced live if a client
  is there, else lost and later marked interrupted) and counts a drop; it cannot stall the pump,
  because the write is a small file and errors are swallowed.
- **Two clients importing at once** (app and `ys`) can both read the same spool file; the unique
  source key makes the second a counted duplicate.
- **A daemon started by an older build** never spools. Detectable in `ys doctor` (spool path
  present, never any files) and self-healing once the daemon is replaced.
- **Windows and macOS** were not exercised by hand in this pass; CI runs the Rust suites,
  including the real-daemon spool test, on all three. The manual rows are in 08 §9.
- **Next:** Stage 2, Claude Code first — record real hook payloads for 2.1.x as fixtures under
  `crates/core/fixtures/claude-hooks/`, then an opt-in, reversible hook installer that chains
  existing hooks, a local receiver in the persistent process, and correlation through
  `YARDSORT_RUN_ID` with the native session id as the fallback key.
