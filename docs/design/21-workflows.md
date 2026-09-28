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

```yaml
# <data dir>/workflows/code-review.yaml
id: code-review
name: Request code review
description: >
  A second agent reviews this workspace's pull request and posts its findings on GitHub.
  When it has, you are told, and so is the agent that wrote the PR.
version: 1

trigger:
  kind: manual
  context: workspace # runs against one workspace; `project` is the other value

inputs:
  - id: reviewer
    kind: harness # harness | text | choice
    label: Who reviews
    required: true
  - id: focus
    kind: text
    label: Anything to look at in particular
    required: false

steps:
  - id: review
    action: start_session
    harness: "{{ inputs.reviewer }}"
    prompt: |
      You are reviewing pull request #{{ pr.number }} ({{ pr.url }}) for the branch
      `{{ workspace.branch }}`, based on `{{ workspace.base_branch }}`. Another agent wrote it;
      its task was:

      {{ workspace.task }}

      {{ inputs.focus }}

      Read the diff against the base and run the project's checks. Do not edit any file. When you
      are done, post your review with `gh pr review {{ pr.number }}` (approve, comment or request
      changes) and put line-level findings on with `gh pr comment` or review comments. Then stop.

      {{ memory }}

  - id: settled
    action: wait_session
    needs: [review]
    session: "{{ steps.review.session }}"
    until: settled # settled (quiet, or turn.completed where hooks exist) | exited
    timeout: 45m

  - id: posted
    action: wait_pr_activity
    needs: [settled]
    kind: any # review | comment | any
    timeout: 10m

  - id: tell_user
    action: notify
    needs: [posted]
    title: "Review posted on #{{ pr.number }}"
    body: "{{ inputs.reviewer }} reviewed {{ workspace.name }}. Open the PR to read it."

  - id: tell_author
    action: send_to_session
    needs: [posted]
    session: origin # the workspace's live harness session that produced the branch
    when: quiet # never interrupt a working agent; wait for it to go quiet
    timeout: 30m
    prompt: |
      Pull request #{{ pr.number }} has a new review from another agent. Read the comments with
      `gh pr view {{ pr.number }} --comments` and `gh api`, address what is right, push, and reply
      to each thread saying what you did.
```

The last two steps both need `posted` and nothing else, so they run together — the first parallel
branch, in the first built-in.

### Schema

