# 37 — The Floor Manager

**Status:** first slice built (M30). Permission is a setting `ys` enforces, settled
2026-11-10 ([open question 53](06-open-questions.md#floor-manager)). _Floor Manager_ is a working
name; _Floor Planner_ was the other one offered. The owner called the third level "god mode";
the setting is named Trusted.

The idea, as captured on 2026-11-07: an always-available coordinator inside the client, with a
persistent button in the lower-right corner. It sees every project, with its resources,
configuration, workflows, tasks, sessions and code reviews. It answers questions across them,
takes a task conversationally, suggests a workflow and starts the work. It finds the pull
requests that need a review, and on request starts sessions to review them. It is meant to
orchestrate work, not only to answer.

## What it is

**An agent in a terminal, not a chat UI.** [Open question 4](06-open-questions.md) settled that
the centre stays a real PTY running a real CLI, and that Yardsort does not build its own chat.
The Floor Manager follows that rule. It is whichever harness the user picks, started like any
other, with no output parsed. What makes it a coordinator is two things:

1. **Where it runs and what it is told.** It runs in `<data dir>/coordinator`, which belongs to no
   project. That folder holds an `AGENTS.md`, assembled for the active permission from the pieces
   in `crates/core/src/coordinator/`, and a `CLAUDE.md` that imports it. Both are rewritten at
   every start and again when the setting is saved, so they describe the level that is set. A
   running agent has already read the old file; it reads the new one the next time it starts.
   The gate does not wait for that.
2. **What it can reach.** It reaches `ys`. The launch puts the folder of the app's own `ys` first
   on `PATH`, sets `YARDSORT_DATA_DIR` to the profile, sets `YARDSORT_CONFIG_DIR` to the
   directory that holds `settings.toml` (on Linux that is not the data directory), and sets
   `YARDSORT_COORDINATOR`. The
   variable's value is ignored. While it is set, `ys` reads the permission from settings on
   every command and enforces it. `ys` already covered projects, workspaces, sessions, logs,
   activity, tasks, workflows, tests, schedules and triggers. What it lacked was pull requests,
   so this slice adds `ys pr` (below).

It has a `coordinator` label on its PTY session and no `workspace` label. The tab strip, the
session records, the activity timeline and every per-workspace list therefore leave it out,
without each of them having to know it exists. It is still an agent (it has the `harness`
label), so the quit dialog counts it, and names it _Floor Manager_. Only one exists at a time:
`Launcher::coordinator` hands back the running one rather than starting a second, and forgets an
ended one before starting anew.

## The window

A **Floor Manager** button sits at the right end of the status bar. A floating button over the
content was considered, as the assistants in hosted dashboards do it. It was turned down because
the lower-right corner of the content is the Changes panel's commit box. The status bar is the
lower-right corner of the window, and it covers nothing. **Mod+J** toggles the panel, and so
does the command palette.

The panel is non-modal, an `aside` rather than a `dialog`, so the app's shortcuts and the rest of
the window keep working beside it. **×** hides it and leaves the agent running. **Stop** asks
before ending it, as the rule against destroying work silently asks of anything that ends work
in progress. An ended Floor Manager keeps its last screen, with **Start again**.

## Permission

The level is `[floor_manager] level` in `settings.toml`: `read-only`, `ask` (the default, and
omitted from the file) or `trusted`. Settings → General saves it on its own, the moment it
changes. `ys` reads the file on every command, so the next one follows it. The instructions are
rewritten then too, when the coordinator folder already exists.

Classification lives in the `ys` binary and is exhaustive over its subcommands, so a new one
does not compile until it is named as a read, a start or write, or something never allowed:

- **Read.** Allowed at every level. Listing, showing, logs, handoff, validate, and the forms of
  `ys task close` and `ys task reopen` that only say what they would do.
- **Act.** Read only refuses. Ask first confirms. Trusted allows. A new workspace, a checkout,
  a workflow, a test, a schedule, a trigger, a task comment, and the rest of the starts and
  writes.
- **Deny.** Refused at every level, with no confirmation: `ys workspace delete` with or without
  `--force`, `ys task close --yes`, `ys attach`.

Ask first does not trust the agent to ask, and it does not read stdin: the agent would answer
itself. `ys` checks that the desktop app holds the profile lock. If it does not, the command is
refused at once. If it does, `ys` writes `<data dir>/coordinator/confirm/<uuid>.json` and polls
for `<uuid>.answer` (`yes` or `no`) until the app writes it, the lock drops, or ten minutes
pass. `ys` touches the request while it waits. One that has not been touched for a few seconds
is hidden, and comes back when it is touched again. It is removed only once it is older than the
ten minute wait, and so is an answer file left behind with no request, so a command that was
killed does not stay in the dialog for the whole wait. The window polls the same folder and
shows **Allow** / **Don't allow**. Answering writes the file. It does not run the command: the
process that is waiting is the one that parsed it. If nothing is waiting, the window says so
instead of recording an answer nothing will run.
The id has to be a uuid whose stem matches the request, so an answer cannot name a path outside
that folder.

This is not a security boundary against a Floor Manager that wants to get around it. It runs as
the user. It can write the answer file, unset `YARDSORT_COORDINATOR`, edit `settings.toml` so
the level is `trusted`, point `ys --data-dir` at another profile, or run `git`, `gh` or an
editor. The instructions tell it not to. The guide says the same to the user. There is no `ys`
command for archiving a workspace, deleting a branch, merging, closing or approving a pull
request, or editing project files, so those stay instructions: the gate cannot see them. The
harness's own permission file would be a second line. It is not this one, because it has to
work for every harness, and a user's own Claude Code settings have already been seen to run
commands without asking.

The variable is a marker of which process is the Floor Manager, not a secret. The user's own
`ys` does not have it, and is not gated, including after the setting is changed.

## `ys pr`

- **`ys pr list`** asks the same GraphQL question per repository as the Pull requests view
  (`Gh::open_pull_requests`, up to 200 open). It applies the view's review rules: the forge's
  decision, then the latest reviews, and _me_ meaning whoever `gh` is logged in as. It adds
  `--attention`, whose reasons are `checksFailing`, `conflicts`, `changesRequested`,
  `awaitingYou`, and `unreviewed` (not a draft, no reviews). Repositories are asked in parallel,
  and one that fails is named on stderr while the rest are listed.
- **`ys pr checkout`** is the view's **Start workspace** without the composer.
  `prepare_branch` moved from the app into `yardsort_core::forge::checkout` so both use it.
  It then calls `Workspaces::open_branch`. A workspace already on the branch is returned, not
  duplicated.

Reviewing someone else's pull request then takes two commands: `ys pr checkout`, then
`ys workflow run code-review`. At ask first each waits for Allow. A fork's `pr/<number>` branch
is not one the review can find by branch name, which is open question 54.

## Not in this slice

- **Resuming its conversation.** **Start again** is a new conversation. The harness's session
  id could be recorded and resumed as a workspace's are, but the Floor Manager has no workspace
  to hang the record on.
- **Cloud runs.** Yardsort has none. The instructions say so, rather than pretend.
- **Starting it with a question already in it**, for instance _Ask the Floor Manager_ on a pull
  request row. `Launcher::coordinator` takes a first message already; nothing in the window sends
  one yet.
