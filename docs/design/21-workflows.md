# Workflows

_Proposal, 2026-09-28. Decisions from the design pass are recorded; nothing here is built yet._

A workflow is a named, reusable sequence of things Yardsort does with agents on your behalf —
"request a code review of this workspace's pull request" — written in YAML, shown as a flow
chart, started from the app or from `ys`. Version one has one trigger, **manual**. The model is
built so that schedules and forge events can become triggers later without changing what a
workflow _is_.

## Decisions

Eight questions were put to the user on 2026-09-28. The answers, and what follows from each:

1. **Workflows live in the user's data directory, not in repositories.** `<data dir>/workflows/*.yaml`,
   plus built-ins compiled into the core. A cloned repository must never define what gets sent to
   your agents — the same line [project automation](03-architecture.md) drew: repository files
   never enable commands. A user file whose `id` matches a built-in replaces it; deleting the file
   restores the built-in. Per-repository workflows with an explicit trust step are a later item.
2. **The sidebar gets a WORKFLOWS section above PROJECTS**, in the same shape as the project
   tree: a header with **+**, then one row per workflow. Clicking a row opens it in the main
   panel. [02-ux](02-ux.md) said the bottom of the panel is where future top-level features go;
   this is the first one to go at the top instead, because a workflow is something you _start_,
   like a workspace, not something you configure, like Settings.
3. **The app runs workflows. `ys` needs the app open.** The engine is in `yardsort-core`, driven
   from the app's Rust side. `ys workflow run` writes a queued run and tells the app through the
   activity inbox the app already watches. With no app running, `ys` says so and creates nothing.
   One engine, so no two processes ever advance the same run; notifications, PR polling and the
   PTY events a run waits on already live in the app. The daemon stays storage-free and tiny.
4. **The reviewer works in the PR's own workspace, as a new session tab.** It sees the exact tree,
   can run the tests, and you can watch it or take over. It shares the worktree with the author's
   session, so the built-in prompt tells it to read, run, and post — never to edit.
5. **A model writes workflows through the Drafting path**, never Jev. Jev answers typed questions
   and cannot author YAML ([open question 19](06-open-questions.md)). Drafting already has the
   route: the project's harness in its non-interactive mode, an Anthropic key as fallback. The
   result is validated and opened in the editor; nothing is saved until the user says so.
6. **Steps form a DAG from day one.** Every step may name the steps it `needs`; steps whose needs
   are all done run together. The validator rejects cycles, unknown ids and unknown variables. The
   built-ins are chains, so the first flow charts are vertical, but the engine, the schema and the
   chart layout never assume that.
7. **"The review is done" is PTY quiet or `turn.completed`, then the forge.** The reviewer's step
   ends when its session settles; a `wait_pr_activity` step then polls `gh` until a review or
   comment newer than the run exists. Only then does the user get notified and the author's session
   told. Nothing an agent prints is parsed.
8. **The reviewing agent posts to GitHub itself, with `gh`.** Its prompt says to run `gh pr review`
   and `gh pr comment`. Yardsort only checks that activity appeared. Yardsort's own write path to
   the forge stays what it is today: opening a pull request.

## The file

The built-in, exactly as `crates/core/src/workflow/builtin/code-review.yaml` has it:

