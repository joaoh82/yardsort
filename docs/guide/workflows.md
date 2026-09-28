# Workflows

A workflow is a named, reusable piece of agent work that runs in steps. For example: have a second
agent review this workspace's pull request, then tell me and the agent that wrote it. You write
one as a short YAML file. Yardsort checks every name and every `{{ variable }}` in it before it
runs anything, and names the line and column of each mistake.

This version reads and checks workflow files: `ys workflow list`, `show` and `validate`. Running
them, from the app and from `ys`, and the **Workflows** section of the sidebar are being built;
the [roadmap](../design/05-roadmap.md) has the plan. Until then, what this page says a step
_does_ is what it is for. The checks are all real now.

## Where workflows come from

- **Built in.** Yardsort ships with one: **Request code review** (`code-review`). It is for
  having a second agent, one you pick, review the workspace's pull request in the same worktree
  and post its review on GitHub with `gh`. When the review is on the pull request, you get a
  notification, and the agent that wrote the pull request is told once it is not busy. `ys workflow show code-review`
  prints the whole file.
- **Yours.** Every `.yaml` or `.yml` file in the `workflows` folder of Yardsort's data directory.
  [Where Yardsort keeps things](troubleshooting.md#where-yardsort-keeps-things) says where that
  is on your system, and `ys workflow list` prints it.

A file with a built-in's `id` is used instead of that built-in. That is how you change one: copy
it with `ys workflow show code-review > <folder>/code-review.yaml`, then edit the copy. Delete
the file and the built-in comes back. If your copy has a mistake, it is listed with its problems
and does not run. The built-in does not quietly run in its place.

If two of your files have the same `id`, the first by file name is used. The other is listed with
a problem saying which file it clashes with.

Workflows only come from your own folder, never from a project's repository. A workflow decides
what is sent to your agents. A repository you cloned should not get to decide that.

## An example

```yaml
id: fix-ci
name: Fix the failing checks
description: An agent reads the failed checks on this workspace's pull request and fixes them.
version: 1

trigger:
  kind: manual

inputs:
  - id: fixer
    kind: harness
    label: Who fixes it
    required: true

steps:
  - id: fix
    action: start_session
    harness: "{{ inputs.fixer }}"
    prompt: |
      The checks on pull request #{{ pr.number }} ({{ pr.url }}) are failing. Read them with
      `gh pr checks {{ pr.number }}`, fix the cause on `{{ workspace.branch }}`, and push.

      {{ memory }}

  - id: done
    action: wait_session
    needs: [fix]
    session: "{{ steps.fix.session }}"
    timeout: 1h

  - id: tell_me
    action: notify
    needs: [done]
    title: "{{ workspace.name }}: the fix is pushed"
```

Steps run as soon as every step in their `needs` has finished, so steps can run side by side.
`tell_me` waits for `done`, and `done` waits for `fix`. Two steps that both need `done` would run
together.

## The file

| Key           | What it is                                                                                                               |
| ------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `id`          | Lowercase letters, digits and `-`, like `fix-ci`. Name the file after it.                                                |
| `name`        | What the workflow is called in lists.                                                                                    |
| `description` | Optional. One or two sentences.                                                                                          |
| `version`     | `1`. A file with a later version is listed but not run, and says to update Yardsort.                                     |
| `trigger`     | `kind: manual`: you start it. Optional `context: workspace`, the default and the only one: a run is about one workspace. |
| `inputs`      | Optional. What you are asked before a run starts.                                                                        |
| `steps`       | At least one.                                                                                                            |

Keys a workflow does not have are mistakes, not ignored. A misspelt `promt:` is reported, not
silently skipped.

### Inputs

| Key        | What it is                                                                                                |
| ---------- | --------------------------------------------------------------------------------------------------------- |
| `id`       | Starts with a lowercase letter; then lowercase letters, digits, `_` and `-`. Used as `{{ inputs.<id> }}`. |
| `kind`     | `harness` (one of your agents), `text`, or `choice`.                                                      |
| `label`    | Optional. The question. The `id` when left out.                                                           |
| `required` | Optional. `false` when left out. An optional input left empty is empty text wherever it is used.          |
| `default`  | Optional. For a `choice`, one of its options.                                                             |
| `options`  | For a `choice` only: the answers to pick from.                                                            |

### Steps

Every step has an `id`, named the same way as an input, and an `action`. `needs` is optional: the
steps that must finish first. A step cannot need itself. Steps that wait for each other in a
circle are reported, because none of them could ever start.

| Action             | Keys                                                             | What it does                                                                                                                                                                                            |
| ------------------ | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `start_session`    | `harness` (required), `prompt`, `model`, `effort`, `skip_memory` | Starts an agent in the workspace, like the composer does, with `prompt` as its first message. `harness` is an agent's id, like `claude`, or exactly `{{ inputs.<id> }}` for an input of kind `harness`. |
| `wait_session`     | `session` (required), `until`, `timeout`                         | Waits for a session. `until: settled`, the default, means it has gone quiet or finished its turn. `until: exited` means the program has ended.                                                          |
| `send_to_session`  | `session` (required), `prompt` (required), `timeout`             | Types `prompt` into a running session. It waits until that session is quiet: a busy agent is never interrupted. If the session is gone, the step is skipped.                                            |
| `wait_pr_activity` | `kind`, `timeout`                                                | Waits for a new review or comment on the workspace's pull request. `kind` is `review`, `comment` or `any`, the default.                                                                                 |
| `notify`           | `title` (required), `body`                                       | Tells you: a notification when Yardsort is in the background, a message in the window otherwise.                                                                                                        |

A key that belongs to another action is reported as misplaced. `title` on a `start_session` step
is an example.

**`session`** is `origin`, meaning the agent already working in the workspace, or
`"{{ steps.<id>.session }}"` for a session an earlier `start_session` step started. That step
must be among this step's `needs`, directly or through another step.

**`timeout`** is a whole number and a unit: `30s`, `45m`, `2h`. At most a week. A step that runs
out of time fails.

### Variables

Any text in a step can use `{{ name }}`. Only these names exist. A name is a dotted path and
nothing else: no filters, no expressions. Anything else is reported where it is written.

| Variable                                                                                          | What it is                                                                                                                                                                                        |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `project.name`, `project.root`                                                                    | The project, and the folder of its own checkout.                                                                                                                                                  |
| `workspace.name`, `workspace.branch`, `workspace.base_branch`, `workspace.path`, `workspace.task` | The workspace the run is about. `task` is the first message it was started with.                                                                                                                  |
| `pr.number`, `pr.url`, `pr.title`                                                                 | The open pull request for the workspace's branch.                                                                                                                                                 |
| `inputs.<id>`                                                                                     | What was given for that input.                                                                                                                                                                    |
| `steps.<id>.<field>`                                                                              | What an earlier step left: `session` and `run` from `start_session`, `outcome` from `wait_session`, `count` and `latest_url` from `wait_pr_activity`. The step must be among this step's `needs`. |
| `memory`                                                                                          | The project's [memory](memory.md), as an agent is given it, when the project shares it. Empty otherwise.                                                                                          |
| `handoff`                                                                                         | The workspace's [handoff](terminals-and-sessions.md#handing-work-to-another-agent) packet: what happened there so far.                                                                            |

To write `{{` literally, for example in a prompt about a template language, write `\{{`.

## Checking a file

`ys workflow validate <file>` checks a file without needing Yardsort to have run on this machine.
It prints each problem as `file:line:column: message` and exits 1 if there are any:

```text
$ ys workflow validate fix-ci.yaml
fix-ci.yaml:20:35: `{{ pr.nubmer }}`: `pr` has no `nubmer`. It has `number`, `url`, `title`.
fix-ci.yaml:27:13: There is no step `fx`. Did you mean `fix`?
fix-ci.yaml:28:15: `{{ steps.fix.session }}` is used before step `fix` is sure to have run. Add `fix` to this step's `needs`.
```

That is the example above with `pr.number` misspelt and `needs: [fix]` written `[fx]`.

`-` reads the file from standard input. `--json` prints `{ valid, id, problems }`.

`ys workflow list` shows every workflow, whether it is ready to run, and where it comes from.
`ys workflow show <id>` prints the file that would be used, with any problems after it. See
[the `ys` command line](cli.md#ys-workflow).
