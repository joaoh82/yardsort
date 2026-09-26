# 17 — Agent events, stage 3: review and provenance

Status: **two slices shipped** · 25–26 September 2026 · Stage 3 of
[09](09-agent-events-and-memory.md): _join event ranges to workspace diffs and Jev assessments;
show evidence and uncertainty in a review panel_, with the exit gate _review can distinguish
reported writes from Git-observed changes; no claim of line-level causality without exact
evidence_. This slice builds the join and the words for it, on the panel that already exists.
It does not touch what Assist is sent.

## 1 · What there was to join

Stage 2 left every built-in harness reporting, but not one of them reporting the same way about a
file. What the store holds, per producer, when an agent writes:

| Producer    | Kind                                         | The file                                                    |
| ----------- | -------------------------------------------- | ----------------------------------------------------------- |
| `claude`    | `tool.completed` with `tool`                 | `path`, for `Write`, `Edit`, `MultiEdit`, `NotebookEdit`    |
| `pi`, `omp` | `tool.completed` with `tool`                 | `path`, for `write`, `edit`                                 |
| `codex`     | `file.reported_write`                        | `path`, one event per file of a `FileChange` item           |
| `opencode`  | `file.reported_write` _and_ `tool.completed` | `path` on both: the plugin sees the tool and the file event |
| `cursor`    | `file.reported_write`                        | `path`, from `afterFileEdit`                                |
| `grok`      | `tool.completed` with `tool`                 | **none** — its log names the tool and never the file        |

Every `path` is already workspace-relative, and a path outside the workspace was never recorded
(`pathOutsideWorkspace: true` in its place). So the join needs no path handling at all, and no
knowledge of where the worktree is: the Changes panel's `FileChange.path` is the same string.

On the other side, the Changes panel lives in the app (`src-tauri/src/changes`), not the core:
git's answer, two scopes, refreshed on every file-system signal. The CLI has no diff. Assist's
review sits beside it, per file, advisory, with a note above the list — the shape this slice
copies.

## 2 · Decisions

