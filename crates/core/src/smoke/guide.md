# Writing a smoke test for Yardsort

A smoke test is a YAML file in the repository, `.yardsort/tests/<id>.yaml`, that checks a piece
of software from the outside: start it, wait for it, call it, open a page, look for a word.
Yardsort runs it step by step and shows every step, its output and why it failed. Write one for
the feature you built, beside the project's others, and check it with `ys test validate <file>`
before you mention it. Ask before running it: the user approves each new or changed file in
Yardsort, and nothing runs until they do.

Three rules, then the format:

- **Steps are typed and few.** `run` a program, a `shell` line, `start` a server for the length
  of the test, `wait_for` it, an `http` request, a `browser` step, or a `manual` step a person
  does. Nothing else.
- **Use `{{ run.port }}`, never a fixed port.** Several workspaces may run the same test at
  once. Start the server on the run's port and point every later step at it.
- **No secrets in the file.** Passwords and machine-specific values come from the project's
  test environment as `{{ env.NAME }}`; the user sets them in Yardsort.

## A whole file

```yaml
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
    program: npm
    args: [run, dev, --, --port, "{{ run.port }}"]
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
    program: npm
    args: [run, seed, --, --user, "{{ inputs.user }}"]
    expect:
      stdout_contains: seeded

  - id: sign_in
    action: browser
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
  - id: reset
    action: run
    program: npm
    args: [run, db:reset]
```

`setup` runs first; a failure there fails the run before any step. `steps` run in order and stop
at the first failure. `teardown` always runs, however the run ended. A `start` process is
stopped at the end whatever happens. Steps in all three lists share one set of ids.

## The file

| Key           | Rule                                                                                                                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `id`          | Lowercase letters, digits and `-`. Name the file after it.                                                                         |
| `name`        | Required.                                                                                                                          |
| `description` | Optional.                                                                                                                          |
| `version`     | `1`.                                                                                                                               |
| `inputs`      | Optional. What the person is asked before a run: `id`, `kind` (`text` or `choice` with `options`), `label`, `required`, `default`. |
| `timeout`     | Optional, for the whole run. `30m` unless said. Written as a number and `s`, `m` or `h`.                                           |
| `setup`       | Optional list of steps.                                                                                                            |
| `steps`       | At least one.                                                                                                                      |
| `teardown`    | Optional list of steps.                                                                                                            |

Keys a test does not have are mistakes, not ignored. Every step has `id` and `action`, then
the action's keys. Each step may have `timeout` (`5m` unless said), except `start` and `manual`.

## Steps

| Action     | Keys                                                                                                                     |
| ---------- | ------------------------------------------------------------------------------------------------------------------------ |
| `run`      | `program`, `args` (a list, each argument its own item), `cwd`, `env`, `stdin`, `expect`                                  |
| `shell`    | `line`, `cwd`, `env`, `expect`. The login shell on Unix, PowerShell on Windows.                                          |
| `start`    | `program`, `args`, `cwd`, `env`. Kept running until the end of the test.                                                 |
| `wait_for` | One of `port`, `url` (answers 2xx), `file`, or `program` with `args` (exits 0); `interval` (`1s`).                       |
| `http`     | One of `get`, `post`, `put`, `delete`, `patch` with the address; `headers`, `body` or `json`, `expect`.                  |
| `browser`  | `do` (a list of actions, below), `screenshots` (`on_failure`, `every_step`, `never`), `viewport` (`1280x800`), `headed`. |
| `manual`   | `prompt`: the question the person answers with Pass or Fail.                                                             |

`cwd` is relative to the workspace and inside it: no `..`, no absolute path, no variables. `env`
is a mapping of names to values; values may use variables.

A `shell` line cannot hold `{{ … }}`: a value pasted into a shell line could become a command.
The run's variables are in the environment instead — `$YS_RUN_PORT`, `$YS_RUN_DIR`,
`$YS_WORKSPACE_PATH`, `$YS_INPUT_<ID>`, `$YS_ENV_<NAME>` — and every `run` and `start` process
gets them too. Prefer `run` where there is no pipe; it works the same on every platform.

