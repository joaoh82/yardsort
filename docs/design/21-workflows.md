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
lock dies with the process, so a crash cannot leave a stale "running", and a second app on the
same profile does not get it, so it does not drive runs.

Rules the engine enforces regardless of what a file says:

- **A busy agent is never written to.** `send_to_session` waits for quiet. A session that ended
  skips the step rather than failing it.
- **One run per workspace per workflow at a time.** Starting a second is refused with the first's
  id.
- **Cancel stops the run, not the agents.** Sessions a run started keep running as ordinary
  sessions; the run's remaining steps become `cancelled`.
- **Memory once.** A `start_session` prompt that places `{{ memory }}` itself starts the agent
  with `skip_memory`, so the launcher does not append the memory a second time.

## The command line

```
ys workflow list                          # built-ins and yours, with where each comes from
ys workflow show <id>                     # the YAML, or --json the parsed form
ys workflow copy <id> [--as <new-id>]     # a copy in your folder to edit
ys workflow validate <file>               # exit 0 or the errors, one per line, with line numbers
ys workflow run <id> [--workspace <name>] [--input reviewer=claude --input focus="…"]
ys workflow runs [--workspace <name>] [--run <id>]   # history, or one run step by step
ys workflow cancel <run>
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
to. Three parts:

- **Chart.** The steps as nodes, `needs` as edges, laid out top-down by `@dagrejs/dagre` and
  drawn by `@xyflow/react`, read-only: the YAML is what runs and the chart is a view of it. With
  a run open, each node shows that step's status. The layout is a pure function with tests of
  its own.
- **File.** A CodeMirror editor (`@codemirror/lang-yaml`, `@codemirror/lint`). Every pause in
  typing goes to `workflow_check`, which is `workflow::parse`: the problems become gutter marks
  and a list under the editor, and the last shape that checked out stays on the chart. Unsaved
  text is kept in the store, per workflow, while the app runs. A built-in is read-only with
  **Customize** (a copy under the same id, used instead) and **Duplicate** (a copy under a new
  id); a user file has **Save**, **Revert**, **Duplicate**, and **Delete** or **Reset to
  built-in**, which asks first through the OS dialog.
- **Runs.** Newest first, each opening to its steps, notes and pull request; **Cancel run** on
  one still going. `WorkflowRunsChanged`, emitted by the driver after any tick that changed a
  run, reloads what is on screen.

**Run…** is one dialog, opened from the workflow, from a workspace's menu (**Run workflow…**,
and **Request code review…** straight onto the built-in), or from either with the other still
to choose. It asks the workflow's inputs, an agent from the composer's list, and shows the pull
request a run would use through `workflow_preview`, or why there is none. `workflow_start` goes
through the same `runs::queue` as `ys workflow run`, and the app answers `Look` the way `ys`
does.

**Describe it**, a description written into a workflow by a model, is slice 5.

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
