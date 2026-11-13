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
   _Since 0.28: the section is one **Workflows** row, like Tests and Triggers, that opens the
   list in the main panel — each row marked built in or custom, a custom one deletable from
   there. A list of every workflow outgrew a third of the sidebar. The open list is
   `workflowId = ":list"` in the projects store, so "one thing takes the center panel" still
   holds without another flag._
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

| Key           | Rule                                                                                                                                                                                                                                                                                                                   |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`          | `[a-z0-9-]+`, unique across built-ins and user files; the file name should match                                                                                                                                                                                                                                       |
| `name`        | Required, shown in the sidebar                                                                                                                                                                                                                                                                                         |
| `description` | Optional, shown under the name                                                                                                                                                                                                                                                                                         |
| `version`     | The schema version, `1`. A file with a higher version than the app knows is listed, not runnable                                                                                                                                                                                                                       |
| `trigger`     | `kind: manual` only in v1; `context: workspace`, the default, says what the run is about and which variables exist. `context: tracked_pull_request` (since [38 § slice 5](38-pull-requests-elsewhere.md#slice-5-what-shipped)) is a run about a pull request a workspace tracks in another repository, started from it |
| `inputs[]`    | `id`, `kind` (`harness`, `text`, `choice` with `options`), `label`, `required` (default `false`), `default`. Collected before the run starts, in the app or as `--input`                                                                                                                                               |
| `steps[]`     | `id`, `action`, `needs` (step ids), then the action's own keys. An `approve` or `run_test` step may set `on` and `max_attempts`. See [Conditional paths](#conditional-paths). A step id is `[a-z0-9_-]+`                                                                                                               |

### Actions in v1

| Action             | What it does                                                                                                                                        | Ends when                                                | Produces              |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- | --------------------- |
| `start_session`    | Starts a harness in the run's workspace through the shared launcher, with a first message. `harness`, optional `model`, `effort`, `skip_memory`     | The process has been spawned                             | `session`, `run`      |
| `wait_session`     | Watches one session. `until: settled` is PTY quiet past the attention threshold or a `turn.completed` event; `until: exited` is the process ending  | The condition holds, or `timeout` → step fails           | `outcome`             |
| `send_to_session`  | Pastes a message into a live session — one this run started, or `origin` — once it is quiet. There is no key to change that                         | The paste was written, or the session was gone → skipped |                       |
| `wait_pr_activity` | Polls the forge for a review or comment on the workspace's pull request newer than the run started. `kind: review \| comment \| any`                | Activity exists, or `timeout` → step fails               | `count`, `latest_url` |
| `notify`           | An OS notification when the window is unfocused, a toast in the app otherwise; also a line in the run's log                                         | Immediately                                              |                       |
| `report`           | Says what the run came to: `body` (Markdown) and `title`, rendered, with consumed artifacts appended as a prompt's are. Nothing leaves the computer | Immediately                                              | `title`, `body`       |

Every action is a plain description of intent. None takes a shell string; `start_session` goes
through the same argv-building launcher as the composer and `ys workspace new`, so hooks are armed
and the run is recorded in activity like any other.

### Variables

`{{ path }}` with dotted lookup, nothing else — no filters, no logic. Whitespace inside the braces
is ignored. A `{{` that is not a known variable is a validation error, so a typo cannot reach an
agent as literal text. `\{{` writes a literal `{{`. Names are checked when the file is loaded,
against the namespaces the trigger's `context` allows:

| Namespace   | Fields                                                                                                                                                                                                                                                                                                                                                                                                      | Available                                    |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `project`   | `name`, `root`                                                                                                                                                                                                                                                                                                                                                                                              | always                                       |
| `workspace` | `name`, `branch`, `base_branch`, `path`, `task` (its first message, as outcomes record it)                                                                                                                                                                                                                                                                                                                  | `context: workspace`                         |
| `pr`        | `number`, `url`, `title` — the open pull request whose head is the workspace's branch. A step that names one before any `wait_pr_activity` needs it open when the run starts. When every such step comes after that wait, the run can start without one and looks again. For `tracked_pull_request`, the tracked one is required, with `repo` (`owner/name`) and `clone` (the project's nested clone of it) | `context: workspace`, `tracked_pull_request` |
| `inputs`    | One field per declared input, by `id`. An optional input with no value renders empty                                                                                                                                                                                                                                                                                                                        | always                                       |
| `steps`     | `<step id>.<produced field>`; only steps named in `needs`, transitively                                                                                                                                                                                                                                                                                                                                     | always                                       |
| `memory`    | The project's approved memory, rendered as the launcher renders it, or empty when the project does not share it                                                                                                                                                                                                                                                                                             | always                                       |
| `handoff`   | The workspace's handoff packet, as `ys workspace handoff` prints it                                                                                                                                                                                                                                                                                                                                         | `context: workspace`                         |

A step's `session` is not a variable but a reference: `origin`, the workspace's most recent live
harness session, or exactly `{{ steps.<id>.session }}` for an earlier `start_session` step among
its needs. A `harness` is a harness id or exactly one `{{ inputs.<id> }}` of kind `harness`.

## The run

A run is rows, not a process. Migration `0012_workflow_runs.sql`:

- `workflow_runs`: id, workflow id and name, the **file as it was** when the run was asked for (a
  run must not change under an edit), project id, workspace id and a copy of its name (history
  reads the same after the workspace is deleted), the inputs as JSON, `status` (`queued`,
  `running`, `succeeded`, `failed`, `cancelled`), `requested_by` (`app`, `cli`), `error`,
  `created_at`, `started_at`, `ended_at`. A partial unique index on `(workflow_id,
workspace_id)` over queued and running rows is what makes "one at a time" hold across two
  processes.
- `workflow_step_runs`: run id, step id, its place in the file, its action, `status` (`pending`,
  `running`, `waiting`, `succeeded`, `failed`, `skipped`, `cancelled`), `started_at`,
  `ended_at`, what it left for later steps as JSON, and a `note` saying why it failed or was
  skipped.

Every step change is conditional: it applies only if the run is still `running` and the step is
in the state the mover expects. So a cancel from `ys` can never be undone by an app that had not
heard of it, and nothing is moved twice.

Nothing about a run blocks the app. A step that waits — for an agent to settle, for a review to
appear — is a row marked `waiting`, not a thread asleep; the window stays a view that reads rows
and refreshes on an event, and any number of runs can be in flight at once.

**The engine** (`yardsort_core::workflow::engine`) is a function, not a thread: `advance(workflow,
inputs, steps, world) -> Plan`, which returns at once. `World` answers the clock, what a session
is doing (alive, busy, when it last settled) and the variables outside `inputs` and `steps`. A
`Plan` is conditional step moves, effects for the driver (`Start`, `Paste`, `Notify`), and whether
the run is finished. Steps start as soon as every need has succeeded, so side-by-side steps start
in the same call; a need that ended any other way skips the step, saying which. Tested with a fake
world: no terminal, no database.

**The driver** (`yardsort_core::workflow::driver::tick`) applies plans to the store and effects
to sessions, until a run can go no further for now. What only the app can do comes in through a
`Hands` trait: start an agent exactly as the composer does, show a notification, and write the
handoff packet, which needs git and the login environment. The app's side
(`src-tauri/src/workflows.rs`) is a thread of its own, never the UI's. It calls `tick` every
second, and at once when the host reports a session quiet or exited: those events pass through
the driver on their way to the window. `ys` does not poke the app. A queued row is found within a
second, which removed the inbox file the proposal had.

- **Settled** is a quiet after at least 8 seconds of output (the window's own bar, which also
  ignores the echo of a paste), or a `turn.completed` event for the agent's run, whichever is
  later, and it must come after the waiting step began. Quiets are remembered in memory, so
  after a restart only reported turns are known.
- **Typing** is the first-prompt delivery's method: paste, 150 ms, then Enter. The session is
  looked at again at that moment, not only when the step was planned: busy by then, the step goes
  back to waiting, and a session typed to once in a tick is not typed to again in that tick,
  because the host sees it busy only once output comes back.
- **A step is claimed just before its effect,** through the same conditional update as every
  move, not with the rest of the plan. A cancel that lands while one effect is under way (a slow
  agent start, say) stops every effect after it.
- **A restart** fails any step found `running`, with a note saying Yardsort stopped during it:
  whether the agent started or the text was typed cannot be known, and doing it twice is worse
  than saying so. The proposal would have checked the activity table instead; a run row carries
  no link to its launch yet, so that waits.
- **A session the driver starts** is announced to the window with a `SessionStarted` event, and
  the window adds it as a tab without changing the one in front. The window had only ever listed
  sessions when it loaded.

**Presence** (`yardsort_core::presence`, open question 21): the app holds an exclusive OS lock on
`app.lock` in the data directory for as long as it runs, with the standard library's file
locking (flock on Unix, `LockFileEx` on Windows). `ys` tries the same lock without waiting. The
lock dies with the process, so a crash cannot leave a stale "running". A second copy on the same
profile does not get it, and since 0.22.0 gives way to the first rather than starting without driving
runs.

Rules the engine enforces regardless of what a file says:

- **A busy agent is never written to.** `send_to_session` waits for quiet. A session that ended
  skips the step rather than failing it. Nor is one controlled from elsewhere: while a lease is
  held the step waits, and its typing never takes control from anyone
  ([27 §8](27-remote-r2.md#8--what-the-fifth-slice-adds)).
- **One run per workspace per workflow at a time.** Starting a second is refused with the first's
  id.
- **Cancel stops the run, not the agents.** Sessions a run started keep running as ordinary
  sessions; the run's remaining steps become `cancelled`.
- **Memory once.** A `start_session` prompt that places `{{ memory }}` itself starts the agent
  with `skip_memory`, so the launcher does not append the memory a second time.

**Continuing a failed run** (`workflow::runs::continue_failed`, `Store::continue_workflow_run`)
is one transaction on the same rows, not a new run: the run goes from `failed` back to
`running`, with its error and end time cleared, and every step that is `failed`, `skipped` or
`cancelled` goes back to `pending`. Nothing else is needed, because the engine already knows
what to do with a running run whose steps are pending: the failed step's needs have succeeded,
so it begins; a step skipped by a branch that did not choose it is skipped again, for the same
reason. A step that goes back is treated as a rewind treats one: outputs, note and outcome
cleared, its decision and its clarification rounds archived and removed, its `changes-*` and
`failure-*` notes retired, a smoke test it still held cancelled in the same transaction, and
its attempt count kept, so `max_attempts` still bounds it. Steps that succeeded are not
touched, which is the point: their outputs, including the terminal ids later steps address,
and their artifacts are what the continued steps read.

It is refused, writing nothing, for a run that is not `failed`, while another run of the same
workflow is queued or running in the workspace (the same rule, and the same unique index, as
a new run), and when the workspace is archived or gone. Cancelled runs are left out on
purpose: a cancel is a decision, and a step cancelled mid-effect is the case a restart already
refuses to guess about. A continued run is no longer `unattended`, whoever started it:
somebody is there now.

A run can therefore fail more than once under one id. The phone's failure notice is keyed by the
run and by the time it ended (`notify::workflow_failure`), not by the run alone, so a continued
run that fails again is told about again, while the same failure asked about twice is still one
notice.

Two things it does not do. It does not bring back a session that has ended: a later step that
addresses one skips or ends as it would have in the first place. And it does not pick the
step: it is always the failed one and what was skipped behind it, never an earlier step that
succeeded. Running an arbitrary step again is a rewind, and a file asks for that with a route.

## The command line

```
ys workflow list                          # built-ins and yours, with where each comes from
ys workflow show <id>                     # the YAML, or --json the parsed form
ys workflow copy <id> [--as <new-id>]     # a copy in your folder to edit
ys workflow validate <file>               # exit 0 or the errors, one per line, with line numbers
ys workflow run <id> [--workspace <name>] [--input reviewer=claude --input focus="…"]
ys workflow runs [--workspace <name>] [--run <id>]   # history, or one run step by step
ys workflow cancel <run>
ys workflow continue <run>                # a failed run, from the step that failed
```

`run` refuses while the app is not running, then goes through `workflow::runs::queue`, the one
door for a run, which the app's Run dialog will use too. It checks the workflow, that this
version can do every step, the workspace, and every input; an agent named by an input or a step
must be set up and on `PATH`, by the same check `ys workspace new` uses. Refusals write nothing.
Missing inputs are listed by id and label: there is no prompt in the terminal. Inside a workspace,
or in an agent's terminal there, `--workspace` can be left out, found as `ys memory propose` finds
its workspace.

## The UI

**Sidebar.** `WORKFLOWS` above `PROJECTS`, one row per workflow with a count of running runs, a
mark for problems and one for unsaved changes, **+** for a new one. The list is the store's
array; anything derived is derived outside the selector.

**Center panel.** The open workflow lives in the projects store beside the composer's fields
(`workflowId`), because one `set` must clear both: selecting a workspace or composing closes the
workflow, opening a workflow closes the composer, and the selected workspace is kept to come back
to. Two views, switched in the header, held per workflow in the view's own state: **Runs** and
**Definition** (since 2026-11-13; before, all three parts below were on one screen, and the run
and its steps were squeezed into a 320px column). The page opens on Runs when the newest run is
failed, queued or running, or a run was asked for from elsewhere (`shownRun`), and on Definition
otherwise. That is decided once, when the runs are first known, so a run starting later does not
move the page. The layouts are in the Figma file "Yardsort — Workflows UX" (frames A1–A3).

- **Definition: chart.** The steps as nodes, `needs` as edges, laid out top-down by
  `@dagrejs/dagre` and drawn by `@xyflow/react`, read-only: the YAML is what runs and the chart
  is a view of it. A 26rem column the full height of the page, left of the file, so the nodes
  fit at a readable size; it was 42% of the height above the file, mostly empty. Each node shows
  its status in the run on screen in Runs, behind a checkbox. The layout is a pure function with
  tests of its own.
- **Definition: file.** A CodeMirror editor (`@codemirror/lang-yaml`, `@codemirror/lint`). Every
  pause in typing goes to `workflow_check`, which is `workflow::parse`: the problems become
  gutter marks and a list under the editor, and the last shape that checked out stays on the
  chart. Unsaved text is kept in the store, per workflow, while the app runs. A built-in is
  read-only with **Customize** (a copy under the same id, used instead) and **Duplicate** (a copy
  under a new id); a user file has **Save**, **Revert**, **Duplicate**, and **Delete** or **Reset
  to built-in**, which asks first through the OS dialog. Save and Revert stay in the header in
  Runs while the text is unsaved.
- **Runs.** Three columns. The list, newest first; the newest is on screen until another is
  picked. The run (`RunPanes`): its times, the error with **Continue from the failed step** under
  it, **Cancel run** on one still going, **Copy id**, then **Summary** and the steps. Over six
  steps, three or more succeeded in a row fold into one row (`railRows`), and the picked step
  never folds. The detail takes the rest of the width: the summary, or the picked step's note,
  approval or question, decision and artifacts (Markdown when `looksLikeMarkdown`). A run opens on
  the waiting step, else the failed one, else the summary (`openingPick`).
  `WorkflowRunsChanged`, emitted by the driver after any tick that changed a run, reloads what is
  on screen.

**Run…** is one dialog, opened from the workflow, from a workspace's menu (**Run workflow…**,
and **Request code review…** straight onto the built-in), or from either with the other still
to choose. It asks the workflow's inputs, an agent from the composer's list, and shows the pull
request a run would use through `workflow_preview`, or why there is none. `workflow_start` goes
through the same `runs::queue` as `ys workflow run`, and the app answers `Look` the way `ys`
does.

**Describe it.** A new workflow has a description box above its editor. The description goes
through Drafting's own writers and switch (`draft::commands::writers`): the agent the user has, in
its non-interactive mode, or their Anthropic key — never Jev. The standing instruction
(`workflow::describe::SYSTEM`) is the file format, the rules the validator enforces, the
variables, and the built-in review whole as the example. The answer is checked as a typed file
is; with problems it is sent back once, with them at their lines, for one more answer. Whatever
comes back is the new workflow's unsaved text, with what is still wrong marked in the editor.
The agent runs in an empty git repository of the profile's (`<data dir>/drafting`, made once),
not in a project: a description is not a change to describe, and nothing here should read the
user's repositories. A repository rather than a bare folder because Codex refuses to run in a
folder that is not a trusted checkout.

## Slices

Each is one pull request with its docs, tests and changelog line, in this order. Each stands on
its own.

1. **The file and the core.** ✅ Schema types, YAML loading, the validator with line numbers,
   the variable checker, the built-in `code-review`, the user directory, the guide for the file
   format, and `ys workflow list | show | copy | validate`. See
   [slice 1](#slice-1-what-shipped) below.
2. **The engine and the driver.** ✅ Migration 0012 and the run store, moved here from slice 1: a
   migration ships once and cannot be edited, so it waits for the code that writes its rows.
   `advance` with a fake world; the driver with real sessions in a temp data directory: a
   workflow that starts a shell-backed "harness", waits for it to settle, pastes into it,
   notifies. `ys workflow run | runs | cancel`, the app presence check, a restart mid-run. See
   [slice 2](#slice-2-what-shipped) below.
3. **The forge step and the built-in end to end.** ✅ `wait_pr_activity` over `gh pr view
--json url,reviews,comments`, the `pr` variables and resolving `session: origin`. The
   code-review workflow run against a real pull request is the hands-on pass in
   [08 §22](08-manual-checklist.md#22--workflow-runs). See [slice 3](#slice-3-what-shipped).
4. **The UI.** ✅ Sidebar section, the workflows view, chart, editor, runs, the run dialog, the
   workspace menu entry. Testing Library for the list, the dialog's input rules and the run
   history; the chart's layout as a pure function with tests of its own. See
   [slice 4](#slice-4-what-shipped).
5. **Describe it.** ✅ `workflow::describe` in the core, the `workflow_describe` command, the
   validate-and-retry loop, the editor hand-off. Tests with a fake writer that returns bad YAML
   once. See [slice 5](#slice-5-what-shipped).
6. **Docs and pictures.** ✅ The README's highlight and section, the website's section and
   `ys` line, the quick start, and screenshots of the view and the Run dialog with the throwaway
   profile. See [slice 6](#slice-6-what-shipped).

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

## Slice 2: what shipped

Runs, from `ys`, carried out by the app. [The run](#the-run) and
[the command line](#the-command-line) above describe what was built; what changed on the way:

- **The driver's loop is in the core,** behind `Hands`, rather than in `src-tauri`. That is what
  let it be tested against a real program in a real PTY, a shell script standing in for the
  agent, from start to settle to typed message to notification, and through a cancel, a failed
  start and a restart. That test is Unix-only, as the PTY host's own tests that wait on output
  are; the engine, store and queue tests run everywhere.
- **No inbox handoff.** The driver ticks every second and finds queued rows itself; a queued run
  starts within a second without `ys` reaching the app at all.
- **The presence lock is the standard library's** file lock, so there is no platform code of our
  own to get wrong, and a crashed app cannot look alive.
- **Steps found running after a restart fail,** rather than being checked against the activity
  table, because nothing yet links a run's step to the launch it made.
- **From review:** claims and readiness are both checked at the moment of acting, as described
  under [the run](#the-run). The first version claimed a whole plan's steps before acting on any,
  so a cancel mid-plan still let the later effects happen; and it trusted the planning-time look
  at a session, so two messages due at once could both be typed into an agent the first had
  just set working.
- **Sessions started by a run needed a new event.** The window only listed sessions once, when
  it loaded, so an agent a run started would have had no tab until a restart.
- **`notify` is a system notification, always.** Showing it in the window instead when the window
  is in front waits for the Workflows view.
- **Refused at the door for now:** `wait_pr_activity`, `{{ pr.… }}` and `session: origin`, which
  are slice 3. The built-in code review uses all three, so it cannot run yet.

## Slice 3: what shipped

The pull request and the workspace's own agent; the built-in code review runs.

- **A run keeps what it found out** in a `context` column (migration 0013, not an edit of 0012,
  which had shipped): the workspace's open pull request when the run was queued — number, link,
  title — and its own agent when it started. `{{ pr.… }}` and `session: origin` mean those for
  the whole run, however long the review takes and whatever opens meanwhile.
- **The pull request is looked up when the run is queued,** through `runs::Look`, the queue's
  one way of asking the outside world, which `ys` answers with `gh` and the app's Run dialog will
  answer the same way. The branch is git's checked-out one, as the publish panel finds it, so
  `local` works too. No open pull request, no `gh`, or `gh` logged out: refused before anything
  is written, saying which.
- **The origin is resolved when the run starts,** not when its step comes: the workspace's newest
  conversation record whose session is still running with a harness label. Resolving it later
  would find the reviewer the run had started itself, the newest agent of all. None running then:
  the step is skipped, saying so.
- **`wait_pr_activity`** counts reviews and comments posted since the run started, to the second
  GitHub keeps. The driver asks `gh pr view --json url,reviews,comments` at most every 30 s per
  run, on a thread of its own, and keeps the last answer between ticks; a failed or stopped ask
  is logged and asked again next interval, and the step's timeout is what gives up.
- **From review:** the pull request is looked up by branch (`gh pr list --head <branch> --state
open`), not found among the repository's fifty newest: an open pull request with fifty newer
  merged ones was reported missing. And the forge is asked off the driver's thread. A question
  goes to a thread of its own, one per run at a time, and its answer is there for a later tick;
  `gh` itself is stopped after 60 s. Asked on the driver's thread, a slow `gh` held up every
  other run, its messages, notifications and timeouts, for as long as it took.
- **What `gh` says about reviews,** checked against a real pull request: replying in a review
  thread appears as a review of its own, with an empty body. So a person replying to an old
  thread while the reviewer works would end a `review` wait early. A review with only line
  comments has an empty body too, so the body cannot tell them apart; left as it is.
- **The built-in's prompt** now tells the reviewer that GitHub refuses an author's approval or
  request for changes, and to post as a comment when it is posting as the author — as it is when
  both agents use the same `gh` login. And the author is given the review's link.
- **A code review against a real pull request** was not run here: it posts to GitHub. It is the
  hands-on pass in [08 §22](08-manual-checklist.md#22--workflow-runs).

## Slice 4: what shipped

The Workflows section, the view and the Run dialog, as [the UI](#the-ui) above now describes
them. What changed on the way:

- **The open workflow is in the projects store,** not the layout store the proposal named. The
  composer's fields live there, and the rule "one thing takes the center panel" is one `set`.
- **Customize, not Duplicate, is the built-in's first button.** The proposal had one button that
  copied under a new id. Replacing the built-in under its own id is what most people want ("the
  review, but with my prompt"), so that is **Customize**; **Duplicate** makes a second workflow.
- **`workflow::save` never overwrites,** and only writes to a file the catalog lists as the
  user's, found by path; a path outside the folder is refused. A new file goes through
  `create_new`. Deleting is the same check.
- **The driver's tick says whether anything changed,** so the window is told only then; a run
  that is waiting costs it nothing.
- **Timeouts became `u32`** (a week is 604 800 s), because the generated bindings refuse `u64`.
- **Chart and editor are loaded when first shown,** as the diff viewer is; in tests they are
  stood in for, and the layout is tested on its own.
- **From review:** a save no longer drops what was typed while the file was being written —
  the draft is cleared only when it still is what was saved, and follows the new id when saving
  gave one; a required `choice` starts on its first option, so what the control shows is the
  answer; the driver's tick compares the active runs to what was active after its last tick, so
  a run cancelled from `ys` between ticks is a change the window is told about; and a
  workflow's run history is the workflow's own query with its own limit, not the newest two
  hundred of every workflow filtered afterwards.
- **Owed:** screenshots of the section, the view and the Run dialog with the throwaway profile
  (slice 6), and the hands-on pass in [08 §22](08-manual-checklist.md#22--workflow-runs).

## Slice 5: what shipped

Describe it, as [the UI](#the-ui) above now describes it. What changed on the way:

- **No `Want::Workflow`.** Drafting's `Want` is a diff-shaped question with a static system
  prompt; a workflow's system prompt carries the built-in as an example, so it is its own
  `concat!` constant in `workflow::describe`, and Drafting's `Prepared` and `run` became
  `pub(crate)` for the workflows command to use. The writers and the switch are shared through
  one `writers` function, so the two paths cannot drift.
- **The loop is the core's** — `next_prompt` and `finish` — and the app only runs it, so the
  fake-writer test (bad YAML once, then good; bad twice, problems left) is a core test with no
  agent. `MAX_TRIES` is two.
- **The agent stands in an empty repository of the profile's,** not the project root the
  proposal had. There is no project chosen in the Workflows view, and an agent reading a
  repository to write a workflow file would be reading it for nothing.
- **From review:** the scratch folder is a git repository, because Codex's `exec` refuses a
  folder that is not a trusted checkout and would have failed every time it was the writer; the
  retry repeats what was asked before showing the problems, since a second answer is a fresh
  conversation and a first answer that lost the task left the second writer nothing to go on;
  and a generation's answer is kept only if the new workflow is as it was when the question went
  out, so a slower answer never replaces a newer one, what was typed meanwhile, or a discard.
- **Who wrote it is said in words** (the agent's label, or the model the key asks), from the
  same answer the commit-message button shows, so the person knows what they are trusting.
- **A run says what it will find nothing for.** Found in the window: a workflow a model wrote
  used `{{ workspace.task }}`, and the workspace had been started without a first message, so
  the prompt said "Its task is:" and nothing. `runs::empty_variables` names each variable the
  file uses that this workspace has nothing for — the task, a base branch on the default branch,
  memory a project does not share — and the Run dialog shows the list before starting while
  `ys workflow run` prints it as notes after queuing. Not a refusal: an empty line is allowed,
  and the built-in behaves the same. The first cut also flagged `{{ workspace.branch }}` in the
  project's own checkout, whose row has no branch of Yardsort's making; git knows the one
  checked out, so `Place` now asks it where the row does not know, as the pull-request lookup
  does, and the base branch falls back to the repository's default the way the publish panel
  finds it.
- **The agent path gets the standing instruction in front of its prompt.** Drafting gives an
  agent only the question, and the system prompt to the key's path; a commit message needs no
  more, a workflow's format is the whole point. Checked once for real: Claude Code, in an empty
  folder, given the exact prompt for "have Codex review the pull request and tell me, not the
  original agent", wrote a valid four-step workflow first time, with no step for the author, and
  wrote nothing into the folder.

## Describe a change: what shipped

Added after slice 5. The same writer changes a workflow that exists, from a description of the
change, in the window and through the Floor Manager:

- **One loop, two asks.** `workflow::describe::Ask` is the description plus, for a change, the
  current text. A change's prompt shows the whole file, asks for only what was described, and
  names the `id` to keep. `finish` counts another `id` as a problem, so it goes back with the
  others: a customized built-in replaces the built-in only by that id. `workflow_describe` takes
  the current text as an optional second argument.
- **Customize opens it.** The copy is made at once, as before, and the view opens a **Describe a
  change** box on it. A file of the user's has a button for the same box. The answer is the
  workflow's unsaved draft. The serial that sets aside a stale answer is now kept per workflow
  id, not only for the new one, so a save, a revert, or typing meanwhile sets it aside too.
- **The Floor Manager writes through `ys workflow save`**, not into the folder. The gate cannot
  see a plain file write. `save` is an `Act`, refuses a file with problems (`ys` has no editor
  to leave it open in), and replaces the user's own file with that id wherever it is. At ask
  first it is treated like `task comment --body-file`: the file is read once, shown whole in the
  confirmation, and that text is what is saved; `-` is refused there.
- **The window notices the folder.** The driver's tick compares `workflow::folder_stamp` (names,
  sizes, times) and emits `WorkflowRunsChanged` when it moves, so a file saved by `ys` shows up
  without waiting for a run.

## Slice 6: what shipped

Words and pictures, nothing of the code. The guide's running section and the changelog line had
gone out with the slices they described, so what was left was the places that repeat the
README's claims:

- **The README** gained a highlight, a section of its own after the handoff one, the workflow
  commands in the scriptable highlight, a fourth quick-start step and a documentation row.
- **The website** gained a sixth idea, "Named agent work, in steps", with the view's screenshot,
  the same claims as the README's section, and the sections after it moved down one number; the
  command-line list gained `ys workflow run code-review`.
- **The quick start** points at **Request code review…** once the pull request is open, and its
  reading list at the guide.
- **Two screenshots**, taken with `scripts/screenshots.sh` on the throwaway profile: the built-in
  review open, with the chart, the file and its runs, and the Run dialog opened from a
  workspace's menu. The guide and the README show the first; the guide shows the second too.
  The demo repositories have no remote, so the first take of the dialog said finding the pull
  request needs `gh`; the script's setup now puts a stand-in `gh` on the profile's `PATH` that
  answers with one demo pull request, on the newest workspace branch only.

The hands-on pass on macOS and Windows, [08 §22](08-manual-checklist.md#22--workflow-runs), is
still to be done, and is the milestone's exit.

## Not in v1, on purpose

- Schedules and forge-event triggers. The roadmap's "automations" item lists the questions those
  raise (a run while the previous one is going, a worktree per run, permission with no one to
  ask); a manual workflow answers none of them and does not need to. **Schedules have since been
  answered in [34](34-schedules.md)**, and not as a trigger: a schedule is a row of its own that
  queues a run through the same door, so `kind: manual` is still the only trigger a file has.
  Forge events are proposed in [36](36-triggers.md), the same way: a trigger is a row that
  queues a run, and the file keeps `kind: manual`.
- Editing the chart by hand; per-repository workflow files; `if:`; unbounded retries of a step.
  Conditional paths and bounded revision loops are in [Conditional paths](#conditional-paths).
  A step that runs a program would be a shell string by another name. Project automation is
  where commands live, and a `run_automation` step can call it later.
- Reviewing in a fresh worktree; headless reviewers.
- Jev's judgment of a review, or of a workflow a model wrote.

## Open questions

[21–23 in open questions](06-open-questions.md#workflows): how `ys` knows the app is running
(settled in slice 2), what `settled` means without native events, and which harnesses a
`harness` input offers.

## Documentation this touches

`docs/guide/workflows.md` (new), `docs/guide/cli.md`, `docs/guide/terminals-and-sessions.md`
(notifications from a run), `docs/guide/assist.md` (Describe it is Drafting, not Assist),
`docs/README.md`, `website/src/lib/docs.ts`, `README.md`, `CHANGELOG.md`, [02-ux](02-ux.md) (the
left panel now has a section above projects), [03-architecture](03-architecture.md) (the engine
and the driver), [05-roadmap](05-roadmap.md) (M22), [06-open-questions](06-open-questions.md)
(21–23), [08](08-manual-checklist.md) (§22).

### Workflow writer preference and chart navigation

Settings → Assist now stores `[draft].workflow_harness_id`: absent means Automatic, otherwise
workflow descriptions use that installed, enabled harness with `write_args`. Automatic filters
for installed commands before choosing, so an absent Claude installation cannot hide Codex or
a custom writer. Status and execution share the same resolution. An unavailable explicit choice
is reported rather than silently choosing another harness. Workspace drafting keeps its existing
preference for the workspace's agent; both paths retain the optional Anthropic fallback.

Chart nodes are keyboard-accessible buttons with one selected step. The YAML parser supplies a
step-id-to-source-line map with each check, including semantically invalid files. It ignores
lookalikes in prompts and omits ambiguous duplicate ids. The editor waits for positions from its
current text before navigating, marks the selected id line, and scrolls it into view without
changing the file or taking focus from the chart. Clicking the same step again navigates again;
clicking the background clears the mark. Built-in read-only files support the same navigation.

## Test verdict gates

A step that judges something records a structured outcome on its row, separate from the strings
later steps read as `{{ steps.<id>.<field> }}`. The outcome is JSON,
`{"verdict":"passed","reason":null}` or
`{"verdict":"failed","reason":"..."}`. `verdict` is `passed` or `failed`. `reason` is the
text when it failed, and null when it passed. An approval adds `decision`, `feedback`,
`actor` and `decidedAt`. Those default to null, so an outcome saved before them still
reads. This is not `wait_session`'s `outcome` output,
which stays the string `settled` or `exited`.

`engine::react` is the one place a verdict changes the run. A gate whose verdict is `failed`
fails the step and halts the run: every step that has not finished (pending, waiting, or
running) is skipped in that same tick. A step that lists the gate in `needs` is skipped
because the gate did not succeed, and its note says so. Every other step that was still
going is skipped because the gate failed, and its note says `Skipped: the gate <id> failed.`
An effect the plan was about to start is dropped. A step already running from an earlier
tick is skipped too. A smoke test that step was waiting on is cancelled, so it does not
keep running or keep asking for approval. Other work that effect already started is left
running. The run then ends failed. Any
other verdict succeeds the step and keeps the outcome, so a later step can still read what
happened. Later actions that judge something, an approval or a branch, call `react` instead of
inventing a stop of their own. A branch reads the outcome, not free text. Only a gate
with no `on` routes turns a failed verdict into status `failed`. A step that is not a
gate stays `succeeded` when the verdict is `failed`, with that verdict on its outcome.
A step with `on` follows [Conditional paths](#conditional-paths) instead.

`run_test` accepts `gate`, and it is a field of the step, next to `needs`, not of
the action. `allowed` is what decides which actions may set it. Absent, or `gate: false`,
is not a gate for a test. An `approve` step is always a gate. `gate: false` on one is a
parse error, and leaving `gate` out is the same as `gate: true`. A failed test verdict on a
step that is not a gate still succeeds the step, with the note `The test failed.` and outputs
`run`, `verdict` and `summary`, exactly as before. A passed verdict, gated or not, notes `The test passed.` A run
that reaches no verdict (cancelled, errored), or that times out, fails that step and records
no outcome, because there was nothing to react to. It does not halt the other branches.

The reason on a failed gate is the report `ys test wait` prints, under a line that says the
test failed. That full report is the step's note. The run's error names the step and then
the first line of that report after the line that says the test failed, such as
`Step smoke failed: failed login (2 of 6 steps passed).` The workflow run
view shows the short line on the run and the report on the step. `ys workflow runs` lists
runs, and after the table it prints a failed run's error, which is the short line. The runs
list in the app shows that line on a failed row. `ys workflow runs --run <id>` prints the short line, the
first line of each note in the table, and the full note under the table when it continues.
`ys workflow runs --run <id> --json` includes the full note and the outcome on the step
when there is one.

Starting a step again, from pending to running or waiting, clears its outcome. A later
revision can re-run the step and record a new verdict. Leaving the outcome out of a move does
not clear it.

A schedule or a trigger whose `notify` is `failure` (the default) or `always` sends the first
line of the run's error, kept to about 200 characters. When that reason is more than one
line, or the line was cut short, it adds `Open Yardsort for the report.` A one-line reason
is sent as it is. The full text stays on the run. `never` sends nothing. A `notify` step that
needs the gate does not run when the gate fails. Its title was written for a pass, and a step
runs only when every step it needs succeeded. A run started by hand, with no schedule and no
trigger, is not sent as a system notification. The failure is still on the run.

Migration `0029_workflow_step_outcome` adds a nullable `outcome` column. Rows from before it,
and steps that judged nothing, read as none. A workflow file that does not mention `gate`
still parses. A run that was queued before this change keeps the definition it was queued
with, which has no gate, so it keeps the old behaviour.

An `approve` step calls the same `react`. It is always a gate. While it waits, the steps
that need it stay pending. Approving records `decision: approved` and succeeds the step.
Rejecting records `rejected` and, with no `on` routes, halts the run. Requesting changes records
`changes_requested`, which is not a rejection, and with no `on` routes halts the run.
`on` can send either somewhere else. See [Conditional paths](#conditional-paths).
The outcome also stores `feedback`, `actor` and `decidedAt`. Outputs are
`decision` and `feedback`, so `{{ steps.<id>.decision }}` works. A timeout fails that
step with no outcome and does not halt a step that does not need it.

The decision is written to `workflow_approval_decisions` (migration `0031`, index 30)
before the engine applies it, so it survives a restart and is visible to `ys` at once.
The row is who, when, the decision and the feedback, plus a salt and a slow hash of
the one-time code and a count of wrong tries. The code itself is not stored. The row
is inserted when the step starts waiting, with the decision still null, and filled in
when someone decides. It is not deleted when the engine applies it. It is deleted
when a route sends the run back through that step, in the same change as the rewind,
and again when that step starts from pending, which issues a new code. A decision
is not reused on the next pass. A second decision
is refused, and so is one on a run that is not running, or on a step that is
not an `approve` step waiting. Requesting changes requires feedback. The
`changes-<step id>` artifact (source `decision`) is written in the same database
change as the step move and the wait's `applied_at`, when the outcome stops the run.
A route that sends the work back writes that artifact in the rewind instead.
An artifact name starting with `changes-` or `failure-` is refused at parse time.

A timeout moves the step to failed only when no decision has been recorded
(`NOT EXISTS` a row whose decision is set). A code row with no decision yet does not
block the timeout. If the timeout lands first, `ys` is told the step is not waiting.

You decide in the app, which passes no code, or with `ys workflow approve`, `reject`
and `request-changes` from a terminal Yardsort did not start. Leave `--code` out and
`ys` reads it from the terminal with echo off (`rpassword`), and only when both
standard input and standard error are terminals. Otherwise it fails at once and
says to pass `--code -` and pipe the code in. On Windows that prompt needs a real
console: cmd, PowerShell, or Windows Terminal. Git Bash and mintty do not, so
those use `--code -`. `--code -` reads one line from standard input and strips a
leading byte-order mark before trimming. A code passed in argv warns on stderr,
because it lands in shell history and in the process list. The code is shown in
the window. The waiting notification includes it unless `workflows.approval_code_in_notification`
is false. Unset, that follows the platform: included on macOS and Windows, omitted
on Linux, where a notification is readable on D-Bus. The omitted form says to open
Yardsort for the code. The code is nowhere else: not in `ys workflow runs`,
`--json`, the step note, outputs or a log. New hashes use 600000 PBKDF2 rounds.
`code_rounds` on the decision row (default 100000, from before this column) is
what a check uses, so an older hash still verifies. Any other stored count,
including 0 or 1, is refused and does not burn a try. The check runs before the
write transaction, which then writes only if the attempt count, the code hash and the
decision are still what the check saw. Five wrong codes lock the terminal. That count
survives a restart: opening the app replaces the code, because the plaintext was
only in memory, and it does not reset the count. **Show a new code** does reset
it, and is the only thing that does. Either replacement sends a notification
when the run would be told. Listing a run does not issue a new code. The
setting that leaves the code out of the notification is not a lock. A process
running as the same user can edit `settings.toml` and set
`[workflows] approval_code_in_notification` back on, and the next notification
includes the code.
`ys` also refuses when any of
`YARDSORT_COORDINATOR`, `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID` or
`YARDSORT_SESSION_RECORD_ID` is set. The Floor Manager's `ys` is `Action::Deny` at
every permission, including trusted and ask. This is not a sandbox. A process running
as the same user that writes SQLite directly is not stopped. Waiting for a GitHub
review is `wait_pr_activity`, and it is not a gate.

## Human waits

Migration `0032_workflow_human_waits` is index 31. It adds `workflow_waits` and
`code_rounds` on `workflow_approval_decisions`. A database from before it still
opens. Old runs keep working.

`workflow_waits` is why a run is blocked and what the person must do. An approval
opens a row when the step starts waiting: `kind` `approval`, `blocked_on`
`your approval`, the prompt, the artifact names it reads, and a phase object
(workflow, workspace, step, action). `notified_at` is set when the waiting
notification is sent, so a restart does not send it again. `applied_at` is set
in the same change as the engine applying the decision. A restart applies a
decision whose `applied_at` is still null exactly once, and does not approve a
gate that has no decision.

A clarification uses the same row, `kind` `question`, the questions in `prompt`
and the reply in `answer`. It is a mode of `wait_session` (`clarify: true`), not
an approval and not a new action. The agent asks with `ys workflow ask`. The
person answers in the run view, or with `ys workflow answer` from a terminal
Yardsort did not start. The terminal must present the one-time code. The window
does not: `answer_in_window` takes a `DesktopWindow`, the same guard as
`decide_in_window`. The code lives on the question wait (`code_salt`, `code_hash`,
`code_rounds`, `attempts`, migration `0035_workflow_clarifications`, index 34). It
is checked before the write, and the update matches that hash and attempt count.
Five wrong codes lock the terminal until Show a new code. A new round clears the
hash, so the next question gets a new code.

`applied_at` on the round and the wait is the claim, taken before the keystrokes.
`typed_at` on the round is set only after typing returns done. A session that is
held (`ControlHeld`) rolls both `applied_at` values back, and the next tick claims
again. A restart that finds `applied_at` set and `typed_at` null does not type
again. The step fails with "The answer may not have reached the session." The
keystrokes may already have landed, so a second paste would not be exactly once.
The claim also requires the run to be `running` and the step to be `waiting`, so
a cancel or a halt cannot type into a dead run. An answer to a step that has
already failed or timed out is refused, even when the run is still going.

A summary is refused while a question is still unanswered. A clarifying step finishes
only when `ys workflow summary` has been recorded, or
when `until` is `exited` and the session has ended with no question still open. A
settled session that is still running, with no summary, keeps waiting until the
step's timeout, then the step fails. A session that has ended, with no summary,
fails at once, including while an answer is still owed. An answer that was given
and not yet typed fails because the session ended before it could be sent back.
The code is issued even when `notify` is `never`; that setting only skips the
notification. A question only printed in the terminal is not a round, and a
session going quiet is not a summary.

Each round is also a row in `workflow_clarification_rounds` (migration
`0035_workflow_clarifications`, index 34), because the wait is one row per step
and a later round reuses it. The agent's `ys workflow summary` is one row in
`workflow_clarification_summaries`. When the step finishes, `from: brief` compiles
those rows into a named artifact. Nothing is read from the terminal.
Past `max_rounds` the extra round is stored and the step fails. It does not open
another wait. A rewind archives the rounds and the summary into
`workflow_clarification_history` and `workflow_clarification_summary_history` at
that step's attempt, then removes the live rows, the same way an approval decision
is archived. A merged rewind stores each `failure-<step>` or `changes-<step>` note
at that step's own attempt count, not at the attempt that gated the rewind.

`workflow::waits::list_open`, `decide_with_code` and `answer_with_code` are the
boundary a phone or a remote crate calls. They read and write this database. A
remote caller must use `decide_with_code` or `answer_with_code`, which always take
the one-time code. The window calls `decide_in_window` or `answer_in_window` with a
`DesktopWindow` token. Only the app enables the
`desktop-window` feature, so a remote crate cannot construct that token. The phone
uses the same guarded update, with the one-time code, and does not use the window
token. That route is [Mobile workflow controls](41-mobile-workflow-controls.md).
A decision recorded while the app was closed is not in `list_open`: the person has
done their part, and the engine applies it.

The step move, `applied_at`, and the decision's artifacts are one change. The
driver can stop after that change, or after the next step is claimed and before
its effect runs. The next start does not write the artifact again, and it fails
a step that was claimed and not carried out, so a notification or a session is
not started twice.

What this leaves for later, and does not do: notifications and controls from a
phone. Those read this wait, this outcome and the same failed run error. The
phone does not get a decision route here. Branching on a verdict or on
`changes_requested` is [Conditional paths](#conditional-paths). Asking mid-run is
the clarifying `wait_session` above.

## Conditional paths

An `approve` or `run_test` step may set `on`. Each key is a structured outcome, and the
value is a step id or `stop`. Free text is not an outcome. With no `on`, a failed gate
still halts the run and a success lets every step that needs this one start.

An `approve` step must name `approved`, `rejected` and `changes_requested`. A `run_test`
step must name `passed` and `failed`. Any other action refuses `on` and `max_attempts`.
A target must be a later step that needs this one, or an earlier step this one needs,
or this step itself, or `stop`. A step cannot be named `stop`, because that word means
the branch ends. A target that is neither is a parse error, and so is a
step that no path from the start can reach. A route back needs `max_attempts`, from 2
to 10, counting the first entry. Above 10 is a parse error. Setting it when nothing
loops is a parse error too. The engine also stops at 10. An inner loop's `max_attempts`
counts entries across outer loops, because `attempts` is never reset.

`stop` on a failed verdict halts the run. `stop` on a passed verdict skips steps that
have not finished and ends the run succeeded. A step that `produces` something stays
running while those artifacts are stored, and the stored outcome is what stops the run
once the step has succeeded, including on a later tick. A route to a later step succeeds this
step even when the verdict failed, so the chosen step can start, and skips the other
steps that needed this one and were not chosen.

A route back sets the target, and every step that needs it, back to pending. That
includes the step that branched. Two looping steps in the same tick whose sets
overlap are one change: the sets are joined and both `failure-<step>` or
`changes-<step>` notes are stored. A second rewind is not planned for a step the
first rewind already sends back, because that step would no longer be waiting and
its note would be dropped. The plan is one database change, taken only while
that step is still in the status the plan saw and its `attempts` count is still the
count the plan saw. A second apply, including after a restart, changes nothing.
`attempts` is not reset and is not increased by the rewind. It increases by one when
a step leaves pending for running, waiting or succeeded. A skip, a rewind, and a
restart that fails a step already running do not add one. Migration
`0033_workflow_attempts` adds the column (default 0) and is index 32.

When `attempts` is already `max_attempts`, the run halts instead of looping. The
error names the outcome and the limit, and says to start a new run. `ys workflow runs`
prints that line after the table. The runs list in the app shows it on the failed row.

The feedback goes to the step being run again as a named artifact. A request for
changes writes `changes-<step>`, source `decision`. A failed verdict writes
`failure-<step>`, source `verdict`, whether the route goes back or forward to a later
step. A later step may `consumes` that name only when the producer routes that outcome
onto a set that includes the consumer. The first pass has no body yet, and the handoff
skips a missing artifact. Each attempt's body is kept in `workflow_artifact_versions`.
The row in `workflow_artifacts` is the latest, which is what a consumer reads. A later
attempt does not replace an older body. The same attempt stored again updates that
version, so it matches the body that was used. A rewind drops the current `changes-*`
and `failure-*` rows of the steps it runs again, after copying them into the version
table, so an earlier loop's note is not handed out on the next pass.

Looping back deletes the approval decision and the wait for every step that runs
again, after copying the actor, the time and the decision into
`workflow_approval_history` (migration `0034`, index 33). The next pass through an
approval waits, issues a new one-time code, and needs a new decision. An old decision
is not applied again. The checks that refuse a session Yardsort started, and the Floor
Manager, still apply on that new decision. A rewind is not applied when the same plan
is already halting or stopping the run, so those steps are skipped rather than sent
back to pending. Judged steps later in the file still conclude in that same pass, so a
halt or a stop wins over a rewind even when the looping step comes first. A `run_test`
step that rewind sends back has its smoke test cancelled in the same transaction, so a
crash after the rewind still leaves that test cancelled.

A file with no `on` still loads, and a run queued from it keeps today's behaviour.

## Named artifacts

A step may declare `produces`, a list of named documents, and a later step may list those
names under `consumes`. Parse time refuses an unknown name, a duplicate, a consumer that does
not have the producer among its needs (directly or through other steps), and a
`{{ artifacts.<name> }}` that the step does not list under `consumes`. A transcript is copied
only when the step says `from: transcript`. A `start_session` or `send_to_session` cannot hand
on a file or a transcript, because that read happens before the agent has written or replied.
A `notify` step may hand on a file or a transcript that is already there.

The body is stored on the run when the producing step succeeds, in `workflow_artifacts`
(migration `0030`, index 29). The engine does not read files. A step that would succeed inside
`advance` stays running until the driver has stored the body, so a consumer in the same plan
does not start on an empty handoff. The artifacts and that move to succeeded are one database
change, and only while the run is still running. If Yardsort stops after the step has finished
and before those artifacts are stored, the step goes back to waiting and the capture runs
again. The pending marker stays in the step's outputs, so the capture runs again even
when the action would not succeed a second time, such as a notification. A step that was
still doing its own work fails instead, so it does not happen twice. A capture that then
fails drops the marker. A transcript read stays off the driver thread. The same read is
reused on the next tick, and the step fails only after about 20 seconds. The read is
keyed by the run and the step, so a finished read is not handed to a later step. A job
older than that limit is dropped on the next look and is not started again. The text
is capped inside the read, before it is sent back.
An effect that finishes in the driver, such as a notification, a started session or a paste,
stores its artifacts in that same change, before the step is marked succeeded.

Sources are a workspace file, text from the workflow file, one output field the step already
leaves, or a transcript. The transcript is opt-in. Without `raw`, it is the budgeted
conversation digest: tool output and reasoning are not part of a turn, and older turns are
shortened. `raw: true` keeps each turn's own text, still without tool output, and still not
the terminal scrollback. The driver does not attach to the session to copy it.

A file path must stay inside the workspace after links are resolved. The resolved path is
checked again, so `notes.md` pointing at `.env` is still a secret. A component containing `:`,
or ending in a dot or a space, is refused. A secret-looking path is refused when it is written
literally, and again when a variable in it is filled in. The body is cut at 32000 characters,
and the stored copy says it was cut. Control characters other than newline and tab are dropped
before that, and so are bidirectional controls (U+202A through U+202E, U+2066 through
U+2069, and the bidi marks), U+200B, U+2060 and U+FEFF. U+200C and U+200D stay.
An empty body fails the step, so the next phase does not start with a blank
handoff.

A re-run replaces that step's rows only after the new bodies have been read. A failed attempt
leaves the last good ones. Each attempt's body is also kept in `workflow_artifact_versions`
(migration `0033`), so a later attempt does not overwrite an older one. A consumer reads
the row in `workflow_artifacts`, which is the latest body. A `start_session` or `send_to_session` that consumes an artifact and
does not place `{{ artifacts.<name> }}` itself is given the text under a heading. A later
approval, a revision loop, or a control on a phone can point at the same name without parsing
the prompt.

A workflow file from before this has no `produces` or `consumes`, and still parses. A run
saved before the table still opens, and lists no artifacts. The schema version stays 1.

## What a run made

_Built 2026-11-13._ A run that opened pull requests did not say so: the owner's software-factory
run opened two, in two repositories, and they had to be found on GitHub. A run now lists them
(`workflow::made`), without reading anything an agent printed.

- **The project's own repository.** Each time the app reads a project's pull requests
  (`publish::commands::look`, the same place `outcomes::observe` is called), every run asked
  for in that project in the last week is matched: a pull request on its workspace's branch
  opened within its life counts. The life is `created_at − 2 min` to `ended_at + 15 min`, open
  while the run is going; the slack before is clock skew, the slack after is an agent still
  pushing as the last wait ends. A match is kept in `workflow_run_pull_requests` (migration
  `0040`), so `ys` and the window read it without the network, and its title and state follow
  the forge at each later look. A pull request with no opening time is not matched.
- **Other repositories.** The workspace's tracked pull requests (doc 38) whose tracking time
  falls within the same life, read at the time of asking. Not copied: untracking one removes it
  from the run too, and their titles are the Pull requests view's to fetch.
- A continued run keeps one life, from when it was asked for to when it last ended.

`WorkflowRun.pullRequestsMade` carries them to the window: counted on the run's row and the
Summary row, listed on the summary. `ys workflow runs` has a `PRS` column and `pullRequests` in
`--json`; `--run` prints them under **Pull requests**. The phone's notices are left as they
are: they carry no run content, by design, and the phone's run view is not changed here.

### The `report` action

_Built 2026-11-13._ The owner asked for a step whose job is only to output information. A
`report` renders `body` and `title` like a `notify` (`handoff_prompt` appends consumed artifacts
the body does not place) and keeps them as its outputs, cut at the artifact cap. It notifies no
one. `workflow::made::reports` reads a run's succeeded report steps in file order; that is the
run's result, carried to the window as `WorkflowRun.reports`, first on the summary, and printed
by `ys workflow runs --run` under **Result** (`reports` in `--json`). A report waiting to run
again after a continue is not part of the result until it succeeds again.

`run` is a new variable namespace, always available: `run.id`, and `run.pull_requests`, the
lines of `workflow::made::pull_requests_text`, read when the step renders. A run's own-repository
pull requests are otherwise only as current as the app's last look at GitHub, about a minute, and
a report is saved once, so a stale list would stay wrong. A `report` that places
`run.pull_requests` therefore stays pending until `World::pull_requests_current` says the forge
has been asked about the branch since the step became ready: the driver runs
`Hands::branch_pull_requests` (`gh pr list --head <branch> --state all`) once per report step on a
thread of its own, and feeds the answer through `made::observe_pull_requests`, the same matching
the app's look does. An error counts as an answer, so an unreachable forge delays a report by one
attempt, not forever. Other steps reading `run.pull_requests` read the table as it is.

The workflow writer's standing instruction (`describe::SYSTEM`) and the Floor Manager's
`AGENTS.md` say both, so an agent writing or changing a workflow, or answering about a run, knows
them. The built-in software factory has a `result` report between `release` and `ready`, so the
chart stays one column and the notification comes once the result is written.
