# 37 — The Floor Manager

**Status:** first slice built (M30). _Floor Manager_ is a working name; _Floor Planner_ was the
other one offered.

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
   project. That folder holds an `AGENTS.md` (`crates/core/src/coordinator/AGENTS.md`) and a
   `CLAUDE.md` that imports it. Both are rewritten at every start, so a new version's
   instructions arrive with the next start. They say what to run for each kind of question, and
   where the confirmation lines are.
2. **What it can reach.** It reaches `ys`. The launch puts the folder of the app's own `ys` first
   on `PATH`, and sets `YARDSORT_DATA_DIR` to the profile, so `ys` sees exactly what the window
   sees, a development profile included. `ys` already covered projects, workspaces, sessions,
   logs, activity, tasks, workflows, tests, schedules and triggers. What it lacked was pull
   requests, so this slice adds `ys pr` (below).

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

## Confirmation boundaries

These are drawn in the instructions, and the guide states them to the user as instructions, not
as enforcement:

- **Free:** anything that only reads.
- **Proposed, then a yes:** anything that starts an agent, makes a branch or a workspace, runs a
  workflow, a test or a schedule, or writes to GitHub. The proposal names the exact command. A yes
  covers that proposal only.
- **Never:** delete or archive a workspace, delete a branch, force anything, merge, close or
  approve a pull request, edit or commit in a project, type into or stop another agent, approve
  memory, read secrets.

The harness's own permission prompts are a second line, and the only one with teeth. Making the
first line enforced rather than told is [open question 53](06-open-questions.md#floor-manager).

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

Reviewing someone else's pull request then takes two proposals:
`ys pr checkout` → `ys workflow run code-review`. A fork's `pr/<number>` branch is not one the
review can find by branch name, which is open question 54.

## Not in this slice

- **Resuming its conversation.** **Start again** is a new conversation. The harness's session
  id could be recorded and resumed as a workspace's are, but the Floor Manager has no workspace
  to hang the record on.
- **Cloud runs.** Yardsort has none. The instructions say so, rather than pretend.
- **Starting it with a question already in it**, for instance _Ask the Floor Manager_ on a pull
  request row. `Launcher::coordinator` takes a first message already; nothing in the window sends
  one yet.
