# Smoke tests

A smoke test is a short, structured list of steps that checks a piece of software from the
outside: start the server, wait for it, hit an endpoint, open a page, look for a word. It is the
ten-minute check you do before saying "done", written down so an agent can write it and Yardsort
can run it. A test is a YAML file in your repository, `.yardsort/tests/<id>.yaml`, so it belongs
to the code it checks: an agent writes one for the feature it built, in the worktree it is
standing in, and the test travels with the branch.

**This release reads, checks and runs the files.** `ys test validate` names every mistake at its
line and column, `ys test list` shows a workspace's tests, `ys test guide` tells an agent how to
write one, and `ys test run` queues a run that Yardsort carries out, step by step, with the
verdict from `ys test wait`. A file from a repository never runs unseen: a run of a new or
changed file waits until you approve it in Yardsort. The card that shows you the file, the tab
that shows the run as it goes, and the Tests view are the next step; until they arrive a run
started from `ys` waits, and `browser` steps are not run. See [Running a test](#running-a-test).

## Where tests live

Every `.yaml` or `.yml` file directly in a worktree's `.yardsort/tests` folder is a test. Each
workspace has its own copy, since each worktree is its own checkout: a test an agent wrote on a
feature branch exists in that workspace until the branch is merged. `ys test list` reads the
folder of the workspace it is run in, or of `--workspace <name>`.

A file that does not check out is still listed, under the id it says or its file name, with its
problems. Two files with one `id`: the first by file name is used, and the second is listed with
the clash.

A test says what to run on your machine, and it comes from a repository. That is why nothing
here runs unseen: when running arrives, Yardsort shows a new or changed file's steps and asks
you first, once per version of the file.

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
text. Every action waits for its selector to exist and be visible before acting, up to the step's
`timeout`, so there is no `sleep` and none is needed.

| Action           | Value                                           |
| ---------------- | ----------------------------------------------- |
| `goto`           | An address                                      |
| `click`          | A selector                                      |
| `fill`           | `{ selector, text }`; an empty `text` clears it |
| `press`          | A key, to the focused element: `Enter`, `Tab`   |
| `select`         | `{ selector, value }`                           |
| `wait_for`       | A selector, or `{ text: … }`                    |
| `expect_text`    | Text that must be on the page                   |
| `expect_visible` | A selector                                      |
| `expect_hidden`  | A selector                                      |
| `expect_url`     | A pattern over the address, like `**/dashboard` |
| `expect_title`   | The page title                                  |
| `screenshot`     | A name for the file                             |

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
anything is queued, and refuses a test with a `browser` step, which this release cannot run.
It prints the run's id and returns at once.

**Approval.** A test says what to run on your machine, and it comes from a repository. So a run
of a file you have not seen — new, or changed in any way since you last approved it — is queued
as _awaiting approval_, and nothing runs until you approve the file in Yardsort. The approval is
recorded for that exact file, by its content hash, so the next run of the same file goes straight
ahead; a changed file waits again. `ys` has no way to approve, for the reason `ys memory` has no
way to approve: an agent can run anything `ys` offers. The card in the app that shows the file
and approves it is the next step; until it arrives a run started from `ys` waits.

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

## Agents

The usual way a test gets written is by the agent in the workspace, because it knows the feature.
Ask it to write a smoke test for what it built: `ys test guide` is the reference it reads,
`ys test validate` is how it checks the result, and `ys test run` then `ys test wait` is how it
runs the test and reads the verdict, once you have approved the file. Every example in the guide is itself checked
against the parser when Yardsort is built, so what an agent is told and what the validator
accepts cannot drift.