- **Joined at read time; nothing stored.** `activity::provenance::of(store, workspace)` reads
  the three kinds that can carry a write (`file.reported_write`, `tool.completed`,
  `process.started`) and folds them, per path and per run, into a `Report`: who (the run's
  harness, the event's producer, method and fidelity), when (first and last), and how many
  times. The panel joins that to git's list by path. No table, no migration, and nothing to
  keep consistent when the user commits, reverts or clears activity.
- **`workspace.changed` stays unrecorded.** Stage 1 deferred it to here "alongside the review
  join". With the join computed from git's answer at the moment of reading, a row per
  file-system signal would only restate the Changes panel inside the store, at the watcher's
  best-effort cadence. Not built.
- **Which events count as a write is a table, not a guess.** `file.reported_write` is the
  contract's own kind and always counts. For the adapters that report tool calls instead, the
  harness's writing tools are named per producer (above); a tool the table does not know is
  not a write, and a write with no path is no claim about any file. OpenCode's double report
  is one write: only its `file.reported_write` is counted. A report that says the change did
  not land is not a write either: Codex writes a `FileChange` item's `status` through, and a
  failed patch against a file the user changed must not badge that file as Codex's.
- **Coverage is part of the answer.** The join also lists the workspace's agent runs and how
  each was asked to report (`capture` from its `process.started`, or nothing). That is what
  lets the panel say _why_ a changed file has no report — a run that was not reporting — and
  what keeps it silent when no run was: with no reporting run, every file is unreported for the
  same uninteresting reason, and the list reads as it always did. The start event is not the
  only witness, because it can be gone while the run is not: **Clear** during a live run, or
  retention, removes it and keeps the run and the reports that follow. So a run with no start
  event is known by what it reported through — capture is named by the events' `method` — and
  a run with neither is `capture_known: false`: unknown, counted as neither reporting nor
  silent.
- **The words never reach a line.** A badge names the agent; its tooltip says how many times it
  reported writing the file, when it last did, and through what, then: _Git shows every change
  since the last commit; which of those lines came from that report is not known._ The note
  above the list counts files with a report and files without one, and names the alternatives
  for the latter: you, a script, or a run that was not reporting. Nothing joins an event's time
  to a hunk; no adapter records one.
- **The trace link runs both ways.** A timeline row that names a file on the change list gets a
  **Show diff** button that opens that file's diff in the Changes panel. The button's own words say
  the diff is everything that changed since the last commit, whoever changed it.
- **Assist is not changed.** Stage 3's Jev row asks for "selected event-derived facts" beside
  the diff. That changes what is sent to a third party under the user's key, which is an
  opt-in decision of its own and a design pass on what those facts are; it is not folded into a
  slice whose point is honesty about evidence. Recorded as the next step, not done.

## 3 · What shipped

- `crates/core/src/activity/provenance.rs`: `Report`, `FileReports`, `RunCoverage`,
  `Provenance`; `join(runs, events, native)` pure, `of(store, workspace)` over two new store
  queries: `events_of_kinds`, which reads only the kinds that can carry a write, and
  `native_methods_by_run`, which says which runs have events of the agent's own and through
  what.
- `workspace_provenance` command and DTOs (`src-tauri/src/activity.rs`);
  `ipc.workspaceProvenance`; a `provenance` store that follows the Changes panel's workspace
  and refreshes on every file-system signal and every `activityChanged`.
- Changes panel: a badge per reporting agent on each changed file, a line above the list, and
  a word in the expanded viewer's header — _reported by claude · 2 writes · 14:02_ or _no agent
  reported writing this_. Activity panel: the **Show diff** button.
- Guides: [changes and files](../guide/changes-and-files.md#who-wrote-it),
  [activity](../guide/activity.md#the-timeline-experimental); checklist
  [08 §18](08-manual-checklist.md#18--reported-writes-beside-the-diff).

## 4 · Verification

- `activity::provenance` tests (9): Claude's reading tool and command are not writes and its
  `Write` and `Edit` are two; OpenCode's tool call beside its file event is one write and Codex's
  a second row for the same file; a Codex report with `status: "failed"` is not a write; Grok's
  `write` without a path claims nothing; the pi family's `write`/`edit`; coverage lists spawned
  agent runs oldest first with their capture and leaves shells and never-spawned runs out; a run
  whose start event was cleared is known by what it reported through, and one with nothing is
  unknown; an unlinked report keeps its file and has no run; only the three kinds are read.
- `ChangesPanel.test.tsx` (3): the badge, its tooltip's count, source and caveat, the note's
  arithmetic, the expanded header's two wordings; nothing shown while no run was reporting; a
  report landing refreshes the join for the followed workspace only.
- `ActivityPanel.test.tsx` (1): the **Show diff** button on a row naming a listed file, absent on
  rows that do not, absent when the Changes panel follows another workspace, and what it opens.
- Live on Linux, 2026-09-26, Claude Code 2.1.280 through the dev build: asked plainly to
  create a file, it ran one Bash command — a tool and a duration reported, no file, nothing
  badged, and the line above the list naming a command the agent ran (§5). Asked to use its
  Write tool for a second file, the `Write done · hello4.txt` row landed with its **diff**
  button, that file gained the **claude** badge without a refresh, the shell-made file did not,
  and the line read _Agents reported writing 1 of 2 changed files_. Checklist
  [08 §18](08-manual-checklist.md#18--reported-writes-beside-the-diff) rows 1, 2 and the
  button of 4 seen; the rest of the pass is owed.

## 5 · What this slice does not do, and the next

- **Jev with event-derived facts** (the Assist join above): the next slice of stage 3, with its
  own opt-in.
- **Committed scope.** Reports are joined to a path whether it is uncommitted or on the branch;
  a file the agent wrote and then committed keeps its badge in the committed group. Reports
  are not attributed to a commit, and a file written by one run and committed by another says
  both.
- **Retention.** Reports come from the store, so they age out with it — the newest 20 000
  events and 90 days, see [Keeping it small](../guide/activity.md#keeping-it-small); a diff older
  than the retained activity has no report, and the note says so in the same words as any other
  unreported file. Whether that case deserves its own words is left to seeing it.
- **A shell command is not a reported write**, and that is the common case for a small file.
  Found on the first hands-on pass: asked to "create hello.txt", Claude Code ran one Bash
  command, which reports a tool and a duration and no file — so no row named the file and
  nothing was badged, correctly. Reading the command to find the file is exactly what the
  contract forbids. The second slice (§6) answers it another way.
- **Grok** cannot contribute: its log has no path. The note counts its runs as reporting, since
  they were; its files are simply never badged. Honest, and worth a line in the guide.

## 6 · Second slice: observed writes

The file the agent made with `echo hello > hello3.txt` has a witness after all: its own
modification time. Every adapter bounds a tool call — a `tool.started` and a `tool.completed`
or `tool.failed` for the hook, plugin, extension and Grok adapters; a `durationMs` on the
completion for Codex's session file and Cursor's post-hooks — and a file whose last write falls
inside such a window was written while that tool ran. That is an _observation_: Yardsort read
the time on the file and the times on the timeline, not who wrote the file. It is worded and
styled as one, and ranked below a report.

- **Only a tool call is a window.** A run as a whole is not, though the run's start and end
  are known: an agent writes through its tools, so a file written while it sat idle is more
  likely the user's, and a run window would badge every file a user touched while an agent was
  open — with lifecycle recording on by default, that is every user. With no capture there are
  no tool windows and nothing changes.
- **A tool that named its own file is a window for that file alone.** `Write hello4.txt`
  cannot vouch for `other.txt` written in the same instant; a command, which names nothing,
  can vouch for anything.
- **Two agents at once decide nothing.** Every window containing the time is listed; the badge
  then says _2 agents_ and the tooltip names both.
- **The clock is the file's, not a watcher's.** The `workspace.changed` event stays unrecorded
  for a third reason: the app's watcher is debounced and best-effort, and it is not there while
  the window is closed. `mtime` is exact, is there afterwards, and costs one `stat` per changed
  file. The command takes the change list's paths from the panel and stats them inside the
  workspace (`resolve_inside`); a deleted or renamed-away file has no time and is left out.
- **A report outranks an observation.** A file with both shows its report; the observation is
  in the data for the row and in the tooltip's absence.
- **Words.** Badge: the agent's name in a dashed border. Tooltip: _This file was last written
  at 07:57:46, while claude was running Bash (07:57:46 to 07:57:47). Yardsort read the time on
  the file, not who wrote it: you or a script could have written it in that window, and no
  agent reported it._ Note: _One was last written while an agent ran a command — seen on the
  file's clock, not reported._ Header: _last written while claude ran Bash · 07:57_.

Limits, stated: a file the agent made with a command and the user then edited carries the
user's time and no badge — `mtime` is the last write only. A command that runs for minutes is
a wide window, and everything written in it is attributed to it. Checkouts and formatters the
agent runs through a command are, correctly, "while claude ran Bash". Tests: `activity::provenance`
gains three (a command's window names the file with the tool and leaves a later write alone; a
file tool's window is for its file only and meets its report on one row; pairs, durations, open
starts, two agents at once, and never a shell); `last_written` in the app reads the file's clock
and skips what it cannot; `ChangesPanel.test.tsx` gains one for the badge, the tooltip, the
note's arithmetic, the header, and the report outranking the observation.