```yaml
# Built into Yardsort. To change it, duplicate it: a file in your workflows folder with the same
# id is used instead of this one, and removing that file brings this one back.
id: code-review
name: Request code review
description: >
  A second agent reviews this workspace's pull request and posts its review on GitHub.
  When the review is there, you are told, and so is the agent that wrote the pull request.
version: 1

trigger:
  kind: manual
  context: workspace

inputs:
  - id: reviewer
    kind: harness
    label: Who reviews
    required: true
  - id: focus
    kind: text
    label: Anything to look at in particular

steps:
  - id: review
    action: start_session
    harness: "{{ inputs.reviewer }}"
    prompt: |
      Review pull request #{{ pr.number }} ({{ pr.url }}), "{{ pr.title }}". It is the branch
      `{{ workspace.branch }}`, based on `{{ workspace.base_branch }}`, and you are in its
      worktree. Another agent wrote it. Its task was:

      {{ workspace.task }}

      {{ inputs.focus }}

      Read the whole diff against `{{ workspace.base_branch }}` and run the project's checks.
      Do not edit, commit or push anything: the author is working in this same folder.

      When you are done, post your review on the pull request with `gh pr review
      {{ pr.number }}` — approve, comment or request changes — and put each finding that is
      about particular lines on those lines. Say what is wrong, why, and how you know. Then stop.

  - id: settled
    action: wait_session
    needs: [review]
    session: "{{ steps.review.session }}"
    until: settled
    timeout: 2h

  - id: posted
    action: wait_pr_activity
    needs: [settled]
    kind: any
    timeout: 15m

  - id: tell_user
    action: notify
    needs: [posted]
    title: "Review posted on #{{ pr.number }}"
    body: "{{ inputs.reviewer }} reviewed {{ workspace.name }}. The pull request has it."

  - id: tell_author
    action: send_to_session
    needs: [posted]
    session: origin
    timeout: 1h
    prompt: |
      Pull request #{{ pr.number }} has a new review from another agent. Read it with
      `gh pr view {{ pr.number }} --comments` and the review comments with
      `gh api repos/{owner}/{repo}/pulls/{{ pr.number }}/comments`. Fix what is right, push,
      and answer each comment saying what you did or why you did not.
```

The last two steps both need `posted` and nothing else, so they run together — the first parallel
branch, in the first built-in.

### Schema

