# Triggers

_Proposal, 2026-11-06. The questions put to the user that day are answered under
[Decisions](#decisions). Nothing is built yet._

A trigger starts a [workflow](21-workflows.md) or a [smoke test](32-smoke-tests.md) when
something happens, instead of when a person presses **Run** or a [schedule](34-schedules.md)'s
time comes. Something happening is an agent finishing its work, a pull request opening, a review
or a comment arriving, the checks failing, a test or a workflow ending. A trigger is attached to
one workspace or to one agent session, and what it starts is an ordinary run: it shows in the
workflow's runs or in **Tests** like any other, marked with the trigger that started it.

What it is built for:

1. **A review when the work is done.** The agent finishes its task and says so; a second agent
   reviews the branch and tells the first what it found. Or, once a pull request is open, the
   built-in code review runs on it without anyone asking.
2. **Reviews answered as they arrive.** A teammate, or a review bot, posts a review on the
   workspace's pull request. A few minutes after the last comment of that review, the agent that
   wrote the pull request is told, once it is quiet, to address it, push, and answer each
   comment. The next review starts the next round.
3. **Checks fixed when they fail.** The pull request's checks fail on its newest commit; the
   agent is told which, and fixes them.
4. **A smoke test when the agent is done**, with the verdict sent back to it.

What it is not: a way to run anything while Yardsort is closed, a CI system, or a rule engine.
A trigger is the user's standing instruction to press **Run** when a particular thing happens,
kept by the app while it is open.

Workflows were built so that "triggers other than manual are a matter of adding a producer of
queued runs" ([21](21-workflows.md#open-questions)), and schedules proved it: a schedule calls
the doors a person's click calls, `workflow::runs::queue` and `smoke::runs::queue`, and nothing
after that changed. A trigger is the third producer. Almost everything in this document is about
what comes before the door: seeing that something happened, once, and deciding whether it
should start a run.

## Decisions

Four questions were put to the user on 2026-11-06, and the recommendation was taken on each
(1, 5, 6 and 10 below). The rest follow from those and from the code as it is.

1. **A trigger belongs to a workspace or to one agent session, and the composer offers them.**
   No project-wide trigger that silently applies to every workspace: something that types into
   agents should be visible where the work starts. The composer has a short list of
   [presets](#presets) with checkboxes. Each ticked one becomes a trigger on the new workspace,
   and the composer remembers, per project, which were ticked last time. A trigger can also be
   added to a workspace or a session later, from its menu or from `ys`.
2. **A trigger is a row in the store, not a key in a file.** This is the reasoning of
   [schedules' decision 4](34-schedules.md#decisions) again: a trigger names a workspace, maybe a
   session, and the value of each input, and a repository must not get to decide what runs on
   the user's machine. A workflow file keeps `trigger: kind: manual` as its only trigger. A file
   can be run by hand or by a trigger, and need not say which.
3. **A trigger starts a workflow or a smoke test, and nothing else.** "Tell the agent to address
   the review" is not a third kind of action but a built-in workflow,
   [`address-review`](#built-in-workflows), of one `send_to_session` and one `wait_session`.
   There is then one engine, one list of runs and one history. A user who wants different words
   copies the workflow, as they would `code-review`.
4. **Built-in presets, and custom triggers.** A [preset](#presets) is an event, its filters and a
   workflow or test, chosen and named by Yardsort, like a built-in workflow. A custom trigger is
   any [event](#events), filter and target the user puts together. Both are the same row; a
   preset only fills it in.
5. **"Done" is something the agent says.** An agent settles at the end of every turn, including
   the turns where it stops to ask a question, so settling is not done. A new
   `ys workspace done` is the [signal](#the-done-signal): the agent runs it when its work is
   committed and ready to be looked at, and it is told about it in its first message only when a
   trigger in its workspace listens for it. This needs the agent's cooperation, and is exact, and
   reads nothing the agent printed. The guess — the agent settled, with new commits and a clean
   worktree, and stayed quiet for a few minutes — is offered as an event of its own,
   [`agent.settled`](#events) with filters, for custom triggers. No preset uses it.
6. **Reviews and comments count when they come from another account.** Every agent and the user
   post to GitHub as the user's `gh` login. The agent that addresses a review answers each line
   comment, and GitHub records each such reply as a review of its own by that account. Counting
   the user's own account would therefore have the agent's answers start the next round of
   answering. So by default only posts from other accounts count: teammates and review bots.
   `from: anyone` counts the user's account too, except posts made while a run this trigger
   started is going, which are the agent's own answers. That GitHub records a reply as a review
   is to be confirmed against a real pull request in slice 2; the default is right either way.
   A review that Yardsort's own `code-review` posts is told to the author by that workflow's
   own last step, so it needs no trigger, and with the default it starts none.
7. **A burst is one fire.** A review with ten line comments, or three comments posted in a
   minute, starts one run, not ten. A trigger on reviews and comments waits until nothing new has
   arrived for its `quiet` time, two minutes by default, and at most ten minutes from the first,
   then fires once with everything that came. Other events fire at once.
8. **An event that comes while the trigger's run is going waits for it.** A schedule skips such
   a time ([34, decision 6](34-schedules.md#decisions)), because the next time will come. A
   review that arrives while the agent is answering the previous one will not come again. So it
   is held, and starts one run when the current one ends, however many events piled up meanwhile.
   One fire waits at most.
9. **A trigger stops itself before it loops.** Three limits, each told to the user once:
   - **Rounds.** At most `cap` runs per subject — per pull request for the `pr.*` events, per
     session for `agent.*`, per workspace for the rest. Three by default. The fire that would be
     the fourth is recorded as **capped**, the user is told, and the trigger fires nothing more for
     that subject until they press **Reset**.
   - **Rate.** At most ten runs an hour per trigger, whatever the subjects. This only backs the
     cap up.
   - **Chains.** A run a trigger started can end in an event another trigger listens for
     (`test.finished`, `workflow.finished`). Each run carries a depth: a person's run is 0, and a
     run started by an event from a run of depth _n_ is _n_ + 1. A fire past depth 3 is refused.
10. **Triggers work while Yardsort is open, as schedules do.** A pull request that opened, a
    review that came or checks that failed while the app was closed are found at the first look
    after it opens, and fire once, as one burst. An event older than the trigger itself never
    fires: a trigger's cursor starts when it is saved.
11. **A run a trigger starts is unattended**, with every rule a schedule's run has
    ([34, decision 8](34-schedules.md#decisions)): a smoke test with a `manual` step, or whose
    file is not approved as it is now, is refused when the trigger is saved and again at each
    fire, and a test named by an input fails its step at once instead of waiting. Nothing answers
    an agent's permission prompt on the user's behalf.
12. **The run tables are not touched**, for [schedules' reason](34-schedules.md#decisions): a
    triggered run is queued with `requested_by: app`, and the trigger's own history holds the
    run's id.
13. **The forge is asked about only what a trigger watches.** A pull request is polled only while
    an enabled trigger in its workspace listens for a `pr.*` event, at most once a minute, and
    every question to `gh` has the 20-second limit the doors already use.
14. **`ys` is a second door, as everywhere.** `ys trigger` adds, lists, pauses, resumes and
    removes triggers by writing the same rows; the app sees them within a second.

## The trigger

| Field     | What it is                                                                                        |
| --------- | ------------------------------------------------------------------------------------------------- |
| `name`    | What lists call it. Defaults to the preset's name, or to the event and target in words            |
| where     | A workspace, or one agent session in it. A workspace's name is kept for when it is gone           |
| `on`      | One or more [events](#events) from the same group, such as `pr.review` and `pr.comment`           |
| filters   | What each event allows, below. Every filter given must hold                                       |
| target    | `workflow` or `test`, and its id                                                                  |
| `inputs`  | A value for each of the target's inputs, checked when the trigger is saved                        |
| `quiet`   | How long to wait after the last event before firing. Two minutes for reviews and comments, else 0 |
| `cap`     | Runs per subject before it stops and asks. 3 by default                                           |
| `notify`  | `on failure` (the default), `always` or `never`, as a schedule's                                  |
| `enabled` | Off keeps the trigger and its history and fires nothing; on again starts from now                 |
| `preset`  | The preset it was made from, if any, so the composer and the list can say so                      |

**A workspace trigger** listens to everything in its workspace. For the `agent.*` events that
means any agent the user started there; a shell is not an agent, and neither is an agent a
workflow run started — the reviewer settling must not fire "the agent settled". Its runs'
`session: origin` is what it always is: the workspace's newest agent session running when the
run starts.

**A session trigger** belongs to one agent session. The `agent.*` events are that session's only,
and its runs' `session: origin` is that session, whatever else is running in the workspace; the
door's request gains an `origin` to say so. It fires only while that session's program runs: an
event while it does not is recorded as **skipped**, since there is no one to tell. It goes when
the session's record does.

## Events

| Event               | Happens when                                                                 | Subject            | Filters                                                    | Leaves in `{{ event.* }}`            |
| ------------------- | ---------------------------------------------------------------------------- | ------------------ | ---------------------------------------------------------- | ------------------------------------ |
| `workspace.done`    | An agent ran `ys workspace done`                                             | workspace          | none                                                       | `note`, `head`                       |
| `agent.settled`     | An agent went quiet after real work, or its harness reported a finished turn | session            | `new_commits`, `clean`, `quiet`                            | `head`                               |
| `agent.exited`      | An agent's program ended                                                     | session            | `exit`: `any`, `failed` (non-zero)                         | `summary`                            |
| `pr.opened`         | The workspace's branch has an open pull request it did not have before       | pull request       | `draft`: include or not                                    | `url`                                |
| `pr.review`         | A review was submitted on the workspace's pull request                       | pull request       | `from`: `others` or `anyone`; `state`; `bots`              | `url`, `count`, `authors`, `summary` |
| `pr.comment`        | A comment was posted on its conversation                                     | pull request       | `from`, `bots`                                             | `url`, `count`, `authors`, `summary` |
| `pr.checks_failed`  | Its checks finished on the newest commit, and one failed                     | pull request, head | none                                                       | `head`, `summary`                    |
| `pr.checks_passed`  | Its checks finished on the newest commit, and none failed                    | pull request, head | none                                                       | `head`                               |
| `pr.conflicted`     | GitHub says it conflicts with its base                                       | pull request, head | none                                                       | `head`                               |
| `pr.merged`         | It was merged                                                                | pull request       | none                                                       | `url`                                |
| `pr.closed`         | It was closed without merging                                                | pull request       | none                                                       | `url`                                |
| `test.finished`     | A smoke test run in the workspace reached a verdict, or could not            | workspace          | `test`: an id; `verdict`: `passed`, `failed`, `any`        | `verdict`, `summary`                 |
| `workflow.finished` | A workflow run in the workspace ended                                        | workspace          | `workflow`: an id; `outcome`: `succeeded`, `failed`, `any` | `summary`                            |

Notes on each group:

- **The `pr.*` events** are about the workspace's pull request, found as a run finds it: the open
  pull request for the branch the workspace has checked out. Each is seen once: checks that
  failed on a commit fire once for that commit, and a push that brings a new head can fire again.
  `state` is any of `approved`, `changes_requested` and `commented`. `bots: false` leaves out
  accounts whose login ends in `[bot]`. A line comment is part of the review it belongs to.
  `pr.merged` and `pr.closed` end the subject: its count and its cursor go.
- **`agent.settled`** is the guess decision 5 does not build on. `new_commits` means the
  worktree's `HEAD` has moved since this trigger last fired, or since the session started.
  `clean` means nothing is uncommitted. `quiet` means it stayed quiet that long after settling;
  being busy again in between starts it over. Without filters it fires at every turn, which is
  what a custom "tell me each time it stops" wants and what a review must not have.
- **`workspace.done`** is [the done signal](#the-done-signal). The same signal twice on the same
  commit is one event.
- **`test.finished` and `workflow.finished`** are read from the run rows when a run ends, as a
  schedule's history is (`schedule::fire::settle`). A run's depth is the depth of the fire that
  started it, or 0; a test run a workflow's `run_test` queued has the workflow run's depth, read
  from that step's outputs.

The events are ones Yardsort already sees, or can see without reading what an agent prints. The
agent events come from the host's activity, which the workflow driver already follows for
`wait_session`, and from `agent_events` rows, which a reported turn or an exit already writes.
The pull request's state, its checks and whether it conflicts are in the forge cache the
workspace badges read (`publish::Forge`). Reviews and comments are what `wait_pr_activity`
already asks for, `gh pr view --json url,reviews,comments`, with each post's author and state
added to `forge::PrPost`.

### Variables

Workflows gain one namespace, `event`, with the fields in the table: `event.kind` always, and
the others when the event leaves them. `summary` is one line in words, such as _2 reviews and 3
comments from alice and coderabbitai[bot]_ or _2 checks failed: lint, test (ubuntu)_. `note` is
what the agent passed to `ys workspace done --note`. `authors` is a comma-separated list of
logins.

- **Saving a trigger refuses a workflow that uses a field its events do not leave**, naming the
  variable and the step, the way a schedule refuses an input that does not fit.
- **Run by hand, every `event.*` is empty**, and the Run dialog says so in a note, as it does for
  `{{ workspace.task }}` in a workspace started without a first message. A file is still one
  file whoever starts it.
- `{{ pr.* }}` is found when the run is queued, as now. A trigger on a `pr.*` event has just seen
  the pull request, so the lookup cannot come back empty, short of the pull request closing in
  between, which is a refusal like any other.

## Presets

| Preset                                    | On                                     | Runs              | Asks for     |
| ----------------------------------------- | -------------------------------------- | ----------------- | ------------ |
| **Review when the work is done**          | `workspace.done`                       | `review-branch`   | the reviewer |
| **Review the pull request when it opens** | `pr.opened`, drafts left out           | `code-review`     | the reviewer |
| **Address reviews**                       | `pr.review`, `pr.comment`, from others | `address-review`  | nothing      |
| **Fix failing checks**                    | `pr.checks_failed`                     | `fix-checks`      | nothing      |
| **Smoke test when the work is done**      | `workspace.done`                       | `smoke-then-tell` | the test     |

In the composer they are a short list under the prompt, with the one question each asks beside
it: _☐ Review when the work is done, by [Codex ▾]_. `ys trigger presets` lists them, and
`ys trigger add --preset address-reviews` uses one.

### Built-in workflows

Three join `code-review` and `smoke-then-tell`, read-only and copied to be changed in the same
way:

- **`address-review`.** Tells the workspace's own agent, once it is quiet, that the pull request
  has `{{ event.summary }}`, with the link and the `gh` commands that read reviews and line
  comments, and asks it to fix what is right, push, and answer each comment. Then it waits for
  that agent to settle, so the run lasts as long as the answering does: posts made meanwhile are
  the agent's own (decision 6), and a new review waits for it to end (decision 8).
- **`fix-checks`.** The same shape: which checks failed on which commit, how to read them
  (`gh pr checks`, `gh run view --log-failed`), fix and push.
- **`review-branch`.** A second agent, one the user picks, reviews the branch's diff against its
  base in the same worktree, reading and running but not editing, as `code-review` does, and
  writes its findings as its last message. Then the workspace's own agent is told, once it is
  quiet, that the review is in the reviewer's tab, and to read it with `ys workspace context` and
  address it. It needs no pull request, which is why it, rather than `code-review`, is the one
  that runs when the work is done. It relies on
  [conversation context](29-conversation-context.md) to carry the findings across.

## The done signal

`ys workspace done [--note "…"]`, run in an agent's terminal, says the work in this workspace is
committed and ready to be looked at. It is known which workspace and which session are asking by
`YARDSORT_WORKSPACE_ID` and `YARDSORT_SESSION_RECORD_ID`, as for `ys test run`; outside an agent's
terminal it takes `--workspace`. It writes one `agent_events` row, kind `workspace.done`, with
the worktree's `HEAD` and the note, and needs the app to be open no more than
`ys memory propose` does: an app that opens later finds the row.

The agent is told about it in its first message, in a short section after the memory and the
smoke tests, **only when an enabled trigger in its workspace listens for `workspace.done`** when
it starts. It says to run it once the work is committed, and not to run it to pause or to ask
something. An agent that started before the trigger was added was not told; the trigger dialog
says so and offers **Tell the agent**, which sends the same paragraph by `send_to_session`'s rule:
once the agent is quiet, never into a busy one. No harness parses or answers anything here: the
agent runs a command, as it runs `gh`.

## Firing

Everything a trigger does is in four steps, each a function over rows that is handed the time
and never reads the clock, so that every rule above is a test with a made-up clock:

1. **Seeing.** What happened arrives as observations: the host's quiet and exit events, new
   `agent_events` rows after the trigger thread's cursor, run rows that ended, and the difference
   between what the forge says now about a watched pull request and what it said last time. An
   observation that no enabled trigger matches is dropped; nothing is stored for its own sake.
2. **Gathering.** A matching observation joins the trigger's open fire for that subject, or opens
   one, due `quiet` after it — moved later by each new observation, never past ten minutes from
   the first. The fire counts what it has gathered, for `{{ event.count }}` and the summary.
3. **Deciding.** When a fire is due: the trigger is still on; its workspace is there and not
   archived; a session trigger's session is running; the subject's count is under `cap`; the
   hourly rate is under ten; the depth is at most 3. If the trigger's previous run is still going,
   the fire is held as **waiting** and decided again when that run ends.
4. **Queueing.** The door is called with the trigger's workspace, inputs and `origin`, and
   `unattended: true`. The workflow driver or the smoke runner is woken, and from there the run
   is the run as it already is.

Each fire leaves one line in the trigger's history:

| Outcome   | Meaning                                                                         |
| --------- | ------------------------------------------------------------------------------- |
| `started` | A run was queued. The line links to it, and shows its result once it has one    |
| `waiting` | The previous run is going; this one starts when it ends                         |
| `skipped` | There was no one to run it for: a session trigger whose session was not running |
| `capped`  | The subject has had `cap` runs. Nothing more fires for it until **Reset**       |
| `refused` | The door refused, with its reason, or the rate or the depth was over its limit  |

**Told.** The trigger's `notify` decides about runs that end, as a schedule's does. `capped` and
`refused` have no run to end, so they are told when they happen, once while they last, which is
what review settled for schedules.

## Storage

One migration, `0023_triggers.sql`, with three tables:

- **`triggers`**, one row per trigger, as [above](#the-trigger), with the events, filters and
  inputs as JSON, the workspace (set to NULL when it is deleted, its name kept), the session
  record (the row goes with it), and a cursor: what this trigger has already seen, so a restart
  starts where it stopped.
- **`trigger_fires`**, one row per fire: the subject, when its first observation came, when it
  is due, when it was decided, how many events it gathered and their summary, the outcome, the
  run's id or the reason, the depth, and whether it has been told. The newest 200 per trigger are
  kept, and any fire still open, waiting or with a run that has not ended is kept whatever its
  age — the lesson schedules learnt the day after they shipped.
- **`pr_watches`**, one row per watched pull request: its head, its checks' state on that head,
  whether it conflicts, whether it is open, and the newest review and comment seen with their
  ids. It is what makes "checks failed" an event once rather than a state seen every minute, and
  what lets an app that was closed find what changed meanwhile.

A removed project's triggers are neither listed nor fired and go with its row; removing a
trigger removes its history and nothing else.

## The app

A `triggers` thread, started only in the app that holds the profile's lock, beside the schedules
thread, for the reason that one has its own: asking the forge must not stall anything else. It
ticks once a second. It is given the host's events, as the workflow driver is, reads
`agent_events` from its cursor, and copies run endings onto history lines. Once a minute, for
each watched pull request, it reads the forge cache — asking for a fresh project list when the
cached one is older than the cache's 30 seconds — and asks `gh pr view` for the posts, on a
thread of its own with the 20-second limit, as `wait_pr_activity`'s questions are. A watched pull
request that goes away from the cache is looked up once by number, as `Forge::follow_open` does
for badges, before its watch is dropped.

## The command line

```sh
ys trigger presets
ys trigger list
ys trigger add --preset address-reviews --workspace fix-login
ys trigger add --preset review-when-done --workspace fix-login --input reviewer=codex
ys trigger add --on pr.review --from anyone --workflow address-review --cap 5
ys trigger add --on agent.settled --new-commits --clean --quiet 3m --test login --session 4c1e
ys trigger show <trigger>
ys trigger pause <trigger>
ys trigger resume <trigger>
ys trigger reset <trigger>      # lift a cap
ys trigger fires <trigger>
ys trigger remove <trigger>
ys workspace done --note "Login form validates on blur; tests added."
```

`add` takes `--preset`, or `--on` (repeatable) with the filters as flags and `--workflow` or
`--test`; `--workspace` or `--session` (the workspace the command is run in when both are left
out); `--input id=value`; and optionally `--name`, `--quiet`, `--cap` and `--notify`. It checks
everything the dialog checks. It writes rows with the app closed and says that nothing fires
until the app is open. `ys` cannot approve a test file, so a trigger on a test whose file is not
approved is refused, saying to run the test once in the app.

## The UI

- **The composer**: the presets, with their one question each, ticked as they were last time in
  that project.
- **Triggers** in the sidebar, under **Schedules**, with a count of triggers that need the user:
  capped, refused, or with a run waiting for approval.
- **The Triggers view**: every trigger, with its name, its event in words (_When a review comes
  in on #142_), where (a workspace, or a session's tab), what it runs, its last fire and its
  badge — on, running, waiting, capped, refused, off, ended. A row opens to its switch, its
  history with a link to each run, **Edit…**, **Reset** when capped, and **Remove…**.
- **The trigger dialog**, in the order the questions can be answered: where → when (the events,
  then their filters) → run what (a workflow or a test, then its inputs, the Run dialog's own
  fields) → options (quiet, cap, notify) → a name. It shows in words what will happen: _When
  someone other than you reviews or comments on this workspace's pull request, 2 minutes after
  the last one, tell the agent to address it. At most 3 times per pull request._
- **Triggers…** in a workspace's **⋯** menu and in an agent tab's menu, opening the view on that
  workspace or session with **New trigger…**.
- **Run lists** mark a run a trigger started with the trigger's name, as they do a schedule's.

## Three platforms

Nothing new spawns a program but `gh`, with an argument list, as the forge code already does, and
`ys workspace done` reads environment variables every `ys` command already reads. The time rules
are durations, not wall-clock times, so there is no time zone in play. The trap is the same as
for every `ys` command an agent runs: the PATH a Windows agent's terminal has, which
`ys test run` already relies on.

## Slices

1. **Core and `ys`, with Yardsort's own events.** The migration, the trigger model and its
   checks at saving, the four steps of firing with a made-up clock, `workspace.done` with its
   first-message section, `agent.settled`, `agent.exited`, `test.finished`, `workflow.finished`,
   the `event` namespace in the workflow parser, the app's thread, `ys trigger` and
   `ys workspace done`. A trigger added from a terminal fires in an open app.
2. **The forge.** `pr_watches`, the author and state on each post, every `pr.*` event, the three
   built-in workflows, and the presets. Confirm against a real pull request how GitHub records a
   reply to a line comment (decision 6).
3. **The app.** The composer's presets, the sidebar row and its count, the view, the dialog, the
   menus, the marks in run lists, and the notifications for capped and refused.
4. **Custom events.** `ys event emit <name>`, and `on: custom.<name>`: a named event a workflow, a
   test's `run` step or an agent can raise, for the cases no built-in event covers.
5. **Documentation**, finished: a new guide, the command-line guide, the workflows and schedules
   guides, the README and site, the changelog, a screenshot, and a seed for the throwaway
   profile.

Each slice ends with a check by hand in a throwaway profile, written down under "what shipped".
Slice 2's needs a real pull request on a scratch repository, reviewed from a second account.

## Not in version one, on purpose

- **Project-wide triggers.** The composer's remembered presets cover "every workspace" without a
  rule that applies where nobody looked. Asked for again after use, it is a scope of its own.
- **Triggers that make a workspace** — an issue assigned, a label added. They need a worktree per
  run, which schedules also left open, and what becomes of yesterday's.
- **Running while Yardsort is closed**, for schedules' reasons.
- **Triggers in a file**, for decision 2's.
- **Conditions over the event's content** — "only if the review requests changes on `src/`".
  `state` and `from` are the filters that stop loops; anything finer is a workflow's business.
- **Forges other than GitHub**, which the forge code does not have yet.
- **Triggers on the phone.** Remote exposes no workflows, tests or runs yet.

## Open questions

[48–52 in open questions](06-open-questions.md#triggers): one view for schedules and triggers,
the quiet time for reviews, whether agents remember the done signal, whether a cap should lift
itself, and the phone.

## Documentation this touches

A new `docs/guide/triggers.md`, linked from `docs/README.md` and `website/src/lib/docs.ts`;
`docs/guide/cli.md` (`ys trigger`, `ys workspace done`); `docs/guide/workflows.md` (the `event`
variables, the three built-ins, runs a trigger started); `docs/guide/schedules.md` (its "What it
does not do" points here); `docs/guide/workspaces.md` (the composer's presets);
`docs/guide/pull-requests.md`; `README.md` and the landing page; `CHANGELOG.md`;
[03-architecture](03-architecture.md) (the third producer of runs); [05-roadmap](05-roadmap.md)
(M29); [06-open-questions](06-open-questions.md) (48–52); [08](08-manual-checklist.md).