| Key           | Rule                                                                                                                                                   |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`          | `[a-z0-9-]+`, unique across built-ins and user files; the file name should match                                                                       |
| `name`        | Required, shown in the sidebar                                                                                                                         |
| `description` | Optional, shown under the name                                                                                                                         |
| `version`     | The schema version, `1`. A file with a higher version than the app knows is listed, not runnable                                                       |
| `trigger`     | `kind: manual` only in v1; `context: workspace \| project` says what the run is about and which variables exist                                        |
| `inputs[]`    | `id`, `kind` (`harness`, `text`, `choice` with `options`), `label`, `required`, `default`. Collected before the run starts, in the app or as `--input` |
| `steps[]`     | `id`, `action`, `needs` (step ids), then the action's own keys. A step id is `[a-z0-9_-]+`                                                             |

### Actions in v1

| Action             | What it does                                                                                                                                       | Ends when                                                | Produces              |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- | --------------------- |
| `start_session`    | Starts a harness in the run's workspace through the shared launcher, with a first message. `harness`, optional `model`, `effort`, `skip_memory`    | The process has been spawned                             | `session`, `run`      |
| `wait_session`     | Watches one session. `until: settled` is PTY quiet past the attention threshold or a `turn.completed` event; `until: exited` is the process ending | The condition holds, or `timeout` → step fails           | `outcome`             |
| `send_to_session`  | Pastes a message into a live session — one this run started, or `origin`. `when: quiet` (default) waits for the session to be quiet first          | The paste was written, or the session was gone → skipped |                       |
| `wait_pr_activity` | Polls the forge for a review or comment on the workspace's pull request newer than the run started. `kind: review \| comment \| any`               | Activity exists, or `timeout` → step fails               | `count`, `latest_url` |
| `notify`           | An OS notification when the window is unfocused, a toast in the app otherwise; also a line in the run's log                                        | Immediately                                              |                       |

Every action is a plain description of intent. None takes a shell string; `start_session` goes
through the same argv-building launcher as the composer and `ys workspace new`, so hooks are armed
and the run is recorded in activity like any other.

### Variables

`{{ path }}` with dotted lookup, nothing else — no filters, no logic. Whitespace inside the braces
is ignored. A `{{` that is not a known variable is a validation error, so a typo cannot reach an
agent as literal text. Names are checked when the file is loaded, against the namespaces the
trigger's `context` allows:

| Namespace   | Fields                                                                                                          | Available            |
| ----------- | --------------------------------------------------------------------------------------------------------------- | -------------------- |
| `project`   | `name`, `root`                                                                                                  | always               |
| `workspace` | `name`, `branch`, `base_branch`, `path`, `task` (its first message, as outcomes record it)                      | `context: workspace` |
| `pr`        | `number`, `url`, `title` — the open pull request whose head is the workspace's branch; a run needs one to start | `context: workspace` |
| `inputs`    | One field per declared input, by `id`. An optional input with no value renders empty                            | always               |
| `steps`     | `<step id>.<produced field>`; only steps named in `needs`, transitively                                         | always               |
| `memory`    | The project's approved memory, rendered as the launcher renders it, or empty when the project does not share it | always               |
| `handoff`   | The workspace's handoff packet, as `ys workspace handoff` prints it                                             | `context: workspace` |
| `origin`    | `session` — the workspace's most recent live harness session, or empty                                          | `context: workspace` |

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

1. **The file and the core.** Schema types, YAML loading through a maintained serde YAML crate
   (`serde_yaml` is archived; pick its maintained successor and record why), the validator with
   line numbers, the variable checker, the built-in `code-review`, the user directory, migration
   0012 and the store. `ys workflow list | show | validate`. Tests: every validation error has a
   fixture that fails without it.
2. **The engine and the driver.** `advance` with a fake world; the driver with real sessions in a
   temp data directory: a workflow that starts a shell-backed "harness", waits for it to settle,
   pastes into it, notifies. `ys workflow run | runs | cancel`, the inbox handoff, the app
   presence check on the three platforms. Resume after the app restarts.
3. **The forge step and the built-in end to end.** `wait_pr_activity` over `gh pr view --json
reviews,comments` (and `gh api` for review threads), the `pr` and `origin` variables,
   `send_to_session: origin`. The code-review workflow run against a real PR on a throwaway
   repository, on each platform, recorded in [08](08-manual-checklist.md).
4. **The UI.** Sidebar section, the workflows view, chart, editor, runs, the run dialog, the
   workspace menu entry. Testing Library for the list, the dialog's input rules and the run
   history; the chart's layout as a pure function with tests of its own.
5. **Describe it.** `Want::Workflow` in Drafting, the `draft_workflow` command, the validate-and-
   retry loop, the editor hand-off. Tests with a fake writer that returns bad YAML once.
6. **Docs and pictures.** `docs/guide/workflows.md`, the CLI guide's new section, the `website`
   nav, README highlights, screenshots of the chart and the run dialog with the throwaway profile.

Everything in slices 1–3 works with no window, which is what makes triggers other than manual a
matter of adding a producer of queued runs later.

## Not in v1, on purpose

- Schedules and forge-event triggers. The roadmap's "automations" item lists the questions those
  raise (a run while the previous one is going, a worktree per run, permission with no one to
  ask); a manual workflow answers none of them and does not need to.
- Editing the chart by hand; per-repository workflow files; conditionals, loops, `if:`; step
  retries; a step that runs a program (it would be a shell string by another name — project
  automation is where commands live, and a `run_automation` step can call it later).
- Reviewing in a fresh worktree; headless reviewers.
- Jev's judgment of a review, or of a workflow a model wrote.

## Open questions (new)

21. **How does `ys` know the app is running?** A lock file the app keeps open is the proposal; on
    Windows an open file cannot be deleted, on Unix a stale one can be detected by trying the lock.
    To be verified in slice 2 on all three.
22. **What is `settled` for a harness without native events?** PTY quiet past the attention
    threshold is the only signal, and a reviewer that pauses to think for longer than that would
    end the step early. The forge check behind it catches the false end; whether `wait_session`
    should require _both_ quiet and no `turn.started` since, where hooks exist, is a slice 3
    finding.
23. **Should a `harness` input show only harnesses that are installed?** The composer does; the
    run dialog should match, and `ys` should refuse an id that is not found.

## Documentation this touches

`docs/guide/workflows.md` (new), `docs/guide/cli.md`, `docs/guide/terminals-and-sessions.md`
(notifications from a run), `docs/guide/assist.md` (Describe it is Drafting, not Assist),
`docs/README.md`, `website/src/lib/docs.ts`, `README.md`, `CHANGELOG.md`, [02-ux](02-ux.md) (the
left panel now has a section above projects), [03-architecture](03-architecture.md) (the engine
and the driver), [05-roadmap](05-roadmap.md) (M22), [06-open-questions](06-open-questions.md)
(21–23), [08](08-manual-checklist.md) (§22).
