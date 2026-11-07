# Triggers

_Proposal, 2026-11-06. The questions put to the user that day are answered under
[Decisions](#decisions). Slices 1 and 2 are built: see [Slice 1](#slice-1-what-shipped) and
[Slice 2](#slice-2-what-shipped)._

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
queued runs" ([21](21-workflows.md#slices)), and schedules proved it: a schedule calls
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
   post to GitHub as the user's `gh` login, and the agent that addresses a review answers each
   line comment as that account. Counting the user's own account would have the agent's answers
   start the next round of answering. So by default only posts from other accounts count:
   teammates and review bots. `from: anyone` counts the user's account too, except posts made
   while a run this trigger started is going, which are the agent's own answers. A review that
   Yardsort's own `code-review` posts is told to the author by that workflow's own last step, so
   it needs no trigger, and with the default it starts none.

   Three kinds of post are watched, from two reads. `gh pr view --json reviews,comments`, which
   `forge::pr_posts` reads today, has each submitted review — one post, with its line comments
   inside it — and each comment on the conversation. A reply on a review thread is in neither:
   it is a line comment with `in_reply_to` (`forge::LineComment`), read through the pull
   request's comments API, which the `code-review` prompt already names. The watch reads that
   too, so a teammate who answers on a thread starts the next round, as the usual way a review
   continues. GitHub does also record each reply as a submitted review of its own — checked in
   slice 2 against a real pull request, below — and a post is counted once whichever list it
   shows in.

7. **A burst is one fire.** A review with ten line comments is already one post. What the quiet
   time folds is several posts: two reviews, three comments and a reply posted in a few minutes
   start one run, not six. A trigger on reviews and comments waits until nothing new has arrived
   for its `quiet` time, two minutes by default, and at most ten minutes from the first, then
   fires once with everything that came. Other events fire at once.
8. **An event that comes while a run is in the way waits for it.** The store allows one active
   run of a workflow, or of a test, in a workspace, whoever started it
   (`workflow_runs_one_at_a_time`, and `test_runs_one_at_a_time`, which counts a run awaiting
   approval as active), and the door refuses a second with `workflow_already_running` and writes
   nothing. A schedule records that
   as skipped ([34, decision 6](34-schedules.md#decisions)), because the next time will come. A
   review that arrives while the agent is answering the previous one will not come again. So a
   fire that meets that refusal — whether the run in the way is this trigger's, another
   trigger's, or one a person started — is held as **waiting**, and decided again when that run
   ends, however many events piled up meanwhile. One fire waits at most.
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
     Nothing has a depth today. A workflow run's goes in its `Facts`, in `workflow_runs.context`,
     beside `unattended`. A test run a trigger queued directly has the depth of its fire. A test
     run a workflow's `run_test` queued has the depth of that workflow run, found through the
     step's one output, `run`, which is the test run's id.
10. **Triggers work while Yardsort is open, as schedules do.** A pull request that opened, a
    review that came or checks that failed while the app was closed are found at the first look
    after it opens, and fire once, as one burst. An event older than the trigger itself never
    fires. For the agent events and the done signal, a trigger's cursor starts at the newest row
    when it is saved. For the `pr.*` events, which are differences from what was seen last, saving
    the trigger writes its pull request's watch from the forge's answer at that moment, and fires
    nothing: a pull request already open is not `pr.opened`, and checks already red are not
    `pr.checks_failed`. That seed is also what makes a later change, made while the app was
    closed, a real one.
11. **A trigger's run may wait for the user's approval, and for nothing else a person does.** The
    doors are called with `unattended: false`, as a schedule's own test is (`ask_for_run` in
    `schedule::fire`), and not with `true`, as a scheduled workflow is. An agent writes a smoke
    test on its branch as it works, so the file a "smoke test when done" trigger runs is new or
    changed nearly every time; refusing an unapproved file, as `smoke::runs::queue` does when
    unattended, would make that preset refuse every time. So a new or changed file waits on the
    workspace's card, the trigger reads _waiting for approval_ and is counted on the sidebar, and
    the user is told once. Two rules keep the wait bounded:
    - **A test with a `manual` step is refused**, by the trigger's own check of the file, when the
      trigger is saved and at each fire, for a test it runs directly and for one a workflow's
      `run_test` names. Nobody is there to press **Pass**.
    - **A workflow is accepted only if each of its `run_test` steps has a `timeout`.** A test
      named by an input is not known until the run; if it waits for a person, the step's timeout
      ends it. So does an approval that does not come.

    A run that waits holds the one-run lock, so the trigger's next events wait behind it
    (decision 8). Nothing answers an agent's permission prompt on the user's behalf.

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
means any agent the user started there. A shell is already left out, since only sessions that
carry `HARNESS_LABEL` are agents (`origin_session` in the driver). An agent a workflow run started
must be left out too — the reviewer settling must not fire "the agent settled" — and nothing on
the session says a workflow started it: `agent_runs.launched_by` is `app` or `cli`, and the launch
labels are the same for every harness launch. The run that started it does say so: a
`start_session` step leaves the terminal's id as `session` and the agent run's id as `run` in its
outputs. The matcher skips a session whose terminal or agent run is in a `start_session` output.
Its runs' `session: origin` is what it always is: the workspace's newest agent session running
when the run starts.

**A session trigger** belongs to one agent session. The `agent.*` events are that session's only,
and its runs' `session: origin` is that session, whatever else is running in the workspace. That
needs a change in the driver, not only in the request. Today, when a queued run starts and its
workflow needs an origin, the driver sets `facts.origin` to the newest live agent session's
terminal id (`origin_session`), over whatever `queue` stored. So the request gains the session's
record id, `queue` keeps it in `Facts`, and at start the driver resolves that record to its live
terminal's id instead of choosing the newest, and does not overwrite it. `Facts`'s description of
`origin` changes with it. It fires only while that session's program runs: an event while it does
not is recorded as **skipped**, since there is no one to tell. It goes when the session's record
does.

## Events

| Event               | Happens when                                                                 | Subject            | Filters                                                    | Leaves in `{{ event.* }}`            |
| ------------------- | ---------------------------------------------------------------------------- | ------------------ | ---------------------------------------------------------- | ------------------------------------ |
| `workspace.done`    | An agent ran `ys workspace done`                                             | workspace          | none                                                       | `note`, `head`                       |
| `agent.settled`     | An agent went quiet after real work, or its harness reported a finished turn | session            | `new_commits`, `clean`, `quiet`                            | `head`                               |
| `agent.exited`      | An agent's program ended                                                     | session            | `exit`: `any`, `failed` (non-zero)                         | `summary`                            |
| `pr.opened`         | The workspace's branch has an open pull request it did not have before       | pull request       | `draft`: include or not                                    | `url`                                |
| `pr.review`         | A review was submitted on the workspace's pull request                       | pull request       | `from`: `others` or `anyone`; `state`; `bots`              | `url`, `count`, `authors`, `summary` |
| `pr.comment`        | A comment on its conversation, or a reply on a review thread                 | pull request       | `from`, `bots`                                             | `url`, `count`, `authors`, `summary` |
| `pr.checks_failed`  | Its checks finished on the newest commit, and one failed                     | pull request, head | none                                                       | `head`, `summary`                    |
| `pr.checks_passed`  | Its checks finished on the newest commit, and none failed                    | pull request, head | none                                                       | `head`                               |
| `pr.conflicted`     | GitHub says it conflicts with its base                                       | pull request, head | none                                                       | `head`                               |
| `pr.merged`         | It was merged                                                                | pull request       | none                                                       | `url`, `number`, `title`             |
| `pr.closed`         | It was closed without merging                                                | pull request       | none                                                       | `url`, `number`, `title`             |
| `test.finished`     | A smoke test run in the workspace reached a verdict, or could not            | workspace          | `test`: an id; `verdict`: `passed`, `failed`, `any`        | `verdict`, `summary`                 |
| `workflow.finished` | A workflow run in the workspace ended                                        | workspace          | `workflow`: an id; `outcome`: `succeeded`, `failed`, `any` | `summary`                            |

Notes on each group:

- **The `pr.*` events** are about the workspace's pull request, found as a run finds it: the open
  pull request for the branch the workspace has checked out. Each is seen once: checks that
  failed on a commit fire once for that commit, and a push that brings a new head can fire again.
  `state` is any of `approved`, `changes_requested` and `commented`. `bots: false` leaves out
  accounts whose login ends in `[bot]`. A line comment in a submitted review is part of that
  review; a reply on a thread is a `pr.comment` of its own ([decision 6](#decisions)).
  `pr.merged` and `pr.closed` end the subject: its count and its cursor go.
- **Finished checks are counted, not read off the verdict.** `CheckCounts::verdict` says
  `Failing` as soon as one check has failed, while others may still be running. `pr.checks_failed`
  is the rollup on the head with nothing running and at least one failed; `pr.checks_passed` is
  nothing running, nothing failed and something passed.
- **Only `CONFLICTING` is a conflict.** `Mergeable::Unknown` is GitHub still working it out, and
  `publish/conflicts.rs` does not treat it as one either; the watch waits for a definite answer.
- **`pr.merged` and `pr.closed` are about a pull request that is no longer open**, and
  `workflow::runs::queue` keeps a workflow's pull request only while it is open. A workflow that
  uses `{{ pr.* }}` or `wait_pr_activity` would be refused at every such fire, so saving a
  trigger on them refuses it, saying to use `{{ event.url }}`, `{{ event.number }}` and
  `{{ event.title }}`, which these two leave.
- **`agent.settled`** is the guess decision 5 does not build on. One turn is one observation,
  folded the way the driver folds it for `wait_session`: a session's settled time is the later of
  a quiet after enough output (`Settled::observe`) and a reported `turn.completed`, and the event
  is that time moving, not each of the two signals. The quiet half is held in memory, so after a
  restart only a turn the harness reported can be found; a quiet while the app was closed is not
  a row anywhere and is not recovered. `new_commits` means the worktree's `HEAD` has moved since
  this trigger last fired, or since the session started. `clean` means nothing is uncommitted.
  `quiet` means it stayed quiet that long after settling; being busy again in between starts it
  over. Without filters it fires at every turn, which is what a custom "tell me each time it
  stops" wants and what a review must not have.
- **`workspace.done`** is [the done signal](#the-done-signal). The same signal twice on the same
  commit is one event.
- **`test.finished` and `workflow.finished`** are read from the run rows when a run ends, as a
  schedule's history is (`schedule::fire::settle`), with the run's depth as
  [decision 9](#decisions) says.

The events are ones Yardsort already sees, or can see without reading what an agent prints. The
agent events come from the host's activity, which the workflow driver already follows for
`wait_session`, and from `agent_events` rows, which a reported turn or an exit already writes.
The pull request events come from the watch's own reads, [below](#the-app), not from the forge
cache the badges read: that cache's open list has check counts but no check names, is filled in
full only while the Pull requests view is showing, and drops a pull request once it is merged or
closed.

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
- `{{ pr.* }}` is found when the run is queued, as now. A trigger on any `pr.*` event but merged
  and closed has just seen the pull request open, so the lookup comes back empty only if it was
  closed in between, which is a refusal like any other.

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
terminal it takes `--workspace`. It writes one row with the worktree's `HEAD` and the note, and
needs the app to be open no more than `ys memory propose` does, which also writes straight to
the store: an app that opens later finds the row.

The row is not an `agent_events` row. Every row there is `privacy_class` `metadata`, which by
its migration holds no prompt, command, path or output, and the note is the agent's own words.
So the signal has a small table of its own, `done_signals`, and the note is copied onto the fire
it starts, beside the summary the fire already keeps.

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

1. **Seeing.** What happened arrives as observations: a session's settled time moving (one per
   turn, [as above](#events)), an agent's exit, new `done_signals` rows, run rows that ended, and
   the difference between what the forge says now about a watched pull request and what its watch
   says it said last time. An observation that no enabled trigger matches is dropped; nothing is
   stored for its own sake.
2. **Gathering.** A matching observation joins the trigger's open fire for that subject, or opens
   one, due `quiet` after it — moved later by each new observation, never past ten minutes from
   the first. The fire counts what it has gathered, for `{{ event.count }}` and the summary.
3. **Deciding.** When a fire is due: the trigger is still on; its workspace is there and not
   archived; a session trigger's session is running; the subject's count is under `cap`; the
   hourly rate is under ten; the depth is at most 3; a test it would run has no `manual` step.
4. **Queueing.** The door is called with the trigger's workspace, inputs, session record and
   depth, and `unattended: false` ([decision 11](#decisions)). If it answers that a run of the
   same workflow or test is already active in the workspace, whoever started it, the fire is
   held as **waiting** and decided again when that run ends ([decision 8](#decisions)). Otherwise
   the workflow driver or the smoke runner is woken, and from there the run is the run as it
   already is.

Each fire leaves one line in the trigger's history:

| Outcome   | Meaning                                                                         |
| --------- | ------------------------------------------------------------------------------- |
| `started` | A run was queued. The line links to it, and shows its result once it has one    |
| `waiting` | A run of the same thing is active there; this one is decided when it ends       |
| `skipped` | There was no one to run it for: a session trigger whose session was not running |
| `capped`  | The subject has had `cap` runs. Nothing more fires for it until **Reset**       |
| `refused` | The door refused, with its reason, or the rate or the depth was over its limit  |

**Told.** The trigger's `notify` decides about runs that end, as a schedule's does. A run left
waiting for approval is told once, whatever `notify` says, as a schedule's is. `capped` and
`refused` have no run to end, so they are told when they happen, once while they last, which is
what review settled for schedules.

## Storage

One migration, `0023_triggers.sql`, with four tables:

- **`triggers`**, one row per trigger, as [above](#the-trigger), with the events, filters and
  inputs as JSON, the workspace (set to NULL when it is deleted, its name kept), the session
  record (the row goes with it), and a cursor: what this trigger has already seen, so a restart
  starts where it stopped.
- **`trigger_fires`**, one row per fire: the subject, when its first observation came, when it
  is due, when it was decided, how many events it gathered and their summary, the outcome, the
  run's id or the reason, the depth, and whether it has been told. The newest 200 per trigger are
  kept, and any fire still open, waiting or with a run that has not ended is kept whatever its
  age — the lesson schedules learnt the day after they shipped.
- **`pr_watches`**, one row per watched workspace: its pull request's number once there is one
  (none yet is a state too, so that `pr.opened` is a change), its head, its checks' counts on that
  head, whether it conflicts, whether it is open, merged or closed, and the ids of the reviews,
  comments and thread replies seen. It is written when a trigger is saved
  ([decision 10](#decisions)) and after each look. It is what makes "checks failed" an event once
  rather than a state seen every minute, and what lets an app that was closed find what changed
  meanwhile.
- **`done_signals`**, one row per `ys workspace done`: the workspace, the session record, the
  `HEAD`, the note and the time.

A removed project's triggers are neither listed nor fired and go with its row; removing a
trigger removes its history and nothing else.

## The app

A `triggers` thread, started only in the app that holds the profile's lock, beside the schedules
thread, for the reason that one has its own: asking the forge must not stall anything else. It
ticks once a second. It is given the host's events, as the workflow driver is, keeps each
session's settled time as the driver does, reads `agent_events` and `done_signals` from its
cursor, and copies run endings onto history lines.

Once a minute, for each watched workspace, it looks at the pull request itself, on a thread of
its own with the 20-second limit, as `wait_pr_activity`'s questions are:

- **Until the watch has a number**, it finds the workspace's pull request as a run does
  (`Look::pull_request`).
- **From then on it asks by number**, so a merged or closed pull request is still answered for:
  `gh pr view <number> --json url,title,state,isDraft,headRefOid,mergeable,statusCheckRollup,reviews,comments`.
  That is one call for the state, the check names and counts, whether it conflicts, the reviews
  and the conversation's comments. The forge cache is not used for this. Its open list has
  counts without names and is filled in full only while the Pull requests view is showing, and
  `Forge::follow_open` returns without asking when that list was never loaded, and stops asking
  once the list has dropped a merged or closed pull request — which is the change `pr.merged` and
  `pr.closed` are.
- **When a trigger listens for `pr.comment`**, it also reads the pull request's line comments
  through the comments API, for the replies on threads.

The watch keeps the ids seen, so a post is one observation however many looks see it. A watch
whose pull request was merged or closed is dropped after the look that saw it, and a new pull
request on the same branch starts a new subject.

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
until the app is open. `ys` cannot approve a test file, so a trigger's run of a test whose file
is not approved waits for approval in the app, as any triggered run does
([decision 11](#decisions)).

## The UI

- **The composer**: the presets, with their one question each, ticked as they were last time in
  that project.
- **Triggers** in the sidebar, under **Schedules**, with a count of triggers that need the user:
  capped, refused, or with a run waiting for approval.
- **The Triggers view**: every trigger, with its name, its event in words (_When a review comes
  in on #142_), where (a workspace, or a session's tab), what it runs, its last fire and its
  badge — on, running, waiting, waiting for approval, capped, refused, off, ended. A row opens
  to its switch, its history with a link to each run, **Edit…**, **Reset** when capped, and
  **Remove…**.
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
   table and first-message section, `agent.settled` folded per turn, `agent.exited`,
   `test.finished`, `workflow.finished`, the `event` namespace in the workflow parser, the depth
   and the session record in `Facts`, the driver keeping a stored origin, the matcher leaving out
   sessions a `start_session` started, the app's thread, `ys trigger` and `ys workspace done`. A
   trigger added from a terminal fires in an open app.
2. **The forge.** `pr_watches` and its seed at saving, the look by number with the check rollup,
   the author and state on each post, thread replies from the comments API, every `pr.*` event,
   the three built-in workflows, and the presets. Confirm against a real pull request whether
   GitHub records a reply on a line as a submitted review too (decision 6).
3. **The app.** The composer's presets, the sidebar row and its count, the view, the dialog, the
   menus, the marks in run lists, and the notifications for capped and refused.
4. **Custom events.** `ys event emit <name>`, and `on: custom.<name>`: a named event a workflow, a
   test's `run` step or an agent can raise, for the cases no built-in event covers.
5. **Documentation**, finished: a new guide, the command-line guide, the workflows and schedules
   guides, the README and site, the changelog, a screenshot, and a seed for the throwaway
   profile.

Each slice ends with a check by hand in a throwaway profile, written down under "what shipped".
Slice 2's needs a real pull request on a scratch repository, reviewed from a second account.

## Slice 1: what shipped

The core, `ys`, and the app's thread, with Yardsort's own events: `workspace.done`,
`agent.settled`, `agent.exited`, `test.finished` and `workflow.finished`. No window, no pull
requests and no presets yet.

**The core** (`crates/core/src/trigger/`, `store/triggers.rs`, migration `0023`). `mod.rs` holds
what a trigger may be and the checks at saving; `fire.rs` the three steps — `observe`, `tick`,
`settle`; `sources.rs` where observations come from; `turns.rs` the folding of a quiet and a
reported turn into one `agent.settled`. **The app** (`src-tauri/src/triggers/`): a `triggers`
thread, started only in the app that holds the profile's lock, after the schedules thread, given
every host event, ticking once a second. **`ys trigger`** and **`ys workspace done`**. The
workflow parser has the `event` namespace, `Facts` the depth, the event and the pinned session,
and the driver keeps a pinned session as `origin`.

What building it settled or changed:

- **Four tables, not three, and not `pr_watches` yet.** `triggers`, `trigger_fires`,
  `done_signals`, and `trigger_cursors`: where the thread has read up to in `agent_events` and
  `done_signals` by sequence number, and in run endings by time. A cursor read for the first time
  starts at the newest row, so what happened before an upgrade is never news. `pr_watches` comes
  with slice 2, in a migration of its own, since nothing here would write it.
- **A trigger's cursor is a time, `since`**, set when it is saved and when it is switched on. An
  observation older than that is dropped. The thread's cursors are shared by every trigger.
- **`quiet` is the trigger's, for every event.** The design gave `agent.settled` a filter of its
  own for staying quiet; it is the same thing as the trigger's quiet time, with one rule more: an
  agent going back to work lets its gathering fire go, without a line.
- **A trigger is about one kind of subject.** An agent's events count their runs per session
  (`session:<record>`), the rest per workspace, so a trigger cannot listen for both kinds; saving
  one that does is refused, saying to make two.
- **A capped subject keeps one line.** Events after the cap go onto the capped line's count
  instead of each writing a line of their own, so a loop that is stopped does not fill the
  history.
- **Run endings are read a second behind the clock**, by time, so a run another process ends in
  the millisecond of the read is not stepped over.
- **An agent a workflow started is found from the step that started it**: a `start_session`
  step's `session` or `run` output, by `json_extract`. Nothing needed to change on the session.
- **The filters' flags are named for what they filter**: `--of-test` and `--of-workflow`, since
  `--test` and `--workflow` already name what to run, and `--failed-exit`.
- **The presets wait for slice 2**, so they arrive together; four of the five need a pull request
  event or `review-branch`.
- **An agent already running is not told about `ys workspace done`.** `ys trigger add` says so;
  the dialog's **Tell the agent** is slice 3's.
- **The `event` namespace lists every field the design names**, including the ones only slice 2's
  events leave. Saving a trigger refuses a workflow that uses one its events do not leave, so a
  file can be written for slice 2's events now and be refused only where it would be empty.

Checked by hand on 2026-11-07, in a development build on Linux with the throwaway profile and
demo projects `scripts/screenshots.sh` sets up, triggers added with the build's own `ys`:

- A trigger on `workspace.done` with a three-second quiet time, and a second on
  `workflow.finished` of the first's workflow. Two `ys workspace done` a second apart became one
  run, given `event.count` 2, the newer note and the worktree's commit; its ending started the
  second trigger's run at depth 1, given the first run's summary. Both histories showed each
  fire and its result.
- It found one mistake, fixed before this was recorded: saving refused a workflow that used
  `{{ event.count }}`, which every fire leaves.

Not checked by hand: an agent's turns and exits through the host, which needs an agent in a
terminal; the tests cover the folding, the rows and the matching with a real store. macOS and
Windows have had the tests in CI only.

## Slice 2: what shipped

The forge: every `pr.*` event, the watch, the presets, and the three built-in workflows,
`address-review`, `fix-checks` and `review-branch`. Still no window.

**The core.** `forge/watch.rs` asks GitHub about one pull request and reads the answer;
`trigger/watch.rs` keeps what the last look saw (`pr_watches`, migration `0024`) and says what
is news, as observations for `fire::observe`, by a function of the two looks and the time;
`trigger/presets.rs` the five presets. `Look` gains `watch_pull_request`, so the app and `ys`
both seed a watch when a trigger is saved. **The app** has a second thread,
`trigger-pull-requests`, that looks at each watched pull request once a minute and hands what it
finds to the trigger thread, which stays the only one that gathers and decides. **`ys trigger
presets`**, **`--preset`**, and the filters `--drafts`, `--from`, `--state` and `--no-bots`.

What building it settled or changed:

- **A reply on a thread is also a review — confirmed.** On #182 of this repository, each of
  thirteen replies made with the API's reply endpoint is listed twice: as a line comment with
  `in_reply_to`, and as a submitted `COMMENTED` review with no body whose one comment is that
  reply. So a review with no words whose every comment answers another is read as one
  `pr.comment` per reply and is not a `pr.review`; a review that says something, or comments on
  a line afresh, is a review, and replies made in it are part of it.
- **One GraphQL question, not three `gh` calls.** `gh pr view --json` has no replies and does not
  say whose account `gh` is, and the comments API's review ids are not `gh pr view`'s. One query
  by number, through `Gh::graphql` as the open list already uses, has the state, the head, the
  check runs by name and conclusion, whether it conflicts, every review with the comments in it
  and their `replyTo`, the conversation's comments, and `viewer`. GraphQL names a bot without the
  `[bot]` the REST answers and the pages put after it, so it is put back from `__typename`.
- **What a look is checked against is a seeded watch, and a watch already there is never
  re-seeded.** Saving a trigger seeds its workspace's watch only when there is none: another
  trigger is watching with it, so it is current, and seeding it again would mark as seen what
  that trigger has not been told. A trigger switched back on needs nothing: its watch is current,
  or was dropped while nothing watched. When the forge cannot be asked at saving, the first look
  seeds the states — opened, checks, conflict — and tells none of them; reviews and comments are
  news either way when they are newer than the trigger, which `observe` already checks.
- **Another branch checked out is another pull request**, seeded afresh rather than told.
- **A draft is `pr.opened` when it is marked ready**, unless the trigger takes drafts, which is
  told when it opens instead.
- **A fire with no agent to tell is skipped, and told.** A workflow that needs `session: origin`
  started in a workspace with no agent running skips every step and succeeds, so with the
  default `notify` the review it was for would go unanswered without a word. Such a fire is
  **skipped** instead — the session trigger's rule, for the whole workspace — and the user is
  told once while it lasts, as for capped and refused. Found in the check by hand.
- **Reviews and comments gathered into one fire add up**: the summary counts each kind and names
  each author once, rather than being the newest post's.
- **The presets' filters are all the defaults** — others' posts, drafts left out — so a preset
  is an event, a workflow, a quiet time and at most one question.
- **`review-branch` reads `{{ event.note }}`**, so it can be triggered only by `workspace.done`;
  a trigger on `agent.settled` with it is refused when saved. Its reviewer's findings reach the
  author through `ys workspace context`, which needs the project's conversations shared, as they
  are unless the user turned that off.
- **The screenshot script's stand-in `gh`** answers the watch's question from
  `watch-<number>.json` when there is one.

Checked by hand on 2026-11-07, in a development build on Linux with the throwaway profile
`scripts/screenshots.sh` sets up, its stand-in `gh` answering for a pull request #31 on a new
workspace's branch, and a stand-in agent — a custom harness that echoes what it is sent and works
for ten seconds when asked to address something:

- `ys trigger add --preset address-reviews --quiet 20s` and a custom trigger on
  `pr.checks_failed` running `fix-checks` both said _Watching #31_; the review already there was
  not news.
- A comment from `coderabbitai[bot]`, a reply of the user's own, and a review dated before the
  trigger: one run of `address-review`, given _1 comment from coderabbitai[bot]_. The agent was
  told, once quiet, with the link and the `gh` commands; the run waited for it to finish
  answering and succeeded, and the trigger's history took the ending.
- With the app closed: a review from bob, a comment from alice, another reply of the user's, and
  a push whose checks failed. At the first look after it opened, one fire of two events, _1
  review and 1 comment from bob and alice_, and one of `fix-checks`, _1 check failed: test
  (ubuntu)_ on the new head. Nothing from the first round was told again.
- Its agent had ended with the first app, and both runs skipped every step and called it a
  success. That is the skip above; with it, the next comment was **skipped**, _No agent was
  running in `add-fahrenheit-flag`_, and no run was started.

Not checked by hand: a review from a second real account on a real pull request. The query was
run against GitHub itself, on #182 and #184, and read as described above; the rest is the
stand-in's answers in GitHub's shape. macOS and Windows have had the tests in CI only.

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