| Key           | Rule                                                                                                                                                                     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`          | `[a-z0-9-]+`, unique across built-ins and user files; the file name should match                                                                                         |
| `name`        | Required, shown in the sidebar                                                                                                                                           |
| `description` | Optional, shown under the name                                                                                                                                           |
| `version`     | The schema version, `1`. A file with a higher version than the app knows is listed, not runnable                                                                         |
| `trigger`     | `kind: manual` only in v1; `context: workspace`, the default and the only value in v1, says what the run is about and which variables exist                              |
| `inputs[]`    | `id`, `kind` (`harness`, `text`, `choice` with `options`), `label`, `required` (default `false`), `default`. Collected before the run starts, in the app or as `--input` |
| `steps[]`     | `id`, `action`, `needs` (step ids), then the action's own keys. A step id is `[a-z0-9_-]+`                                                                               |

### Actions in v1

| Action             | What it does                                                                                                                                       | Ends when                                                | Produces              |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- | --------------------- |
| `start_session`    | Starts a harness in the run's workspace through the shared launcher, with a first message. `harness`, optional `model`, `effort`, `skip_memory`    | The process has been spawned                             | `session`, `run`      |
| `wait_session`     | Watches one session. `until: settled` is PTY quiet past the attention threshold or a `turn.completed` event; `until: exited` is the process ending | The condition holds, or `timeout` → step fails           | `outcome`             |
| `send_to_session`  | Pastes a message into a live session — one this run started, or `origin` — once it is quiet. There is no key to change that                        | The paste was written, or the session was gone → skipped |                       |
| `wait_pr_activity` | Polls the forge for a review or comment on the workspace's pull request newer than the run started. `kind: review \| comment \| any`               | Activity exists, or `timeout` → step fails               | `count`, `latest_url` |
| `notify`           | An OS notification when the window is unfocused, a toast in the app otherwise; also a line in the run's log                                        | Immediately                                              |                       |

Every action is a plain description of intent. None takes a shell string; `start_session` goes
through the same argv-building launcher as the composer and `ys workspace new`, so hooks are armed
and the run is recorded in activity like any other.

### Variables

`{{ path }}` with dotted lookup, nothing else — no filters, no logic. Whitespace inside the braces
is ignored. A `{{` that is not a known variable is a validation error, so a typo cannot reach an
agent as literal text. `\{{` writes a literal `{{`. Names are checked when the file is loaded,
against the namespaces the trigger's `context` allows:

| Namespace   | Fields                                                                                                          | Available            |
| ----------- | --------------------------------------------------------------------------------------------------------------- | -------------------- |
| `project`   | `name`, `root`                                                                                                  | always               |
| `workspace` | `name`, `branch`, `base_branch`, `path`, `task` (its first message, as outcomes record it)                      | `context: workspace` |
| `pr`        | `number`, `url`, `title` — the open pull request whose head is the workspace's branch; a run needs one to start | `context: workspace` |
| `inputs`    | One field per declared input, by `id`. An optional input with no value renders empty                            | always               |
| `steps`     | `<step id>.<produced field>`; only steps named in `needs`, transitively                                         | always               |
| `memory`    | The project's approved memory, rendered as the launcher renders it, or empty when the project does not share it | always               |
| `handoff`   | The workspace's handoff packet, as `ys workspace handoff` prints it                                             | `context: workspace` |

A step's `session` is not a variable but a reference: `origin`, the workspace's most recent live
harness session, or exactly `{{ steps.<id>.session }}` for an earlier `start_session` step among
its needs. A `harness` is a harness id or exactly one `{{ inputs.<id> }}` of kind `harness`.

## The run

A run is rows, not a process. Migration `0012_workflows.sql`:

- `workflow_runs`: id, workflow id, a **snapshot of the YAML** as it was when the run started (a
  run must not change under an edit), project id, workspace id, `status`
  (`queued`, `running`, `succeeded`, `failed`, `cancelled`), `requested_by` (`app`, `cli`), the
  resolved inputs as JSON, `created_at`, `started_at`, `ended_at`, `error`.
- `workflow_step_runs`: run id, step id, `status` (`pending`, `ready`, `running`, `waiting`,
  `succeeded`, `failed`, `skipped`), `started_at`, `ended_at`, the produced fields as JSON, `error`.

Nothing about a run blocks the app. A step that waits — for an agent to settle, for a review to
appear — is a row marked `waiting`, not a thread asleep; the window stays a view that reads rows
and refreshes on an event, and any number of runs can be in flight at once.

The engine in `yardsort_core::workflow::engine` is a function, not a thread:
`advance(run, world) -> Vec<Effect>`, which returns in milliseconds. It reads the two tables and a small view of the world
(session activity states, `turn.completed` events since a timestamp, forge activity, the clock)
and returns what should happen next: `Spawn`, `Paste`, `PollForge`, `Notify`, `Mark(step,
status)`. That makes the whole state machine testable with a fake world and real SQLite in a temp
directory — no PTY needed to prove that a cycle never runs, that a failed need skips its
dependents, that a timeout fails a step exactly once.

The driver in `src-tauri/src/workflows.rs` owns the effects. It wakes on: a new queued run (an
inbox file from `ys`, or the app's own command), every `PtyHostEvent` quiet or exit, every
`ActivityChanged`, and a 15-second timer for the forge polls and timeouts. On each wake it loads
the active runs, calls `advance`, executes the effects with the launcher, the terminal host, `gh`
and the notification path, writes the rows, and emits a `WorkflowChanged` event the frontend uses
to refresh. The driver is a background task on the Rust side, never the UI thread; calls that can
take seconds — `gh`, a spawn — go to blocking workers, the way pull-request polling already does,
so a slow forge never delays the handling of a terminal event. A run whose app quit mid-way is found `running` on the next start; steps that were
`waiting` resume waiting, steps that were `running` a spawn are checked against the activity
table (the run row exists before the spawn, as it does everywhere) and either continue or fail.

Rules the engine enforces regardless of what a file says:

- **A busy agent is never written to.** `send_to_session` waits for quiet, and `origin` resolves
  to nothing when there is no live session, in which case the step is `skipped`, not failed, and
  the run says so.
- **One run per workspace per workflow at a time.** Starting a second is refused with the first's
  id, in the app and in `ys`.
- **Cancel stops the run, not the agents.** Sessions a run started keep running as ordinary
  sessions; the run's remaining steps become `cancelled`. Nothing a workflow starts is ever
  killed without the same confirmation an ordinary session gets.

## The command line

```
ys workflow list                          # built-ins and yours, with where each comes from
ys workflow show <id>                     # the YAML, or --json the parsed form
ys workflow validate <file>               # exit 0 or the errors, one per line, with line numbers
ys workflow run <id> --workspace <name> [--input reviewer=claude --input focus="…"]
ys workflow runs [--workspace <name>] [--run <id>]   # history, or one run step by step
ys workflow cancel <run>
```

`run` resolves the workspace and the pull request, checks every required input is given (the
missing ones are listed by id and label — there is no prompt in the terminal), refuses if the app
is not running, inserts the `queued` row and drops a file in the inbox. The app's presence is a
lock file the app holds open under the data directory; the mechanism is verified against all three
platforms in the first slice, because a stale file after a crash must not read as "running". An
agent inside a workspace can run `ys workflow run code-review` with no `--workspace`, found from
the environment the way `ys memory propose` finds its workspace today.

## The UI

**Sidebar.** `WORKFLOWS` above `PROJECTS`, one row per workflow, a count of running runs on the
row, **+** for a new one. Rows are the store's array; anything derived is derived outside the
selector.

**Main panel, `view: workflows`** in the layout store, holding the selected workflow. Three parts:

- **Chart.** The steps as nodes, `needs` as edges, laid out top-down. Each node shows the action,
  the id and, when a run is selected, that step's status and timing. React Flow (`@xyflow/react`)
  renders and pans; `@dagrejs/dagre` lays out. The chart is read-only in v1 — the YAML is the
  source of truth and the chart is a view of it, which is what keeps the frontend from holding
  truth. Dragging nodes to edit YAML is a later item.
- **YAML.** A CodeMirror editor (the YAML mode is already in `language-data`) with the validator's
  errors as gutter marks, from the core's `workflow_validate` command on every pause in typing.
  **Save** writes the file through the core. A built-in shows read-only with **Duplicate**, which
  copies it under a new id; a user file that shadows a built-in has **Reset to built-in**.
- **Runs.** Every run of this workflow, newest first, each expanding to its steps; a running one
  has **Cancel**. Selecting a run colours the chart.

**Run…** opens a dialog: pick the workspace (or it is preselected from the workspace's own menu
and toolbar, where **Request code review** also appears directly), fill the inputs (a `harness`
input is the same picker the composer uses), see the resolved pull request, start.

**Describe it.** **+** offers a blank file or a description box. The description goes to Drafting
with a system prompt that carries the schema, the actions table and one built-in as an example;
the reply is validated, and if it fails once, sent back with the errors for a single retry. The
result opens in the editor unsaved. This is the only place a model writes a workflow, and it never
runs one.

## Slices

Each is one pull request with its docs, tests and changelog line, in this order. Each stands on
its own.

1. **The file and the core.** ✅ Schema types, YAML loading, the validator with line numbers,
   the variable checker, the built-in `code-review`, the user directory, the guide for the file
   format, and `ys workflow list | show | copy | validate`. See
   [slice 1](#slice-1-what-shipped) below.
2. **The engine and the driver.** Migration 0012 and the run store, moved here from slice 1: a
   migration ships once and cannot be edited, so it waits for the code that writes its rows.
   `advance` with a fake world; the driver with real sessions in a
   temp data directory: a workflow that starts a shell-backed "harness", waits for it to settle,
   pastes into it, notifies. `ys workflow run | runs | cancel`, the inbox handoff, the app
   presence check on the three platforms. Resume after the app restarts.
3. **The forge step and the built-in end to end.** `wait_pr_activity` over `gh pr view --json
reviews,comments` (and `gh api` for review threads), the `pr` variables and resolving
   `session: origin`. The code-review workflow run against a real PR on a throwaway
   repository, on each platform, recorded in [08](08-manual-checklist.md).
4. **The UI.** Sidebar section, the workflows view, chart, editor, runs, the run dialog, the
   workspace menu entry. Testing Library for the list, the dialog's input rules and the run
   history; the chart's layout as a pure function with tests of its own.
5. **Describe it.** `Want::Workflow` in Drafting, the `draft_workflow` command, the validate-and-
   retry loop, the editor hand-off. Tests with a fake writer that returns bad YAML once.
6. **Docs and pictures.** The guide's sections on running (the file format's went with slice 1),
   README highlights, the changelog, screenshots of the chart and the run dialog with the throwaway profile.

Everything in slices 1–3 works with no window, which is what makes triggers other than manual a
matter of adding a producer of queued runs later.

## Slice 1: what shipped

`yardsort_core::workflow` reads and checks files; nothing runs yet. What changed from the proposal
above, and why:

- **The YAML crate is `serde-saphyr`.** `serde_yaml` is archived and `serde_yml` is deprecated.
  `serde-saphyr` is maintained, has no unsafe parser underneath, and has `Spanned<T>`, which keeps
  the line and column of any value through deserialising. That is what puts a line on a mistake
  deep in a file: an unknown variable on the third line of a block-scalar prompt is reported at
  that line and column, found by searching for its text from where the value starts, because
  YAML folding moves offsets. serde's own errors (shape, types, unknown and duplicate keys) come
  first and stop at the first; everything else is collected and reported sorted.
- **Every step is one raw struct with every key optional,** then checked against its action. An
  internally tagged enum would have been shorter, but serde buffers such values and loses the
  spans, and a key on the wrong action would read as unknown rather than misplaced.
- **`send_to_session` has no `when`.** The only value that was ever safe was `quiet`, and the
  engine enforces that whatever a file says, so the key would only have been a way to be wrong.
- **`origin` is a value of `session`, not a variable,** and `session` is `origin` or exactly
  `{{ steps.<id>.session }}`. A session reference inside prose meant nothing.
- **Only `context: workspace`.** `project` had no answer for where a `start_session` would run,
  so it waits for the triggers that need it.
- **A file that fails to parse still has an id**: its own when YAML is well-formed, else its file
  name. So a broken `code-review.yaml` replaces the built-in and shows its problems, instead of
  the built-in running in its place without a word. Two files with one id: the first by name is
  used and the second is listed with the clash.
- **A file from a later schema version** is reported as such before its unknown keys are, by a
  lenient first read of `version`.
- **`ys workflow validate` needs no profile,** so it runs in CI or on a file an agent has just
  written, and takes `-` for standard input.
- **`ys workflow copy <id> [--as <new-id>]`**, added after review. The guide first said to copy a
  built-in with `ys workflow show code-review > <folder>/code-review.yaml`, which cannot work: the
  shell makes the empty file before `ys` runs, the catalog finds it claiming the id, and `show`
  prints it instead of the built-in. Windows PowerShell's `>` writes UTF-16 besides. `copy` is
  `workflow::copy` in the core, so the app's **Duplicate** in slice 4 is the same code; it never
  overwrites.
- **Also from review:** a timeout's unit is matched as a suffix, not sliced by byte count, which
  panicked on a multi-byte last character; and a mistake's position is found by _occurrence_ —
  the second `{{ pr.nubmer }}` in a value is the second in the file, and an escaped `\{{` is
  counted in neither — where it used to point every repeat, and an escaped literal, at the first.

## Not in v1, on purpose

- Schedules and forge-event triggers. The roadmap's "automations" item lists the questions those
  raise (a run while the previous one is going, a worktree per run, permission with no one to
  ask); a manual workflow answers none of them and does not need to.
- Editing the chart by hand; per-repository workflow files; conditionals, loops, `if:`; step
  retries; a step that runs a program (it would be a shell string by another name — project
  automation is where commands live, and a `run_automation` step can call it later).
- Reviewing in a fresh worktree; headless reviewers.
- Jev's judgment of a review, or of a workflow a model wrote.

## Open questions

[21–23 in open questions](06-open-questions.md#workflows): how `ys` knows the app is running,
what `settled` means without native events, and which harnesses a `harness` input offers.

## Documentation this touches

`docs/guide/workflows.md` (new), `docs/guide/cli.md`, `docs/guide/terminals-and-sessions.md`
(notifications from a run), `docs/guide/assist.md` (Describe it is Drafting, not Assist),
`docs/README.md`, `website/src/lib/docs.ts`, `README.md`, `CHANGELOG.md`, [02-ux](02-ux.md) (the
left panel now has a section above projects), [03-architecture](03-architecture.md) (the engine
and the driver), [05-roadmap](05-roadmap.md) (M22), [06-open-questions](06-open-questions.md)
(21–23), [08](08-manual-checklist.md) (§22).
