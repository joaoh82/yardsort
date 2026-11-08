# Smoke tests

A smoke test is a short, structured list of steps that checks a piece of software from the
outside: start the server, wait for it, hit an endpoint, open a page, look for a word. It is the
ten-minute check you do before saying "done", written down so an agent can write it and Yardsort
can run it. A test is a YAML file in your repository, `.yardsort/tests/<id>.yaml`, so it belongs
to the code it checks: an agent writes one for the feature it built, in the worktree it is
standing in, and the test travels with the branch.

**This release reads, checks and runs the files, and shows the runs.** `ys test validate` names
every mistake at its line and column, `ys test list` shows a workspace's tests, `ys test guide`
tells an agent how to write one, and `ys test run` queues a run that Yardsort carries out, step
by step, with the verdict from `ys test wait`. A file from a repository never runs unseen: a run
of a new or changed file waits until you approve it, on a card at the top of its workspace. The
run opens as a tab beside the workspace's terminals, with every step, its output as it comes,
and **Pass** / **Fail** for the steps a person does; **Run smoke test…** in a workspace's menu
starts one from the app, and **Tests** in the sidebar lists every project's tests and runs in
one place. See [Running a test](#running-a-test) and [In the app](#in-the-app).

## Where tests live

Every `.yaml` or `.yml` file directly in a worktree's `.yardsort/tests` folder is a test. Each
workspace has its own copy, since each worktree is its own checkout: a test an agent wrote on a
feature branch exists in that workspace until the branch is merged. `ys test list` reads the
folder of the workspace it is run in, or of `--workspace <name>`.

A file that does not check out is still listed, under the id it says or its file name, with its
problems. Two files with one `id`: the first by file name is used, and the second is listed with
the clash.

A test says what to run on your machine, and it comes from a repository. That is why nothing
here runs unseen: Yardsort shows a new or changed file's steps and asks you first, once per
version of the file. See [The approval card](#the-approval-card).

## An example

`ys test example` prints this file:

```yaml
id: health
name: The server starts and answers
description: Starts the dev server on a free port, waits for it, and checks its health endpoint.
version: 1

inputs:
  - id: greeting
    kind: text
    label: What the home page should say
    default: Welcome

setup:
  - id: server
    action: start
    program: npm
    args: [run, dev, --, --port, "{{ run.port }}"]
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

  - id: version
    action: run
    program: node
    args: [--version]
    expect:
      stdout_matches: '^v\d+\.'

  - id: home
    action: browser
    do:
      - goto: "http://localhost:{{ run.port }}/"
      - expect_text: "{{ inputs.greeting }}"
      - screenshot: home

  - id: looks_right
    action: manual
    prompt: Does the home page load without a flash of unstyled content?
```

`setup` runs first; a failure there fails the run before any step. `steps` run in order and stop
at the first failure. `teardown`, when there is one, always runs, however the run ended, and a
`start` process is stopped at the end whatever happens. The three lists share one set of step
ids.

`{{ run.port }}` is a port that is free when the run starts. Several workspaces may run the same
test at once, and they would otherwise fight over the same port. Start the server on the run's
port and point every later step at it.

## The file

| Key           | What it is                                                                                                       |
| ------------- | ---------------------------------------------------------------------------------------------------------------- |
| `id`          | Lowercase letters, digits and `-`, like `login`. Name the file after it.                                         |
| `name`        | What the test is called in lists.                                                                                |
| `description` | Optional. One or two sentences.                                                                                  |
| `version`     | `1`. A file with a later version is listed but not run, and says to update Yardsort.                             |
| `inputs`      | Optional. What you are asked before a run starts.                                                                |
| `timeout`     | Optional. For the whole run; `30m` when left out. A whole number and a unit: `30s`, `45m`, `2h`. At most a week. |
| `setup`       | Optional. Steps run before the test itself.                                                                      |
| `steps`       | At least one.                                                                                                    |
| `teardown`    | Optional. Steps always run at the end.                                                                           |

Keys a test does not have are mistakes, not ignored. A misspelt `progam:` is reported, not
silently skipped.

### Inputs

| Key        | What it is                                                                                                |
| ---------- | --------------------------------------------------------------------------------------------------------- |
| `id`       | Starts with a lowercase letter; then lowercase letters, digits, `_` and `-`. Used as `{{ inputs.<id> }}`. |
| `kind`     | `text` or `choice`. There is no `harness` input, as a workflow has: a test starts no agent.               |
| `label`    | Optional. The question. The `id` when left out.                                                           |
| `required` | Optional. `false` when left out. An optional input left empty is empty text wherever it is used.          |
| `default`  | Optional. For a `choice`, one of its options.                                                             |
| `options`  | For a `choice` only: the answers to pick from.                                                            |

### Steps

Every step has an `id`, named the same way as an input, and an `action`. Each may have a
`timeout` (`5m` when left out), except `start`, which runs to the end of the test, and `manual`,
which waits for a person.

| Action     | Keys                                                                                                                       | What it does                                                                                                                                                                                                                |
| ---------- | -------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `run`      | `program` (required), `args`, `cwd`, `env`, `stdin`, `expect`                                                              | Runs a program with its arguments, each argument its own item of the list, and waits for it to exit. Passes when every expectation holds; with none given, when it exits 0.                                                 |
| `shell`    | `line` (required), `cwd`, `env`, `expect`                                                                                  | Runs one line in your shell, for pipes: the login shell on Unix, PowerShell on Windows. Passes as `run` does.                                                                                                               |
| `start`    | `program` (required), `args`, `cwd`, `env`                                                                                 | Starts a program and leaves it running, a dev server for example, until the end of the test, when it is stopped.                                                                                                            |
| `wait_for` | One of `port`, `url`, `file`, or `program` with `args`; `interval` (`1s` when left out)                                    | Waits until a port on localhost accepts a connection, an address answers 2xx, a file exists, or a program exits 0, asking again every `interval`. `args` goes with `program` only. Fails when the `timeout` runs out first. |
| `http`     | One of `get`, `post`, `put`, `delete`, `patch` with the address; `headers`, `body` or `json`, `expect`                     | Makes one request. `json` is sent as JSON with the matching content type. Passes when every expectation holds; with none given, on any 2xx.                                                                                 |
| `browser`  | `do` (required), `screenshots` (`on_failure` when left out, `every_step`, `never`), `viewport` (like `1280x800`), `headed` | Opens a page in a Chromium found on your machine, headless unless `headed: true`, and does each action in `do` in turn. A failing action takes a screenshot; `every_step` takes one after each.                             |
| `manual`   | `prompt` (required)                                                                                                        | Shows `prompt` to a person and waits for **Pass** or **Fail**, with an optional note. The half of a smoke test a machine cannot judge stays in the file.                                                                    |

A key that belongs to another action is reported as misplaced. `prompt` on a `run` step is an
example.

**`cwd`** is a folder inside the workspace, relative to it, like `web` or `packages/api`. No
`..`, no absolute path, no variables, and no symbolic link that leads outside the workspace: the
step fails and says where the link went.

**`env`** is a mapping of names to values. A name is letters, digits and `_`, not starting with a
digit. Values may use variables.

**A `shell` line cannot hold `{{ … }}`.** A value pasted into a shell line could become a
command, so the one place where that could happen is the one place nothing is pasted. The run's
variables are in the environment instead, and every `run` and `start` process gets them too:

| Variable               | Environment                 |
| ---------------------- | --------------------------- |
| `{{ run.id }}`         | `YS_RUN_ID`                 |
| `{{ run.port }}`       | `YS_RUN_PORT`               |
| `{{ run.dir }}`        | `YS_RUN_DIR`                |
| `{{ workspace.path }}` | `YS_WORKSPACE_PATH`         |
| `{{ inputs.<id> }}`    | `YS_INPUT_<ID>`, upper case |
| `{{ env.<NAME> }}`     | `YS_ENV_<NAME>`             |

So `line: curl -s "http://localhost:$YS_RUN_PORT/health" | jq .status` is right, and
`line: curl localhost:{{ run.port }}` is reported. Prefer `run` where there is no pipe: it works
the same on every platform.

**A step can raise a [custom event](triggers.md#custom-events)** for a trigger to hear:
`action: run`, `program: ys`, `args: [event, emit, seeded]`. `ys` knows the run from
`YS_RUN_ID`, so it is raised in the run's workspace, and a run the trigger starts counts as one
step further along a chain, which keeps tests that set each other off from looping.

### Expectations

Under `expect`, for `run` and `shell`:

| Key               | What it checks                                                                                                                                                                                |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `exit`            | The exit code: a number, or `nonzero`. `0` when left out.                                                                                                                                     |
| `stdout_contains` | The output has this text.                                                                                                                                                                     |
| `stdout_matches`  | The output matches this regular expression. One that does not compile is reported.                                                                                                            |
| `stderr_empty`    | `true`: nothing was written to standard error.                                                                                                                                                |
| `stdout_json`     | `{ path: status, equals: ok }`: the output is JSON and has this value at this path. `equals` is compared as JSON: `ok` is the string, `1` the number, `true` the boolean, and `"1"` a string. |

For `http`:

| Key             | What it checks                                                                                |
| --------------- | --------------------------------------------------------------------------------------------- |
| `status`        | The status: a number from 100 to 599, or a class like `2xx`. `2xx` when left out.             |
| `json`          | A mapping. The response is JSON and has every key of it, with the same value.                 |
| `body_contains` | The body has this text.                                                                       |
| `body_matches`  | The body matches this regular expression.                                                     |
| `header`        | `{ name: content-type, contains: json }`: the response has this header, containing this text. |

Every expectation given is checked, and the step's note lists each one that failed, as _expected
status 200, got 502_. An expectation of the wrong kind, `status` on a `run` step, say, is
reported.

### Browser actions

Each item in `do` is one key and its value. Selectors are CSS, or `text=…` for an element by its
text (which skips `script` and `style`, and prefers one you can see). Every action waits for its
selector to exist and be visible before acting, up to the step's `timeout`, so there is no `sleep`
and none is needed.

`goto` waits for the page to finish loading and fails if it never does, but an HTTP error status
(a 404 or a 500) is still a page it went to — check what the page shows with `expect_text` or
`expect_title`. `fill` sets the value the way a real keypress would, so an input a framework
controls (React and the like) sees the change. `press` sends one key to the focused element: a
single character types itself, or one of `Enter`, `Tab`, `Escape`, `Backspace`, `Delete`,
`ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`, `Home`, `End`, `PageUp`, `PageDown`, `Space` — any
other multi-letter name is reported when the file is checked.

| Action           | Value                                                                       |
| ---------------- | --------------------------------------------------------------------------- |
| `goto`           | An address                                                                  |
| `click`          | A selector                                                                  |
| `fill`           | `{ selector, text }`; an empty `text` clears it                             |
| `press`          | A key, to the focused element: a character, or a name like `Enter` or `Tab` |
| `select`         | `{ selector, value }`                                                       |
| `wait_for`       | A selector, or `{ text: … }`                                                |
| `expect_text`    | Text that must be on the page                                               |
| `expect_visible` | A selector                                                                  |
| `expect_hidden`  | A selector                                                                  |
| `expect_url`     | A pattern over the address, like `**/dashboard`                             |
| `expect_title`   | The page title                                                              |
| `screenshot`     | A name for the file                                                         |

### Variables

Any text in a step can use `{{ name }}`, except a `shell` line. Only these names exist. A name is
a dotted path and nothing else: no filters, no expressions. Anything else is reported where it is
written.

| Variable                                               | What it is                                                                                                                                                                                                                                                                                |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `project.name`, `project.root`                         | The project, and the folder of its own checkout.                                                                                                                                                                                                                                          |
| `workspace.name`, `workspace.branch`, `workspace.path` | The workspace the run is about.                                                                                                                                                                                                                                                           |
| `run.id`, `run.port`, `run.dir`                        | This run: its id, one free port, picked when the run starts, and a scratch folder of its own, removed with the run's output.                                                                                                                                                              |
| `inputs.<id>`                                          | What was given for that input.                                                                                                                                                                                                                                                            |
| `env.<NAME>`                                           | A value from the project's test environment, for passwords and anything machine-specific. Never in the file; you set them in Yardsort. `NAME` is letters, digits and `_`.                                                                                                                 |
| `steps.<id>.<field>`                                   | What an earlier step left: `exit`, `stdout` and `stderr` from `run` and `shell`; `pid`, `stdout` and `stderr` from `start`; `waited` from `wait_for`; `status`, `body` and `headers` from `http`; `url` and `title` from `browser`; `note` from `manual`. Only steps earlier in the file. |

To write `{{` literally, write `\{{`.

## Checking a file

`ys test validate <file>` checks a file without needing Yardsort to have run on this machine.
It prints each problem as `file:line:column: message` and exits 1 if there are any:

```text
$ ys test validate .yardsort/tests/login.yaml
.yardsort/tests/login.yaml:20:36: `{{ workspace.brnch }}`: `workspace` has no `brnch`. It has `name`, `branch`, `path`.
.yardsort/tests/login.yaml:31:11: A `shell` line cannot hold `{{ … }}`: a value pasted into a shell line could become a command. …
```

`-` reads the file from standard input. `--json` prints `{ valid, id, problems }`.

`ys test list` shows the tests of the workspace you are in, or of `--workspace <name>`, whether
each is ready, and where the folder is. `ys test show <id>` prints the file that would be used,
with any problems after it. `ys test example` prints the example above, and `ys test guide`
prints the format written for an agent: every action, key and variable, with whole files as
examples. See [the `ys` command line](cli.md#ys-test).

## Running a test

`ys test run <id>` queues a run of a test in the workspace you are in, or in `--workspace
<name>`, with `--input id=value` for each of the test's inputs that has no default. Yardsort
must be open: the app carries runs out, and `ys` says so and queues nothing when it is not.
The command checks the file, every input and every `{{ env.NAME }}` the test uses before
anything is queued. A test with a `browser` step needs a browser on the machine; if none is
found it is refused, saying whether the project's named browser was not there or none was found
at all, so name one under the project's smoke test settings. It prints the run's id and returns
at once.

**Approval.** A test says what to run on your machine, and it comes from a repository. So a run
of a file you have not seen — new, or changed in any way since you last approved it — is queued
as _awaiting approval_, and nothing runs until you approve the file in Yardsort, on
[the card](#the-approval-card) at the top of the workspace. The approval is recorded for that
exact file, by its content hash, so the next run of the same file goes straight ahead; a changed
file waits again, and the card shows what changed. A project can ask to see every run instead:
**Ask every time** in its [settings](#project-settings). `ys` has no way to approve, for the
reason `ys memory` has no way to approve: an agent can run anything `ys` offers.

**What a run does.** It gets a free port of its own, `{{ run.port }}`, and a scratch folder,
`{{ run.dir }}`, removed afterwards. `setup` runs first, then `steps`, stopping at the first
step that fails; `teardown` runs whatever happened. Every `run`, `shell` and `start` process
gets the run's variables in its environment (the table above) and the project's test
environment as `YS_ENV_<NAME>`. A `start` process is kept running until the run ends, when it
is stopped with everything it started; if it exits before that, the run fails there and the
step's note has its exit code and last lines. A `manual` step waits for your **Pass** or
**Fail** in the app. Each step has five minutes unless its `timeout` says, the whole run thirty
unless the file's `timeout` says; a process that writes more than 16 MiB to stdout or stderr is
stopped and fails, a `start` process too, whatever step the run is on by then. Each process's
output goes to files under Yardsort's data directory, `test-runs/<run id>/<step>.stdout` and
`.stderr`, kept for thirty days.

**Cancelling.** `ys test cancel <run>` ends a run that is waiting or queued at once. A running
run is asked to stop: the step it is on is ended, with its process and everything that process
started, or the request or question it was waiting on, the steps not reached are marked
cancelled, the teardown runs, and the run ends as cancelled. If Yardsort stops during a run, the
next start ends the run as _errored_ and stops any server it left behind. A run Yardsort could
not see through for a reason of its own, its database locked for too long for one, ends as
_errored_ with that reason, its servers stopped.

**The verdict.** `ys test wait <run>` blocks until the run ends and prints it the way an agent
can act on it:

```text
$ ys test wait 8c1d
failed  login  (3 of 6 steps passed, 1m 12s)
  sign_in  http  failed after 31s
    status: expected 200, got 500.
    last lines of `server` (setup step):
      [vite] 500 /api/session: TypeError: cannot read 'id' of undefined
```

The first line is the status, the test and how many of the setup and test steps passed. Then
every step that did not pass, with its note: each expectation that failed, in words. Then, for a
run that failed, the last lines of each server the test started. `wait` exits 0 when the run
passed, 1 when it failed and 2 for anything else: cancelled, refused, errored, or still going
when `--timeout` (20 minutes unless said) passed. `ys test runs` lists runs, newest first, and
`--run <id>` shows one step by step; `ys test log <run>` prints a step's output, the first
failed step's unless `--step` says, `--stderr` for the other stream. A run can be named by the
start of its id.

## In the app

### The Tests view

**Tests**, in the sidebar under _Workflows_, opens every project's smoke tests in one place. Its
number is the runs waiting for your approval, across every workspace; it reaches zero when nothing
is waiting. The view is read from the repositories when you open it and on **Refresh**, the way
Tasks reads the forge.

It has two tabs. **Tests** lists every test across every workspace, with the verdict of its last
run — _passed_, _failed_, _never run_, _waiting for approval_ — when it ran, and in which
workspace. **Runs** lists every run, newest first. The header filters narrow both: by project, by
workspace, and — on Tests — the last verdict and whether a test **needs attention** (a browser the
machine lacks, or an environment value the project has not set); on Runs, who asked (you, an
agent, a workflow). A search box narrows by name. The filters are remembered between sessions.
Filtering to a workspace reaches back through that workspace's own history, so an older
workspace's runs stay findable however many newer ones other workspaces have since produced.

A test opens beside the list, to its file (read-only — it is the repository's, so edit it in the
worktree), the problems or missing pieces it has, its run history (that test, in that project,
across its workspaces), and **Run in…** a workspace of its project. A run, from either tab, opens its [run tab](#the-run-tab) in its
workspace. A row with a problem, a missing browser or an unset environment value is flagged where
it sits, so what cannot run yet is plain before you try it.

### Describe it

Rather than write a test by hand, press **Describe it…** in the Tests view header, say what it
should check, and a model writes the file: the agent you already have, in its non-interactive
mode, or your Anthropic key — the same writers that draft a [workflow](workflows.md), chosen in
Settings → Assist. What comes back is editable, with any problems marked, and is nothing until you
pick a workspace and **Save** it into that worktree's `.yardsort/tests`, where it is then an
ordinary uncommitted change in the Changes panel. You can save into any live workspace of any
project, including one that has no smoke tests yet. Saving only ever creates: if the id is
already taken — by this test's own file or another's — the save is refused rather than quietly
replacing it, so an agent's uncommitted edits are never lost; give the new test an id of its own,
or edit the existing file in the worktree. The usual way a test gets written, though, is the
agent in the workspace writing it itself, because it knows the feature — press Describe it when
you want one and no agent is on it.

### The approval card

A run waiting for your approval shows at the top of its workspace, above the terminal tabs, and
the workspace's row in the sidebar carries a count of the runs waiting there. The card is the
whole of what the file does, in words: who asked for the run and with what inputs, then every
step, phase by phase — _run `bun run seed --user demo@example.test`; expect exit 0_,
_GET http://localhost:{{ run.port }}/health; expect status 200_, _ask you: Does the dashboard
chart animate in?_ — with every `{{ variable }}` left as written, since a port is picked when
the run starts and a secret is never printed. A script a step feeds to a program and a request
body are shown whole, so nothing a step carries out is hidden behind a summary. Under the steps,
what it reads from the project's test environment: a step that runs a program is handed all of
it (as `YS_ENV_*`), so when the file has such a step the card names the whole environment, not
only the values the file mentions, and flags any it mentions that are not set. A file that was
approved before and has changed since shows what changed, line by line.

**Run** approves the file for the project and starts the run; its tab opens. **Don't run** ends
the run as _refused_, which `ys test wait` reports to the agent in words; nothing is recorded
against the file, so the same file asked for again waits again. While the card has the keyboard,
`Mod+Enter` runs and `Esc` refuses, as in the composer; it never takes the keyboard from the
terminal behind it, so an `Esc` meant for the agent refuses nothing. Several runs waiting in one
workspace are shown one at a time, oldest first.

### The run tab

A run opens as a tab in its workspace's tab strip, beside the terminals, named after the test
with a dot for how it is going. It opens when you approve a run or start one from the app;
**Run smoke test…** lists the workspace's recent runs to open any of them again. The terminal
behind it keeps running; `Mod+W` closes the tab, and closing it loses nothing, since the tab is a
view of what the run recorded.

Down the left are the steps in their three phases, each with its status and how long it took.
The one in view is the step going, else the first that failed, else the last that ended; click
any to see it. On the right: the step in words, its note — every expectation that failed, in
words, or why it was skipped — the facts it left (`exit`, `status`, `pid`, `waited`), the
response body of an `http` step, and the stdout and stderr of a process, tailed as it writes
while the step runs. A `manual` step shows its question with **Pass**, **Fail** and a note,
which the step keeps. The header has **Cancel** while the run goes — the step it is on is ended,
the teardown runs, and the run ends as cancelled — and, once it has ended, **Run again**, which
opens the Run dialog with the same inputs, and **Send result to agent**.

### Run smoke test…

In a workspace's **⋯** menu. The dialog lists the worktree's tests with their readiness — ready,
the number of problems, or that a `browser` test needs a browser none was found for — asks the
chosen test's inputs, starting from their defaults, and shows its steps as the card does, with
the environment it reads and what changed since it was last approved. Pressing **Run** is the
approval — of the file the dialog showed: the file is approved for the project, so no card
follows, and the next run of the same file goes straight ahead. If an agent has changed the file
since the dialog opened, the run waits on the approval card instead, showing the new version, so
what runs is always a version you have seen. A workspace with no tests is told where they go and
that `ys test guide` tells an agent how to write one.

### Schedule…

Beside **Run…** on a test in the Tests view, and in a workspace's **⋯** menu. It runs the test
at a set time or on a routine. The dialog shows the file's steps as the Run dialog does, and
saving approves that file; a file that changes later waits for approval again. A test with a
`manual` step cannot be scheduled. See [Schedules](schedules.md#smoke-tests-on-a-schedule).

### Send result to agent

On an ended run's tab. It types what `ys test wait` would have printed — the verdict, the step
that failed and why, the last lines of any server the test started — into the workspace's
agent, with a line first saying what it is, the way a workflow's `send_to_session` does: at
once if the agent is quiet, otherwise as soon as it is, and never while someone else holds the
terminal. With one agent running in the workspace it goes there; with several, a menu asks
which. A run started by an agent with `ys test run` needs none of this: the agent reads the same
words from `ys test wait`.

### Project settings

**Project settings…** in a project's menu, or **Project settings** above a workspace's terminal
tabs, has a _Smoke tests_ part:

- **The test environment** — names and values a test reads as `{{ env.NAME }}` and every
  `run`, `shell` and `start` process gets as `YS_ENV_<NAME>`. Kept in Yardsort's own database,
  never in the repository; the values are hidden until **Show values**. A test that uses a name
  not set here is refused by `ys test run` and the Run dialog, and the card says which names a
  waiting run would find empty.
- **Ask every time** — show every run's steps and ask, even for a file approved before. Off by
  default: a file you have approved runs straight away until it changes.
- **Keep runs for** — how many days ended runs are kept, with their output files, thirty unless
  changed. Runs waiting, queued or going are never removed.
- **Browser** — the browser a `browser` step drives. Left empty, Yardsort finds one: Chrome,
  then Chromium, then Edge, on your `PATH` and in the usual install folders. Set the full path to
  a Chromium-family browser to use that one, and only that one — a path that is not there is
  _needs a browser_, never a quiet fall back to a different browser. Nothing is installed and no
  Node is needed; a `browser` test whose browser is not found is refused, saying whether it was
  the path you named or no browser at all.

## Agents

The usual way a test gets written is by the agent in the workspace, because it knows the feature.
Ask it to write a smoke test for what it built: `ys test guide` is the reference it reads,
`ys test validate` is how it checks the result, and `ys test run` then `ys test wait` is how it
runs the test and reads the verdict, once you have approved the file. Every example in the guide is itself checked
against the parser when Yardsort is built, so what an agent is told and what the validator
accepts cannot drift.

An agent is told about this the way it learns about `ys memory` and `ys task`: in its first
message, when the project has a `.yardsort/tests` folder or a test environment set. It gets a
short paragraph pointing it at `ys test list`, `ys test run` and `ys test guide`, so you need not
explain the commands each time.

A [workflow](workflows.md) can run a test too, with its `run_test` step, which queues a run and
waits for its verdict — passing or failing with it, so a later step can act on either. The
built-in **Smoke test, then tell the agent** is exactly that followed by a message to the
workspace's agent with the result, whether it passed or failed — the shape of "when the agent is
done, smoke test it and tell it what broke."
