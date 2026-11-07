# Triggers

A trigger starts a [workflow](workflows.md) or a [smoke test](smoke-tests.md) when something
happens in a workspace, instead of when you press **Run** or a [schedule](schedules.md)'s time
comes: an agent says its work is done, an agent finishes a turn or exits, a smoke test or a
workflow run ends. What it starts is an ordinary run, in that workspace, with the inputs you gave
the trigger.

For example: when the agent in `fix-login` says it is done, run a smoke test and tell it how it
went. Or: when that agent's program exits with an error, tell me.

**This release has triggers from the command line.** `ys trigger` adds, lists and changes them,
and Yardsort listens and starts their runs while it is open. The runs show in the workflow's runs
and in **Tests** like any other. The window does not list triggers yet, and the triggers for
pull requests — one opened, a review posted, the checks failing — are still to come. See
[What it does not do yet](#what-it-does-not-do-yet).

Three things to know before you make one:

- **Triggers fire while Yardsort is open.** What happened while it was closed — an agent that ran
  `ys workspace done`, a run that ended, a turn its agent reported — is found when it next opens,
  and acted on once.
- **Only what happens after you make it counts.** A trigger never fires for something that
  happened before it was saved, or while it was switched off.
- **A trigger approves nothing.** A smoke test file nobody has approved waits for you, as it does
  when an agent runs it. See [Smoke tests and approval](#smoke-tests-and-approval).

## Making a trigger

```sh
ys trigger add --on workspace.done --workflow smoke-then-tell --input suite=login --workspace fix-login
ys trigger add --on agent.exited --failed-exit --workflow tell-me --workspace fix-login
ys trigger add --on test.finished --of-test login --verdict failed --workflow fix-it
```

`--on` says what to listen for; give it more than once for more than one event. `--workflow` or
`--test` says what to run, and `--input id=value` gives each of its inputs. `--workspace` takes a
workspace's name or id, and can be left out inside a workspace's folder or in an agent's terminal
there. Everything a run would be refused for is checked before anything is written: the workspace
is there and not archived, the workflow or the test is there and ready, its inputs fit, and the
filters are ones the events have.

| Option        | What it is                                                                                                     |
| ------------- | -------------------------------------------------------------------------------------------------------------- |
| `--on`        | An event, below. More than once for more than one                                                              |
| `--workflow`  | Run this workflow, or                                                                                          |
| `--test`      | run this smoke test, from the workspace's `.yardsort/tests`                                                    |
| `--workspace` | Where. Default: the workspace this is run in                                                                   |
| `--session`   | Only this agent session's events, by its id or the start of it. See [One agent's trigger](#one-agents-trigger) |
| `--input`     | `id=value`, for each of the target's inputs                                                                    |
| `--name`      | What lists call it. Default: the workflow's or the test's own name                                             |
| `--quiet`     | Wait this long after the last event before starting the run: `30s`, `2m`. Default: none                        |
| `--cap`       | Runs it may start for one agent session, or for the workspace, before it stops. Default: 3                     |
| `--notify`    | `failure` (the default), `always` or `never`: when to be told how a run ended                                  |

## What it listens for

| Event               | Happens when                                                                    | Filters                                                 |
| ------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------- |
| `workspace.done`    | An agent ran [`ys workspace done`](#the-done-signal)                            |                                                         |
| `agent.settled`     | An agent finished a turn: it went quiet after real work, or its harness said so | `--new-commits`, `--clean`                              |
| `agent.exited`      | An agent's program ended                                                        | `--failed-exit`                                         |
| `test.finished`     | A smoke test run in the workspace reached a verdict, or could not               | `--of-test <id>`, `--verdict passed` or `failed`        |
| `workflow.finished` | A workflow run in the workspace ended                                           | `--of-workflow <id>`, `--outcome succeeded` or `failed` |

- **An agent's events** are about the agents you started in the workspace. A shell is not an
  agent, and neither is an agent a workflow run started: the reviewer of a code review finishing
  its turn sets nothing off. An agent's events and the workspace's cannot share one trigger,
  since each counts its runs for something different; make two.
- **`agent.settled` fires at every turn**, including a turn where the agent stops to ask you
  something. That is what "tell me each time it stops" wants, and not what "it is finished"
  means. `--new-commits` waits for the branch to have commits it did not have the last time this
  trigger started a run for that agent, or, the first time, commits its base branch does not
  have. `--clean` waits for nothing to be uncommitted. Both are looked at when the run would
  start, after the quiet time. For "the work is done", `workspace.done` is the surer one.
- **`--failed-exit`** is a code that is not 0, a program that was stopped, or one that ended
  without Yardsort hearing how.
- **`--verdict failed`** includes a run that could not finish; **`--outcome failed`** includes a
  cancelled run.

## The done signal

```sh
ys workspace done
ys workspace done --note "Login validates on blur; tests added."
```

`ys workspace done`, run in an agent's terminal, says the work in that workspace is committed and
ready to be looked at. A trigger on `workspace.done` starts what it starts. The note, if there is
one, is there for the workflow as `{{ event.note }}`, and the commit the worktree was on as
`{{ event.head }}`. Outside an agent's terminal, run it in the workspace's folder or give
`--workspace`. It prints whether a trigger there listens for it.

**An agent is told about it** in its first message — a short section after the project's memory
and the smoke tests — when a trigger in its workspace listens for `workspace.done` at the moment
it starts. It is asked to run it once its work is committed, and not to pause or to ask a
question. An agent that was already running when you added the trigger was not told; `ys trigger
add` says so. Agents started from then on are.

Yardsort reads nothing an agent prints to know it is done: the agent runs a command, as it runs
`git` or `gh`.

## When a run starts

**A burst is one run.** With `--quiet 2m`, the trigger waits until nothing new has happened for two
minutes, then starts one run for everything that came — but never later than ten minutes after
the first. For `agent.settled`, the agent going back to work before the quiet time is up means
that turn did not end, and nothing starts. Without `--quiet`, a run starts within a second.

**A run in the way makes it wait.** Only one run of a workflow, or of a test, goes at a time in a
workspace, whoever started it. When the trigger's run would be the second, the trigger waits,
and starts it when the first ends, with everything that came meanwhile. A schedule skips such a
time, because another time will come; something that happened will not happen again.

**It stops before it loops.** Three limits, each told to you once:

- **The cap.** At most `--cap` runs — three by default — for one agent session, for an agent's
  events, or for the workspace, for the rest. The next fire is **capped**, you are told, and the
  trigger starts nothing more for it until you run `ys trigger reset`.
- **The rate.** At most ten runs an hour for one trigger, whatever the cap.
- **Chains.** A run a trigger started can end in an event another trigger listens for. A run a
  person or a schedule started is depth 0; a run started by the ending of a run of depth _n_ is
  _n_ + 1. A fire past depth 3 is refused.

### One agent's trigger

`--session <id>` makes a trigger that belongs to one agent session: it hears that agent's own
turns and exits, and its runs' `session: origin` is that agent, whichever agent in the workspace is
newest. A fire while that agent's program is not running is **skipped**: there is no one to tell.
The trigger goes when the session's record does. `ys session list` shows the ids.

### Smoke tests and approval

A smoke test file comes from a repository and says what to run on your computer, so it never runs
until you have seen it. A trigger keeps to that: a run of a new or changed file waits on the card
at the top of its workspace, the trigger reads _waiting for approval_, and you are told once
(see [Being told](#being-told)).
Approve it and that run goes. Meanwhile the trigger's next events wait behind it.

Nothing else a person does is waited for:

- **A test with a `manual` step cannot be triggered.** Nobody is there to press **Pass**. This is
  checked when the trigger is saved and again each time it fires, for a test it runs itself and
  for one a workflow's `run_test` step names.
- **A workflow's `run_test` steps each need a `timeout`.** A test named by an input is not known
  until the run, and an approval may not come; the timeout ends the wait.

## What the workflow is told

A workflow a trigger starts can read what set it off as `{{ event.* }}`:

| Variable        | What it is                                                                    |
| --------------- | ----------------------------------------------------------------------------- |
| `event.kind`    | The event: `workspace.done`, `agent.settled`, …                               |
| `event.count`   | How many events the run is for, when a burst was gathered into one            |
| `event.note`    | `workspace.done`: what the agent passed to `--note`                           |
| `event.head`    | `workspace.done`, `agent.settled`: the commit the worktree was on             |
| `event.summary` | `agent.exited`, `test.finished`, `workflow.finished`: how it ended, in a line |
| `event.verdict` | `test.finished`: `passed`, `failed`, `errored`, …                             |

A trigger whose workflow uses a variable one of its events does not leave is refused when it is
saved, naming it. Run by hand, every `event.*` is empty, and **Run…** and `ys workflow run` say
so.

## Being told

A triggered run has nobody watching it, so the trigger can tell you how it ended, with a desktop
notification: **when a run fails** (the default), **how every run ends**, or **never**. Unless it
is set to never, a run waiting for your approval is told once, and so is a trigger that is capped
or could not start its run, once while it lasts.

## Its history

`ys trigger fires <trigger>` lists every fire, newest first:

| Outcome                            | Meaning                                                                     |
| ---------------------------------- | --------------------------------------------------------------------------- |
| running, succeeded, passed, failed | A run was started; this is how it stands, or how it ended                   |
| waiting for approval               | Its run waits for you to approve the test file                              |
| waiting                            | A run of the same thing is going there; this one starts when it ends        |
| skipped                            | Its agent session was not running, so there was no one to tell              |
| capped                             | It had started as many runs as `--cap` allows. Later events go on this line |
| refused                            | A run could not be started, with the reason                                 |

## From the command line

```sh
ys trigger list                 # every trigger: what it listens for, what it runs, where, how it stands
ys trigger show Tell            # its events, filters, inputs, limits and last fire
ys trigger pause Tell           # off: keeps its history, fires nothing
ys trigger resume Tell          # on again; only what happens from now counts
ys trigger reset Tell           # lift its cap
ys trigger fires Tell           # its history
ys trigger remove Tell          # the trigger and its history; the runs it started stay
```

A trigger is named by its name, its id, or the start of its id. `--json` works as everywhere.
See [`ys trigger`](cli.md#ys-trigger).

## What it does not do yet

- **Pull requests.** A pull request opened, a review or a comment posted, the checks failing.
  These, and the built-in workflows that answer them, are the next part.
- **The window.** Triggers are listed, made and changed with `ys` for now; the composer's
  ready-made triggers and a view of them come later.
- **Run while Yardsort is closed.** What happened is found when it next opens.
