# Smoke tests

_Proposal, 2026-10-30. The six questions put to the user that day are answered under
[Decisions](#decisions); nothing here is built yet._

A smoke test is a short, structured list of steps that checks that a piece of software works
from the outside: start the server, wait for it, hit an endpoint, open a page, click a button,
look for a word. Today an agent writes those steps as prose at the end of its work ("to smoke
test this: run X, open Y, click Z") and the user does them by hand, every time, for every
feature. This feature makes the steps a file the agent can write, Yardsort can run, and the user
can watch, in the app, step by step, with every output, screenshot and reason for a failure kept.

The flow it is built for:

1. The user, in a workspace, asks the agent: _run a smoke test on this._
2. The agent writes `.yardsort/tests/login.yaml` in the worktree, checks it with
   `ys test validate`, and says: _I wrote a smoke test with six steps. Run it?_
3. The user says yes. The agent runs `ys test run login`. The first time Yardsort sees this
   file, the app shows the steps and asks the user to approve them; from then on the same file
   runs without asking.
4. The run opens as a tab in the workspace, beside the agent's terminal: each step with its
   status, the one running with its live output, a failed one with what it expected, what it got,
   and the screenshot it took.
5. The agent, which ran `ys test wait`, gets the same result in its terminal: which step failed
   and why. It fixes and runs again. The user watches, or does not.

What it is not: a unit test runner, a CI system, or a replacement for the project's own test
suite. It is the layer above them — the ten-minute check a person does before saying "done" —
made repeatable. The name is **smoke tests**, not "automated tests", because to a developer
"automated tests" means the suite that already exists, and this is the thing that was still
manual.

## Decisions

Six questions were put to the user on 2026-10-30. The answers, and what follows from each:

1. **The name is smoke tests.** `ys test`, `.yardsort/tests/`, **Tests** in the sidebar. Not
   "automated tests": to a developer that is the suite the project already has, and this is the
   layer that was still manual.
2. **The files live in the repository.** Workflows are generic — the same review for every
   project — so they are the user's. A smoke test is about one project's code, so it is the
   repository's, with the approval gate below. The user's words: workflows are more generic,
   these are specific to a project.
3. **`shell` steps stay**, with the no-interpolation rule (decision 10).
4. **Approval is per file content**, recorded against the hash. The user leaned to per test id;
   the recommendation was content, because per id lets a file approved once be edited by an
   agent, a rebase or a pull and run unseen, which is the one thing the gate exists to stop. Per
   content costs one more look when a file changes, and the dialog shows only what changed.
5. **Browser steps are in version one**, driven over the DevTools protocol against a Chromium
   found on the machine, so nothing is installed. Playwright can be a later `runner`.
6. **A run shows in a workspace tab, and the Tests view lists everything.** The tab is beside
   the agent whose work it checks; the view is where a developer sees every test and every run,
   filters them, and opens any from there.

The design decisions that follow from those, and from the code as it is:

7. **A file from the repository never runs without the user having seen it.** This is the
   first feature to take a file from a repository that says what to run.
   [Workflows](21-workflows.md) refused that, and [project automation](03-architecture.md)
   refused it, for one reason: a cloned repository must not get to run commands on the user's
   machine. The reason still holds, and the answer is not
   to move the files but to gate the run. A test is about the code it sits beside — a feature's
   smoke test belongs on the feature's branch, gets reviewed with it, and reaches the next
   machine and the next person with it — and an agent writes it the way it writes any other
   file, in the worktree it is standing in. So the file is the repository's, and the **approval**
   is the user's: before a test runs, the app shows its steps, and the user says yes. The
   approval is recorded against the file's content hash, per project, so an unchanged file runs
   at once next time and a changed one — by an agent, a rebase or a pull — is shown again. See
   [Trust](#trust).
8. **A test is a sequence, not a graph.** Setup, steps, teardown, in the order written, stopping
   at the first failure, with teardown always run. Smoke tests are stories; a DAG would be a
   second thing to learn for no step that needs it. The engine is therefore its own small thing,
   not the workflow engine, though it shares the workflow's parser machinery, its template
   language, its runs-as-rows model and its presence rule.
9. **Steps are typed, and the types are few.** `run` (a program and its arguments), `shell` (a
   line for the user's shell, for pipes), `start` (a program kept running for the rest of the
   test), `wait_for` (a port, an address, a file or a command, until it is ready), `http` (a
   request with expectations), `browser` (a short list of page actions over a Chromium on the
   machine), and `manual` (a step a person does, with **Pass** and **Fail** in the app). Nothing
   else in version one. See [Steps](#steps).
10. **Variables never reach a shell string.** `{{ … }}` is allowed in every value except a
    `shell` line; a `shell` step gets the run's variables as environment variables instead. So the
    one place where an interpolated value could become a command is the one place nothing is
    interpolated. `run` and `start` take the program and each argument as separate values, as
    everything Yardsort spawns does.
11. **Every run gets a free port.** `{{ run.port }}` is a port Yardsort found free when the run
    started. Two workspaces running the same test at once — the normal case in Yardsort — would
    otherwise fight over `3000`. A test starts its server on `{{ run.port }}` and points every
    later step at it.
12. **Secrets and machine-specific values come from the project's test environment, not the
    file.** Settings → Projects → _Test environment_ is a list of names and values kept in the
    profile's SQLite store, never in the repository; a test refers to them as `{{ env.NAME }}`.
    The approval dialog says which names a file uses and which of them are not set.
13. **A run is rows, carried out by the app, with `ys` as a second door.** As with workflows: the
    app runs tests, and `ys test run` queues a row the app picks up within a second, refusing when
    the app is closed. The processes a test starts belong to the app's runner thread, which owns
    them for the length of the run and kills what is left at the end. The daemon does not run
    tests: a test is a thing to watch, and the window is where it is watched.
14. **Output lives on disk, bounded; verdicts live in the store.** Each step's stdout, stderr and
    screenshots are files under `<data dir>/test-runs/<run id>/`; the rows hold the status, the
    duration, the exit code, the expectation that failed in words, and the paths. Runs older than
    the retention setting (default 30 days, or the newest 200 runs per project) are pruned with
    their folders.
15. **The agent is told, in its terminal, by `ys test wait`.** Nothing is typed into an agent
    that did not ask. The agent that started a run waits on it with `ys test wait <run>`, which
    returns when the run ends and prints the failing step, its reason and the last lines of its
    output. A person who started a run from the app has **Send result to agent**, which pastes the
    same summary into the workspace's agent once it is quiet, by the workflow driver's rule.
16. **Workflows get a `run_test` step.** It starts a test run in the workflow's workspace and
    ends when the run does, succeeding or failing with it. That is how "when the agent is done,
    smoke test it and tell the agent what broke" becomes a workflow, and how a scheduled smoke
    test becomes a matter of the scheduled trigger workflows do not have yet, not of anything
    here.

## The file

```yaml
# .yardsort/tests/login.yaml
id: login
name: Log in and see the dashboard
description: Starts the dev server, logs in as the seeded user, checks the dashboard loads.
version: 1

inputs:
  - id: user
    kind: text
    label: Who to log in as
    default: demo@example.test

setup:
  - id: server
    action: start
    program: bun
    args: [run, dev, --port, "{{ run.port }}"]
    env:
      DATABASE_URL: "{{ env.TEST_DATABASE_URL }}"
  - id: ready
    action: wait_for
    url: "http://localhost:{{ run.port }}/health"
    timeout: 60s

steps:
  - id: health
    action: http
    get: "http://localhost:{{ run.port }}/health"
    expect:
      status: 200
      json: { status: ok }

  - id: seed
    action: run
    program: bun
    args: [run, seed, --user, "{{ inputs.user }}"]
    expect:
      exit: 0
      stdout_contains: seeded

  - id: sign_in
    action: browser
    screenshots: on_failure
    do:
      - goto: "http://localhost:{{ run.port }}/login"
      - fill: { selector: "input[name=email]", text: "{{ inputs.user }}" }
      - fill: { selector: "input[name=password]", text: "{{ env.TEST_PASSWORD }}" }
      - click: "button[type=submit]"
      - expect_url: "**/dashboard"
      - expect_text: "Welcome back"
      - screenshot: dashboard

  - id: looks_right
    action: manual
    prompt: Does the dashboard chart animate in, with no flash of empty axes?

teardown:
  - id: stop
    action: run
    program: bun
    args: [run, db:reset]
```

### Schema

| Key           | Rule                                                                                                                                                   |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`          | `[a-z0-9-]+`, unique within the folder; the file name should match. Two files with one id: the first by name is used, the second listed with the clash |
| `name`        | Required                                                                                                                                               |
| `description` | Optional                                                                                                                                               |
| `version`     | `1`. A later version is listed, not run, and says to update Yardsort                                                                                   |
| `inputs[]`    | As workflows: `id`, `kind` (`text`, `choice` with `options`), `label`, `required`, `default`. No `harness` kind: a test starts no agent                |
| `setup[]`     | Optional. Steps run before `steps`; a failure here fails the run before any step                                                                       |
| `steps[]`     | At least one                                                                                                                                           |
| `teardown[]`  | Optional. Always run, in order, even after a failure or a cancel, each with its own result; a failure here is reported but does not change the verdict |
| `timeout`     | Optional, whole run, default `30m`. Each step has its own, default `5m`; `manual` has none                                                             |

Every step has `id` and `action`, then the action's own keys. Unknown keys are mistakes, as in
workflows. The parser is the workflow parser's machinery (`serde-saphyr` with spans, one raw step
struct checked against its action, problems with line and column), moved into a shared module
rather than copied.

### Steps

| Action     | Keys                                                                                                                  | Ends when                                                                                                      | Produces                           |
| ---------- | --------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| `run`      | `program`, `args[]`, `cwd` (relative to the workspace), `env{}`, `stdin`, `expect{}`, `timeout`                       | The process exits. Succeeds when every expectation holds; with no `expect`, exit `0`                           | `exit`, `stdout`, `stderr`         |
| `shell`    | `line`, `cwd`, `env{}`, `expect{}`, `timeout`                                                                         | As `run`. The line goes to the login shell on Unix (`$SHELL -lc`) and to PowerShell on Windows                 | `exit`, `stdout`, `stderr`         |
| `start`    | `program`, `args[]`, `cwd`, `env{}`                                                                                   | At once. The process runs until teardown, where it is stopped (TERM, then KILL after 10 s; the job on Windows) | `pid`, `stdout`, `stderr` (so far) |
| `wait_for` | One of `port`, `url` (2xx), `file`, or `program`+`args` (exit 0); `interval` (default `1s`), `timeout`                | The condition holds, or `timeout` → fails                                                                      | `waited`                           |
| `http`     | One of `get`, `post`, `put`, `delete`, `patch` with the address; `headers{}`, `body` or `json`, `expect{}`, `timeout` | The response arrives. Succeeds when every expectation holds; with no `expect`, status 2xx                      | `status`, `body`, `headers`        |
| `browser`  | `do[]` (below), `screenshots` (`on_failure`, the default, `every_step`, `never`), `viewport`, `timeout`               | The last action succeeds, or one fails                                                                         | `url`, `title`                     |
| `manual`   | `prompt`                                                                                                              | A person presses **Pass** or **Fail** in the app, with an optional note; `ys test runs` shows it waiting       | `note`                             |

**Expectations** (`expect`) for `run` and `shell`: `exit` (a number, or `nonzero`),
`stdout_contains`, `stdout_matches` (a regex), `stderr_empty`, `stdout_json` (a path and a value,
`{ path: status, equals: ok }`). For `http`: `status` (a number or a range such as `2xx`),
`json` (a mapping that must appear, key by key, in the response), `body_contains`,
`body_matches`, `header` (`{ name, contains }`). Each expectation that fails is a line in the
step's note: _expected status 200, got 502_. All are checked; the note lists every one that
failed.

**Browser actions** (`do[]`), each one key: `goto`, `click`, `fill` (`selector`, `text`), `press`
(a key, to the focused element), `select` (`selector`, `value`), `wait_for` (a selector, or
`{ text }`), `expect_text`, `expect_visible`, `expect_hidden`, `expect_url` (a glob),
`expect_title`, `screenshot` (a name). Selectors are CSS, or `text=…` for an element by its
text. Every action waits up to the step's `timeout` for its selector to exist and be visible
before acting, so no `sleep` is needed and none is offered. The browser is a Chromium-family
binary found on the machine — Chrome, Chromium, Edge, in that order, on `PATH` and in each
platform's usual places; Settings → Projects can name one — driven over the DevTools protocol
from the core, headless by default with a `headed: true` for watching. Nothing is installed
and no Node is needed. A test that uses `browser` with none found is listed as _needs a browser_
and refused at the door, naming what was looked for.

**`manual`** is what makes the file honest. A smoke test an agent writes today is half commands
and half "then check that it looks right". The half a machine cannot judge stays in the file as a
step a person does, and the run waits for them with the question on the screen, so the checklist
is in one place and its answers are in the record.

### Variables

The workflow template module (`{{ path }}`, dotted lookup, no filters, `\{{` for a literal),
checked at load time against these namespaces:

| Namespace   | Fields                                                                                                                        |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `project`   | `name`, `root`                                                                                                                |
| `workspace` | `name`, `branch`, `path`                                                                                                      |
| `run`       | `id`, `port` (one free port, picked when the run starts), `dir` (the run's own scratch folder, removed with the run's output) |
| `inputs`    | One field per declared input                                                                                                  |
| `env`       | The project's test environment, by name. An unset name is a validation note at approval and an empty string in the run        |
| `steps`     | `<id>.<produced field>`, for steps earlier in the file, including setup from steps                                            |

A `shell` line may not contain `{{`; it is a validation error that says to use `$YS_…` instead.
A `shell` step's process gets `YS_RUN_PORT`, `YS_RUN_DIR`, `YS_WORKSPACE_PATH`,
`YS_INPUT_<ID>` and `YS_ENV_<NAME>`, and so does every `run` and `start` process, so a script a
test calls can find them too.

## Trust

The rule from project automation stands: a file in a repository never enables a command. What
enables a test run is an **approval**: the user, in the app, looking at the steps of this exact
file and pressing **Run**. Migration `0019` adds `test_approvals (project_id, test_id,
content_hash, approved_at)`. A run is queued as `awaiting_approval` when no approval matches the
file's hash; the app shows it — a card in the workspace and the sidebar's count — with every
step rendered as words (_run `bun run seed --user demo@example.test`_, _GET
http://localhost:{{ run.port }}/health, expect 200_), the environment names it uses and
which are not set, and **Run** or **Don't run**. Approving records the hash and starts the run.
Changing the file, in any way, needs a new approval: the dialog then shows what changed.

Settings → Projects → _Smoke tests_ has one switch per project, **Ask every time**, for people
who want to see every run's steps regardless; off by default.

`ys test run` from an agent's terminal therefore cannot run anything the user has not seen. It
says so: _queued; waiting for you to approve it in Yardsort_, and `ys test wait` waits through
the approval. A refusal in the dialog ends the run as `refused`, which `wait` reports in words.
There is no `--approve` flag and no `ys` command that approves, for the reason `ys memory` has
no approve: an agent can run anything `ys` offers.

What a test can do once approved is what the user can do: it runs as the user, in the worktree,
with the login environment. Yardsort adds no sandbox and claims none. The approval dialog is the
sandbox, and the words in it are written to be read.

## The run

Migration `0019_smoke_tests.sql` (numbered 0016 in the proposal, before main took three):

- `test_runs`: id, project id, workspace id and a copy of its name, test id and name, the **file
  as it was** when the run was asked for and its hash, inputs as JSON, `status` (`awaiting_approval`,
  `queued`, `running`, `passed`, `failed`, `refused`, `cancelled`, `errored` — a run that could
  not be carried out, as distinct from one whose steps failed), `requested_by` (`app`, `cli`,
  `workflow`), the requesting session id when `ys` was run from an agent's terminal (the launch
  environment is the witness, as `ys memory propose` finds it), `port`, `error`, `created_at`,
  `started_at`, `ended_at`, and `output_dir`.
- `test_step_runs`: run id, step id, phase (`setup`, `steps`, `teardown`), position, action,
  `status` (`pending`, `running`, `waiting` for `manual`, `passed`, `failed`, `skipped`,
  `cancelled`), `started_at`, `ended_at`, `exit`, outputs as JSON (the produced fields; `stdout`
  and `stderr` as paths, not text), `note` (every expectation that failed, in words, or why it
  was skipped), and `screenshots` as JSON paths.
- `test_approvals` as above.
- `project_test_env (project_id, name, value)`, and the `ask_every_time` and browser path
  settings in the existing per-project settings JSON.

A partial unique index on `(test_id, workspace_id)` over the live statuses makes one run of a
test per workspace at a time hold across two processes, as it does for workflows.

**The runner** (`yardsort_core::smoke::runner`) is a thread per run in the app, started by a
driver that, like the workflow driver's, ticks every second and finds queued rows. One thread per
run rather than a planned step-mover, because a step here is a child process whose output must be
read as it comes, and a `manual` step is the only one that waits on anything but a process. The
thread:

- resolves the variables, picks the port, makes `output_dir`;
- runs setup, steps, teardown in order, each step a conditional row move as in workflows, so a
  cancel from `ys` is seen between steps and acted on;
- pumps each process's stdout and stderr to files, and tells the window every half second that a
  step's output grew (`TestRunChanged`, with the run id and the step), so the tab tails the file;
- stops at the first failing step in `setup` or `steps`, marks the rest `skipped`, then runs
  teardown;
- on cancel, kills the running step's process tree (a process group on Unix, a job object on
  Windows), marks it `cancelled`, and still runs teardown;
- stops every `start` process at the end, however the run ended.

A restart finds runs `running` and fails them with a note saying Yardsort stopped during them;
their `start` processes are orphans by then and are killed by pid if still alive and still the
same process start time. A run `awaiting_approval` survives a restart. The app holds the presence
lock as before; `ys test run` tries it and refuses without it.

**Browser steps** run through `yardsort_core::smoke::browser`: launch the found binary with
`--headless=new --remote-debugging-pipe` and a profile folder under `run.dir`, speak the DevTools
protocol over the pipe (the `chromiumoxide` crate, or a small client of our own over the dozen
methods needed — to be decided in slice 4 by trying both), one page per step, a screenshot into
`output_dir` on failure, or after every action when asked. The browser is closed at the end of
each step; a test that needs state across steps keeps it in one step's `do[]`.

## The command line

```
ys test list [--workspace <name>]        # the tests in the worktree's .yardsort/tests, with readiness
ys test show <id>                        # the file that would run; --json the parsed form
ys test validate <file|->                # file:line:column: message, exit 1 if any; no profile needed
ys test example                          # a complete, valid example file on stdout, to start from
ys test guide                            # the file format, every action, key and variable, for an agent
ys test run <id> [--workspace <name>] [--input user=…]   # queue a run; Yardsort must be open
ys test wait <run> [--timeout 20m]       # block until it ends; print the verdict; exit 0 passed, 1 failed, 2 other
ys test runs [--workspace <name>] [--run <id>]   # history, or one run step by step
ys test log <run> [--step <id>] [--stderr]      # a step's output; --json gives paths and screenshots
ys test cancel <run>
```

Inside a workspace's folder, or in an agent's terminal there, `--workspace` is found as
`ys workflow run` finds it. `run` checks the file, every input, that the file is in the
worktree's `.yardsort/tests`, and that a `browser` test has a browser; it refuses with Yardsort
closed. It prints the run's id, whether it started or is waiting for approval, and returns at
once. `wait` is what an agent calls next; its output is written for an agent to act on:

```
failed  login  (3 of 6 steps passed, 1m 12s)
  sign_in  browser  failed after 31s
    expect_text "Welcome back": not found within 30s; the page title is "Sign in"
    screenshot: /home/you/.local/share/dev.yardsort.app/test-runs/8c1d…/sign_in-failure.png
    last lines of the server (setup step `server`):
      [vite] 500 /api/session: TypeError: cannot read 'id' of undefined
```

`--json` on everything, as elsewhere.

## The UI

**The run tab.** A test run opens as a tab in its workspace's tab strip, beside the terminals,
named after the test with a status dot: the steps down the left in their three phases, the
selected step's output on the right, tailed live, with its expectations and which failed, its
screenshots inline, and its duration. A `manual` step shows its prompt with **Pass**, **Fail**
and a note. The tab's header has **Cancel**, **Run again** and **Send result to agent**. The
tab is a view of rows and files; closing it loses nothing, and the workspace's **⋯** menu and
the Tests view reopen any run.

**The approval card.** A run waiting for approval appears at the top of its workspace's center
panel, above the tabs, as a card: the test's name, its steps as words, the environment names it
uses and which are unset, what changed since the last approved version when there was one, and
**Run** / **Don't run**. The sidebar shows the count of runs waiting on the workspace's row.
Keyboard: Mod+Enter approves, Escape refuses, as the composer's shortcuts go.

**Run smoke test…** in a workspace's **⋯** menu lists the worktree's tests with their readiness,
asks the inputs, shows the steps as the card does, and runs. A test with no approval yet is
approved by this same dialog: seeing the steps and pressing **Run** is the approval.

**The Tests view.** A row at the top of the sidebar beside Pull requests and Tasks, and in the
command palette: every project's tests in one list, from each project's own checkout (the main
branch's, the "standard" ones) and from every live workspace's worktree, so a test that exists
only on a feature branch is listed too, marked with its workspace. Each row shows the last run's
verdict, when, and in which workspace. Filters, in the header as the Pull requests view has
them: project, workspace, verdict of the last run (passed, failed, never run, waiting for
approval), and whether the test needs anything the machine lacks (a browser, an unset
environment name). A second tab in the view, **Runs**, is every run across projects, newest
first, with the same filters plus who asked (you, an agent, a workflow). A test opens to its
file, read-only (it is the repository's; edit it in the worktree), its run history across
workspaces, and **Run in…** with a workspace to pick. A run, from either tab, opens its run tab
in that workspace. The view is read from the repositories on open and on **Refresh**, like
Tasks; the sidebar row's number is the runs waiting for approval.

**Settings → Projects** gains _Smoke tests_: the test environment (name, value; values shown
masked), **Ask every time**, the browser to use, and how long to keep runs.

## Agents

An agent learns that this exists the way it learns about `ys memory` and `ys task`: in the first
message. The launch adds one paragraph, when the project's checkout has a `.yardsort/tests`
folder or the project has the test environment set:

> This project has smoke tests in `.yardsort/tests/`: `ys test list` lists them and
> `ys test run <id>` runs one in this workspace, with the result from `ys test wait`. To write
> one for what you build, `ys test guide` prints the format; put it beside the others, check it
> with `ys test validate`, and ask before running it: the user approves each new file in
> Yardsort.

`ys test guide` is `smoke::GUIDE`, a constant that doubles as the system prompt for **Describe
it** below, so what an agent is told and what the validator accepts cannot drift. It is tested
against the parser: every example in it must validate.

**Describe it** — the Workflows view's box that writes a file from a description — is offered in
the Tests view too, through the same Drafting writers, with `smoke::GUIDE` as the instruction and
the example file as the example. The result goes to `.yardsort/tests/<id>.yaml` in the chosen
workspace's worktree only when the user presses **Save**, and then is an ordinary uncommitted
change in the Changes panel. The usual path, though, is the agent in the workspace writing the
file itself, because it knows the feature.

**The `run_test` workflow action**: `test` (an id), `inputs{}`, `timeout`. The workflow driver
queues a test run with `requested_by: workflow`, waits for its row to end, and the step passes
or fails with it, leaving `run`, `verdict` and `summary` (the text `ys test wait` prints) for
later steps. A `send_to_session` after it can hand `{{ steps.smoke.summary }}` to `origin`.
Approval is needed as anywhere; a workflow run waits for it. The built-in workflow
**Smoke test, then tell the agent** is two steps: `run_test` on an input of kind `test`, and
`send_to_session` to `origin` with the summary — the shape of "automate it later" once a trigger
other than manual exists.

## Three platforms

- **Process trees.** `start` steps spawn in a new process group (Unix) or a job object
  (Windows) so that a dev server's children die with it; project automation's setup command
  already needed the ten-minute kill and gets the same helper. `just lint-windows` covers the
  crate.
- **Shells.** `shell` is `$SHELL -lc` on Unix, with the login environment Yardsort already
  resolves, and `pwsh -NoProfile -Command` (falling back to `powershell`) on Windows. The guide
  says to prefer `run` for a test that must work everywhere.
- **Browsers.** Looked for on `PATH`, then `/usr/bin/{google-chrome,chromium,chromium-browser}`,
  `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`, Edge's and Chrome's usual
  folders under `Program Files` on Windows, then the setting. Headless over a pipe works on all
  three; the manual checklist gets a row for each.
- **Ports.** Binding `127.0.0.1:0` and releasing it is how the port is found; a `wait_for port`
  checks the same address. IPv6-only listeners are the test's problem, and the guide says so.
- **Paths.** `cwd` is joined to the worktree and must stay inside it; the output folder is under
  the data directory, which is per platform already.

## Slices

Each is one pull request with its docs, tests and changelog line, each standing on its own.

1. **The file and the core.** ✅ `yardsort_core::smoke`: the schema types, the parser sharing the
   workflow's spanned-YAML machinery (moved into `crates/core/src/yaml.rs`), the validator with
   line and column, the variable checker over the new namespaces, the `shell`-has-no-braces
   rule, the catalog of a worktree's `.yardsort/tests`, `ys test list | show | validate |
example | guide`, and `docs/guide/smoke-tests.md` with the format. Fixtures: one file per
   validation error.
2. **The runner.** ✅ Migration 0019; the run store and the queue with the approval state; the
   runner thread with `run`, `shell`, `start`, `wait_for` and `http` (`reqwest`, blocking, moved
   from the app's dependencies to the core's); setup, steps, teardown; the port; output files;
   cancellation and the process-tree kill on all three platforms; a restart mid-run; pruning.
   `ys test run | wait | runs | log | cancel`, with `run` saying _awaiting approval_ and nothing
   to approve it. Tests with real processes in a temp data directory: a script that listens on
   `{{ run.port }}`, a `wait_for` on it, an `http` against it, a failing expectation, a cancel
   mid-step, a teardown that still runs.
3. **The UI.** The approval card and its count in the sidebar; the run tab with live output,
   expectations and `manual` steps; **Run smoke test…** in the workspace menu; **Send result to
   agent**; Settings → Projects → Smoke tests with the environment. Testing Library for the card's
   words, the tab's step list and the manual step's buttons; the tailing as a hook with a fake
   file.
4. **Browser steps.** `smoke::browser` over the DevTools protocol, the finder, the action
   vocabulary with waiting, screenshots on failure and on demand, the headed mode; the example
   file gains its `sign_in` step. Tested against a page served by the test itself from a temp
   folder, on all three platforms in CI, skipped with a named reason where no browser is found.
5. **Agents and workflows.** The launch paragraph; `smoke::GUIDE` tested against the parser; the
   `run_test` action in the workflow engine and driver, and the built-in **Smoke test, then tell
   the agent**; Describe it in the Tests view.
6. **The Tests view, docs and pictures.** The sidebar row, the view with its filters, the
   **Runs** tab, history across workspaces, **Run in…**; the README's highlight and section, the website's section and `ys`
   line, the quick start's mention, `docs/README.md`, `website/src/lib/docs.ts`, screenshots on
   the throwaway profile with a demo test that needs nothing but a shell, and [08] gains §26.

Slices 1 and 2 work with no window but for approval, which is the point: a test queued by a
workflow or, later, a schedule needs nothing the window has except a person to approve a new
file once.

## Slice 1: what shipped

`yardsort_core::smoke` reads and checks files; nothing runs yet. What changed from the proposal
above, and why:

- **The shared machinery is two modules at the crate root,** `yaml.rs` (the problem type, serde's
  errors tidied, the lenient heading read, positions, durations, names, "did you mean") and
  `template.rs` (moved from `workflow/`), rather than one `yaml.rs`. The workflow module
  re-exports both, so nothing that used them moved. The spanned raw structs stay with each parser:
  what a key means is each file's own business.
- **Defaults are explicit in the parsed form.** `expect.exit` is `0` and `expect.status` is `2xx`
  unless the file says, and the parsed `ProcessExpect` and `HttpExpect` carry that, so the runner
  reads one shape. The proposal's "with no `expect`, exit 0" became "exit 0 unless said", which
  is what people mean when they write only `stdout_contains`.
- **`json` bodies and expectations are JSON text,** not a tree: the YAML mapping is written out
  as one line of JSON, with any `{{ variables }}` still inside it to render at run time. The
  generated bindings have no type for an arbitrary tree, and the runner wants text anyway.
- **A wrong key in a browser action is reported once.** `fill: { selector, value }` says `fill`
  has no `value`; it does not also say `text` is missing, since the one is the other misnamed.
- **The run's variables reach `shell` as `YS_…` environment variables,** and every `run` and
  `start` process gets them too, so a script a test calls can read them. The names are in the
  guide and the validator's message.
- **The guide is tested.** Every ` ```yaml ` block in `smoke::GUIDE` is a whole file, and a
  test parses each; a block that stops validating fails the build.
- **`ys test list` with no tests** says where the folder is and how to start one, rather than
  printing an empty table.
- **Not here:** readiness beyond "ready" or "N problems". Whether a `browser` test has a browser
  on the machine is the finder's answer, which is slice 4; `--json` says `usesBrowser` so a
  caller can tell.

## Slice 2: what shipped

`yardsort_core::smoke::{runs, runner, exec, report}`, `store/smoke.rs`, `process.rs`, migration
0019, and `ys test run | wait | runs | log | cancel`. What changed from the proposal above, and
why:

- **A cancel of a running run is a request, not a state.** `cancel_test_run` ends a run that
  has not started; one that is running gets `cancel_requested_at`, and the runner, which looks
  at the store every quarter second while a step runs, stops the step, marks what was not
  reached `cancelled`, runs the teardown and ends the run as `cancelled`. A status that changed
  under the runner would have refused the teardown's own moves, which is the one thing a cancel
  must not skip. `ys test cancel` says which of the two happened.
- **One thread per run, started by a driver the workflow driver's thread ticks,** under the
  same profile lock; the runner has no thread of its own to wake. Each thread opens its own
  store connection, as `ys` would.
- **A `start` process that ends before the run does fails the run**, at the step then running
  or between steps, with its exit code and its last lines on the `start` step; the proposal had
  not said. A server that died is the usual reason the next step fails, and the note says so
  instead of leaving a connection refused.
- **A step's stream is capped at 16 MiB** and the step fails past it (06 #46). A `start`
  process is held to the same cap for as long as it runs, by the watch every step and every
  `manual` wait asks: past it the server is stopped, its step fails and so does the run, the way
  a server that died does.
- **`cwd` is resolved before use.** The parser keeps `..` and absolute paths out of it; the
  runner canonicalises the workspace and the folder and refuses one that leads outside, which
  a symbolic link an agent added would, under a `cwd` the approved file shows as a plain folder.
- **Nothing a step waits on is deaf to the run.** A `run` step's `stdin` is written from a
  thread of its own, so a program that never reads it cannot hold the step past its deadline;
  an `http` request, and a `wait_for url` probe, are waited for the same way, with the stop
  signal looked at every tenth of a second meanwhile. A cancel that lands while a `manual` step
  waits keeps its reason: the step and the run are `cancelled`, not failed.
- **Every way out of a run is the same way out.** The run's servers, its scratch folder and
  its row are held by a guard: a store error on the way — the database locked longer than the
  store waits — or a panic still stops the servers, fails the step it was on, skips the rest
  and ends the run as `errored` with the reason, so the next run of that test is not refused
  as one already going. `kill_orphan` likewise kills what is left of the group after its grace
  period whether or not the leader is still in it: `npm run dev` exits on TERM, the node it
  started may not.
- **Orphans are found by start time.** A `start` step records its pid and, on Linux and macOS,
  the process's start time; a restart kills the group only when both still match. Windows
  needs none of it: the job object ends its processes when the app's handle closes.
- **The runner sees `manual` steps through the store:** `waiting` until `answer_test_step`
  moves it, which the app's command does. Nothing in `ys` can.
- **Expectations are rendered.** `stdout_contains` and `stdout_json.equals` may name
  `{{ run.port }}` or what an earlier step left; the proposal only said texts in steps could.
- **`{{ steps.<id>.stdout }}` is the text,** the first 64 KiB of it with trailing newlines
  dropped, while the row keeps the path. A test reads a command's answer into the next step the
  way a shell's `$(…)` would.
- **The run's id is in the environment too,** as `YS_RUN_ID`, beside the five the proposal
  named.
- **`project_automation`'s setup command uses the same process group,** so a setup stopped for
  taking ten minutes takes what it started with it, which the old kill did not.
- **The app has the IPC for slice 3 already:** `test_list`, `test_runs`, `test_run_steps`,
  `test_run_start`, `test_run_approve`, `test_run_refuse`, `test_run_cancel`,
  `test_step_answer`, `test_env_get` and `test_env_save`, with `TestRunsChanged` and
  `TestRunOutput` events. The approval card, the tab and the Tests view are the pixels on top.
- **Not here:** browser steps, which `run` refuses with the step's name; the `requested_by:
workflow` path, which is slice 5; and the **Ask every time** switch, which is a setting the
  slice 3 dialog needs before anything reads it. Until slice 3 there is nothing in the window
  that approves, so a run started from `ys` waits; the changelog says so.

## Not in version one, on purpose

- **Driving native desktop applications.** "Open the app and click around" for a Tauri or
  Electron app is a different problem (no DevTools pipe without the app's cooperation, a
  different one per toolkit); for the web view inside them it is the WebKit or Chromium inspector,
  which is a later `browser` target, not a new step. Version one smoke-tests a desktop app by
  starting it, checking what it does from the outside, and asking the person with `manual`.
- **Playwright as a runner.** The vocabulary is deliberately a subset of what Playwright's would
  be, so that `runner: playwright` can come later for projects that have it, without a second
  file format.
- **Schedules and forge triggers**, which are the workflows' question, answered there when it is.
- **Running a test on another machine, or in a container**; the [remote proposal](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md)
  is where a headless host would run one.
- **Assertions on screenshots** (visual diffs), and an agent's judgment of a screenshot. A
  `manual` step is the honest version of both for now.
- **Retries, conditionals, loops, parallel steps.** A smoke test that needs them is a test suite.
- **Yardsort's own manual checklist as a test.** [08](08-manual-checklist.md) could become a
  test with many `manual` steps, which is a good first dogfooding target after slice 3, and
  nothing in version one depends on it.

## Open questions

[44–47 in open questions](06-open-questions.md#smoke-tests), recorded there on 2026-10-30:

44. **Should an approval follow a rebase?** A file whose content is unchanged but whose path moved
    with a rename keeps its approval under the hash; a file that changed only in a comment does
    not. Whether to hash the parsed form instead of the text is a slice 2 finding.
45. **Should a run tell the agent without being asked?** When `ys test run` came from an agent's
    terminal, the result could be typed into that agent once quiet, as `send_to_session` does.
    Version one does not: the agent asked by running `wait`, and an agent that did not is doing
    something else. Revisit after dogfooding.
46. **How much output to keep per step.** Unbounded files with pruning by age is the proposal; a
    dev server that logs a line per millisecond for an hour fills a disk. A per-step cap, with
    the head and the tail kept, is likely the slice 2 answer.
47. **Should `wait_for url` accept a self-signed certificate?** Local HTTPS dev servers are
    common. `insecure: true` on `wait_for` and `http`, off by default, is the likely answer.

## Documentation this touches

`docs/guide/smoke-tests.md` (new), `docs/guide/cli.md` (`ys test`), `docs/guide/workflows.md`
(`run_test`, the second built-in), `docs/guide/settings.md` (Projects → Smoke tests),
`docs/guide/terminals-and-sessions.md` (the run tab, the approval card),
`docs/guide/workspaces.md` (**Run smoke test…**), `docs/README.md`, `docs/quick-start.md`,
`website/src/lib/docs.ts`, the landing page's copy, `README.md`, `CHANGELOG.md`,
[02-ux](02-ux.md) (the sidebar row, the card, the tab), [03-architecture](03-architecture.md)
(the runner and the trust line), [05-roadmap](05-roadmap.md) (M28),
[06-open-questions](06-open-questions.md) (44–47), [08](08-manual-checklist.md) (§26).