```yaml
id: api-smoke
name: The API answers and the CLI agrees
version: 1
steps:
  - id: health
    action: shell
    line: curl -s "http://localhost:$YS_RUN_PORT/health" | jq -e '.status == "ok"'
  - id: version
    action: run
    program: node
    args: [--version]
    expect:
      stdout_matches: '^v\d+\.'
```

### Expectations

For `run` and `shell`, under `expect`: `exit` (a number, or `nonzero`; 0 unless said),
`stdout_contains`, `stdout_matches` (a regular expression), `stderr_empty` (`true`),
`stdout_json` (`{ path: status, equals: ok }`). For `http`: `status` (a number, or a class like
`2xx`; 2xx unless said), `json` (a mapping whose every key must appear, with its value, in the
response), `body_contains`, `body_matches`, `header` (`{ name, contains }`). Every expectation
given is checked, and the step's note lists each one that failed.

### Browser actions

Each item in `do` is one key and its value. Selectors are CSS, or `text=…` for an element by its
text. Every action waits for its selector to exist and be visible before acting, up to the step's
`timeout`, so there is no `sleep` and none is needed.

| Action           | Value                                           |
| ---------------- | ----------------------------------------------- |
| `goto`           | An address                                      |
| `click`          | A selector                                      |
| `fill`           | `{ selector, text }`                            |
| `press`          | A key, to the focused element: `Enter`, `Tab`   |
| `select`         | `{ selector, value }`                           |
| `wait_for`       | A selector, or `{ text: … }`                    |
| `expect_text`    | Text that must be on the page                   |
| `expect_visible` | A selector                                      |
| `expect_hidden`  | A selector                                      |
| `expect_url`     | A pattern over the address, like `**/dashboard` |
| `expect_title`   | The page title                                  |
| `screenshot`     | A name for the file                             |

The browser is a Chromium found on the machine, headless unless `headed: true`. A failing action
takes a screenshot; `screenshots: every_step` takes one after each action.

```yaml
id: home-page
name: The home page greets
version: 1
setup:
  - id: server
    action: start
    program: npm
    args: [run, dev, --, --port, "{{ run.port }}"]
  - id: ready
    action: wait_for
    port: "{{ run.port }}"
    timeout: 60s
steps:
  - id: greet
    action: browser
    viewport: 1280x800
    do:
      - goto: "http://localhost:{{ run.port }}/"
      - wait_for: { text: Welcome }
      - expect_title: Home
      - screenshot: home
```

## Variables

`{{ name }}` with a dotted path and nothing else: no filters, no expressions. `\{{` writes a
literal `{{`.

| Variable                                               | What it is                                                                                                                                                                                                                                   |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `project.name`, `project.root`                         | The project and the folder of its own checkout                                                                                                                                                                                               |
| `workspace.name`, `workspace.branch`, `workspace.path` | The workspace the run is about                                                                                                                                                                                                               |
| `run.id`, `run.port`, `run.dir`                        | This run: its id, one free port, and a scratch folder removed afterwards                                                                                                                                                                     |
| `inputs.<id>`                                          | What was given for that input                                                                                                                                                                                                                |
| `env.<NAME>`                                           | The project's test environment, set by the user in Yardsort                                                                                                                                                                                  |
| `steps.<id>.<field>`                                   | What an earlier step left: `exit`, `stdout`, `stderr` from `run` and `shell`; `pid`, `stdout`, `stderr` from `start`; `waited` from `wait_for`; `status`, `body`, `headers` from `http`; `url`, `title` from `browser`; `note` from `manual` |

A step can use only the results of steps earlier in the file.

## Checking it

```sh
ys test validate .yardsort/tests/login.yaml   # each problem as file:line:column: message
ys test example                               # a complete file to start from
ys test list                                  # the tests in this workspace, and whether each is ready
ys test show login                            # one file as Yardsort reads it
```

`validate` needs no Yardsort profile and exits 1 when anything is wrong. Fix every line it names
before telling the user the test is ready.
