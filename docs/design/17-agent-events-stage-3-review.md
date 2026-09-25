# 17 — Agent events, stage 3: review and provenance

Status: **first slice shipped** · 25 September 2026 · Stage 3 of
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
  **diff** button that opens that file's diff in the Changes panel. The button's own words say
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
  reported writing this_. Activity panel: the **diff** button.
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
- `ActivityPanel.test.tsx` (1): the **diff** button on a row naming a listed file, absent on
  rows that do not, absent when the Changes panel follows another workspace, and what it opens.
- Live, first pass on Linux: Claude Code's hooks reported the session, the prompt, a Bash
  tool and the turn — and no file, because the file was made by the command (§5). The
  checklist now asks for the Write tool by name.

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
  nothing was badged, correctly. The note above the list now names _a command the agent ran_
  among the alternatives, and the guide says a missing badge is not a change the agent did not
  make. Reading the command to find the file is exactly what the contract forbids; the gap
  stays.
- **Grok** cannot contribute: its log has no path. The note counts its runs as reporting, since
  they were; its files are simply never badged. Honest, and worth a line in the guide.
