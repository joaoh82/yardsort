# Schedules

_Proposal, 2026-11-05, and built the same day. The three questions put to the user are answered
under [Decisions](#decisions); what was built, what it found and how it was checked is under
[What shipped](#what-shipped)._

A schedule starts a [workflow](21-workflows.md) or a [smoke test](32-smoke-tests.md) without a
person pressing **Run**: once, at a time the user picks, or again and again on a routine — every
weekday at nine, every two hours, the first of the month. Until now both could only be started
by hand, from the app or from `ys`, and both design docs put schedules off with the same three
questions: what a run does when the previous one is still going, what happens to a worktree per
run, and how something scheduled asks for permission when there is no one to ask. This document
answers the first and the third, and says why the second waits.

What it is built for:

1. **A smoke test every morning.** The user opens **Schedules**, picks the `login` test and the
   project's own checkout, and says _weekdays at 09:00_. Each morning the run appears in the
   Tests view like any other; a notification arrives only if it failed.
2. **A workflow on a routine.** The same, with a workflow: a review of a long-running branch
   every evening, or a workflow whose steps are an agent and then a `run_test`. A workflow that
   runs tests needs nothing of its own here, because `run_test` is already a step.
3. **Once, later.** _Run this at 18:00, when I have left._ The schedule fires once and is done.

What it is not: a way to run anything while Yardsort is closed, and not a CI system. A schedule
is the user's standing instruction to press **Run**, kept by the app while it is open.

The name is **Schedules**. A schedule either runs **once** or **repeats**; "routine" is the word
for a repeating one in prose, not a second kind of thing with a second list.

## Decisions

Three questions were put to the user on 2026-11-05. The answers, and what follows from each:

1. **Schedules fire only while Yardsort is open.** Runs are carried out by the app that holds the
   profile's lock, and that does not change: no tray icon, no launch at login, no drivers in the
   daemon in this version. Each schedule says what to do with a time that passed while the app
   was closed or the machine asleep: **run once when Yardsort opens**, or **skip it**. See
   [A missed time](#a-missed-time).
2. **A scheduled run happens in a workspace that already exists**, which includes the project's
   own checkout (the workspace called `local`). It runs whatever is checked out there. A fresh
   worktree per run is what recurring agent chores will want, and it waited for an answer to
   what becomes of yesterday's worktree, since Yardsort never deletes work silently. That answer
   came on 2026-11-13: see [A new workspace per run](#a-new-workspace-per-run).
3. **A repeating schedule is written as a preset or as cron.** The dialog offers every N minutes
   or hours, daily at a time, chosen weekdays at a time and monthly on a day; an **Advanced**
   choice takes a five-field cron expression. `ys schedule add` takes either.

The design decisions that follow from those, and from the code as it is:

4. **A schedule is a row in the store, not a `trigger:` in a workflow file.**
   [Workflows](21-workflows.md) said schedules "can become triggers later", and the parser
   refuses `kind: schedule` today. Looking at what a schedule has to hold settles it the other
   way. A workflow file is global to the profile and names no workspace and no input values; a
   schedule has to name both, and the same workflow may be scheduled in three workspaces at
   three times. A smoke test file lives in a repository, and a repository must not get to decide
   when things run on the user's machine. So the file stays what it is, `trigger: kind: manual`
   stays the only trigger, and a schedule is the thing 21 also foresaw: "a producer of queued
   runs". It calls the two doors a person's click calls, `workflow::runs::queue` and
   `smoke::runs::queue`, and everything after that is the run as it already is.
5. **One schedule, one target, one workspace.** A schedule names a workflow or a test, the
   workspace to run it in, and the value of each input. To run two things, make two schedules,
   or one workflow with two steps.
6. **A run that is still going is not joined by a second.** The store already allows one queued
   or running run of a workflow, or of a test, per workspace. A schedule whose time comes while
   its previous run is going records the time as **skipped** and waits for the next. Nothing
   piles up.
7. **Permission is asked of a person, later, and nothing else is invented.** A scheduled test
   whose file has not been approved, or has changed since, is queued as _awaiting approval_
   exactly as one started from `ys` is, and the schedule says so, with a notification. Saving a
   schedule for a test in the app shows the file and approves it then, so the first run does not
   wait. A scheduled agent that stops to ask something waits in its terminal, as an agent
   started by a workflow does, and the existing "is waiting" notification fires. There is no
   flag that approves on the user's behalf.
8. **A test that needs a person cannot be scheduled.** A test with a `manual` step is refused
   when the schedule is saved, in words, rather than left to time out every night. The same
   holds one level down, for a workflow: a `run_test` step waits for its test, with no timeout
   unless the file gives one, so a test that waits for a person would hold the workflow's run
   open for ever and every later time would be skipped behind it. A scheduled workflow whose
   `run_test` names a test with a `manual` step, or one whose file would wait for approval, is
   refused when it is saved and again at each time. A test named by an input is not known until
   the run: the run a schedule starts is marked as unattended, and the door refuses such a test
   there, so the step fails at once instead of waiting.
9. **Times are the machine's local wall-clock time.** _Daily at 09:00_ means nine where the
   machine is, summer and winter. On the night the clocks go forward, a time that does not exist
   fires at the next minute that does; on the night they go back, a time that happens twice
   fires once. The next time is recomputed when the app starts and when the machine's offset
   from UTC changes, so a laptop carried to another zone follows it.
10. **The run tables are not touched.** `workflow_runs.requested_by` is a CHECK of `app` and
    `cli`. Widening it means rebuilding a table whose step rows cascade on delete, inside the
    one transaction every upgrade runs in, with foreign keys on. That is a risk to every
    workflow run in the profile, taken for a label. A scheduled run is queued with
    `requested_by: app`, which is true — the app queued it — and the schedule's own history
    holds the run's id, which is where "started by which schedule" is read from.
11. **The result reaches the user by a notification, per schedule.** _When it fails_ (the
    default), _always_, or _never_. Smoke runs send no notification today because a person who
    pressed Run is watching; a scheduled run has no one watching, which is the point of it.
12. **`ys` is a second door, as everywhere.** `ys schedule` lists, adds, pauses, resumes, removes
    and fires schedules by writing the same rows; the app sees them within a second.

## The schedule

What a schedule holds:

| Field       | What it is                                                                         |
| ----------- | ---------------------------------------------------------------------------------- |
| `name`      | What the list calls it. Defaults to the target's name                              |
| target      | `workflow` or `test`, and its id                                                   |
| workspace   | The workspace to run in, with its name kept for when the workspace is gone         |
| `inputs`    | A value for each of the target's inputs, checked when the schedule is saved        |
| `when`      | One of the kinds below                                                             |
| `if missed` | `run once` (the default) or `skip`                                                 |
| `notify`    | `on failure` (the default), `always` or `never`                                    |
| `enabled`   | A paused schedule keeps its place and its history and fires nothing                |
| `next`      | The next time it will fire, kept in the row so "what is due" is one indexed lookup |

### When

| Kind      | Written as                            | Reads as                         |
| --------- | ------------------------------------- | -------------------------------- |
| `once`    | A date and a time                     | "5 Nov at 18:00"                 |
| `every`   | A number of minutes or hours          | "Every 2 hours"                  |
| `daily`   | A time                                | "Daily at 09:00"                 |
| `weekly`  | One or more weekdays and a time       | "Weekdays at 09:00", "Mon, Thu…" |
| `monthly` | A day from 1 to 28, or the last day   | "Monthly on the 1st at 09:00"    |
| `cron`    | Five fields: minute hour day month dw | "Cron `0 9 * * 1-5`"             |

- `every` counts from when the schedule last fired, or from when it was saved. The shortest is
  one minute.
- `daily`, `weekly` and `monthly` are cron expressions underneath, so there is one engine for
  "the next time after now". They are kept as what the user chose, so the list reads the way it
  was written and the dialog reopens on the same fields.
- `monthly` stops at the 28th, with **last day** as its own choice, so no schedule silently
  skips February.
- `cron` is the standard five fields, with lists, ranges, steps and month and weekday names.
  No seconds field and no `@reboot`. The dialog and `ys schedule show` print the next five
  times beside the expression, which is the reading of it that cannot be wrong.
- A `once` schedule that has fired is kept, switched off and marked **done**, until the user
  removes it. One whose time was missed and skipped reads **missed**, and one whose run could
  not start reads **refused**: neither ran, and neither says it did. A `once` whose time is
  already past when it is saved is refused.

### A missed time

A time is **missed** when the app notices it more than five minutes late: it was closed, or the
machine was asleep. The schedule's own setting then decides:

- **Run once** (the default). One run, now, however many times were missed. A daily schedule on a
  laptop closed for a week runs once on Monday morning, not seven times.
- **Skip.** Nothing runs. The time is recorded as missed and the schedule waits for the next.

Either way the next time is computed from now, not from the missed one.

### What a time can become

Each time a schedule comes due leaves one line in its history:

| Outcome   | Meaning                                                                             |
| --------- | ----------------------------------------------------------------------------------- |
| `started` | A run was queued. The line links to it, and shows its result once it has one        |
| `skipped` | The previous run was still going                                                    |
| `missed`  | The time passed while Yardsort was not running, and the schedule says to skip those |
| `refused` | The run could not be queued, with the reason the queue gave                         |

`refused` is what every refusal at the two doors becomes: the workspace is archived or gone, the
workflow or the test is no longer there or no longer valid, an input no longer fits, a browser
step has no browser, a `{{ env.NAME }}` is not set. Nothing is queued, the schedule is shown as
needing attention, and it tries again at its next time. A schedule whose workspace was deleted
is paused, since no later time can do better. A schedule whose next time cannot be worked out —
its `when` is one this Yardsort cannot read — is refused too, and is left switched on with no
time to come: switched off with no time to come is how a `once` that is done looks.

## The run

A schedule adds nothing to a run. What is new is before it and after it.

**Before.** A `schedules` thread in the app, started only by the app that holds the profile's
lock, looks once a second for schedules whose `next` has come. For each one it moves `next`
forward in a single conditional write — so a time is claimed once, whatever else is writing the
store — and then decides: missed and skipped, or queue. Queueing calls
`workflow::runs::queue` or `smoke::runs::queue` with the schedule's workspace and inputs; a
refusal of "already running" becomes `skipped`, any other becomes `refused`. Then it wakes the
workflow driver, which carries the run out as it would any other. The thread is its own, not the
driver's, because queueing a workflow that needs a pull request asks the forge, and a slow forge
must not stall every run in the profile. It must not stall every other schedule either, this
being the one thread that fires them: `gh` gets 20 seconds to answer, and silence is a refusal
like any other.

**After.** The same thread watches the runs it started. When one ends, the history line takes
its result, and the schedule's `notify` setting decides whether a notification goes out: the
schedule's name, the verdict, and for a failure the first reason. A run left _awaiting approval_
is told once: "needs your approval: the test file changed". A time that was refused has no run
to end, so it is told when it happens, once for as long as the schedule goes on being refused.

**The clock is handed in.** The code that decides what is due and what comes next takes the
time as an argument and never reads it, so every rule above — a missed week, the night the
clocks change, two times in one tick — is a test with a made-up clock, not a wait.

Storage is two tables in one migration:

- `schedules`, one row per schedule, as [above](#the-schedule).
- `schedule_runs`, one row per time a schedule came due: when it was due, when it was acted on,
  the outcome, the run's id when there is one, the reason when there is not, and whether its
  result has been notified. The newest 200 per schedule are kept.

A removed project's schedules are neither listed nor fired, and go with the project's row when
that goes. Removing a schedule removes its history and nothing else: the runs it started stay in
the workflow's and the test's own histories.

## The command line

```sh
ys schedule list
ys schedule show <schedule>
ys schedule add --test login --workspace local --weekly mon-fri 09:00
ys schedule add --workflow code-review --workspace my-branch --input reviewer=codex --daily 18:30
ys schedule add --test login --at "2026-11-06 18:00"
ys schedule add --workflow nightly --cron "0 3 * * *" --if-missed skip
ys schedule pause <schedule>
ys schedule resume <schedule>
ys schedule run <schedule>      # now, without moving its next time
ys schedule runs <schedule>
ys schedule remove <schedule>
```

`add` takes `--workflow` or `--test`, `--workspace` (the workspace the command is run in when
left out, as `ys workflow run` does), any number of `--input id=value`, exactly one of `--at`,
`--every`, `--daily`, `--weekly`, `--monthly` and `--cron`, and optionally `--name`,
`--if-missed` and `--notify`. It checks everything the app's dialog checks and prints the next
time.

`ys` writes rows; it does not need the app to be open to add or remove a schedule, and says
plainly when it is not that nothing will fire until it is. `ys schedule run` refuses with the
app closed, as `ys workflow run` does. `ys` cannot approve a test file, so a test scheduled from
`ys` waits for approval in the app the first time unless its file is already approved.

## The UI

- **Schedules** in the sidebar, beside **Tests**, with a count when any schedule needs attention:
  its last time was refused, or its run is waiting for approval.
- **The Schedules view** lists every schedule in every project: name, what it runs, where, when
  in words, the next time, the last result, and a switch. A row opens its detail: the history,
  each `started` line a link to the run, and **Run now**, **Edit**, **Remove**.
- **The schedule dialog**, for new and for edit, in the order the questions can be answered: a
  workflow or a test → the workspace → which one, since a test is one of that workspace's files →
  its inputs → when → if missed → notify → a name. The inputs are the Run dialog's own fields.
  For a test, the dialog shows the file as the Run dialog does, and saving approves it. For
  cron, the next five times are shown as the expression is typed, or what is wrong with it.
- **Schedule…** beside **Run…** on a workflow and on a test, and in a workspace's **⋯** menu,
  each opening the dialog with what is already known filled in.
- **Run lists** — a workflow's runs, and Tests → Runs — mark a run a schedule started with the
  schedule's name.
- **Remove** asks first, and says what it removes: the schedule and its history, not its runs.

## Three platforms

Nothing here spawns a process or touches a path, so the usual traps are not in play. The one
platform question is the local time zone: the app must read the machine's zone and its daylight
rules on Linux, macOS and Windows, including a Windows machine with no zone database installed.
The time crate is chosen for that, and the rules are tested against fixed zones so the tests do
not depend on where CI runs.

## Slices

1. **Core and `ys`, no window.** The migration and the store, the schedule model and its checks,
   the next-time engine, the firing rules, the app's thread, and `ys schedule`. A schedule can be
   added from a terminal and seen to fire in an open app.
2. **The app.** The commands and bindings, the sidebar row, the view, the dialog, **Schedule…**
   where **Run…** is.
3. **Results.** Notifications, the waiting-for-approval and needs-attention states, the mark in
   run lists.
4. **Documentation**, finished: the guide, the command-line guide, the README and site, the
   changelog, a screenshot.

Each slice ends with a check by hand in a throwaway profile, written down under "what shipped".

## What shipped

All four slices, in one change.

**The core** (`crates/core/src/schedule/`, `store/schedules.rs`, migration `0022`). `mod.rs` holds
what a schedule may be and the checks at saving; `when.rs` the next time; `fire.rs` what becomes
of a time and of the run it started; `clock.rs` is the one place the machine's clock and zone are
read. Times are `chrono` with the machine's local zone, and cron is the `croner` crate, chosen
for what it does on the nights the clocks change, which the tests pin down against fixed zones.
The daily, weekly and monthly kinds are turned into the cron expression they mean, so there is
one engine.

**The app** (`src-tauri/src/schedules/`). A `schedules` thread, started only in the app that
holds the profile's lock, ticks once a second: it works times of day out again when the
machine's offset from UTC changes, fires what is due, wakes the run drivers, copies endings onto
the history, and shows the notifications.

**`ys schedule`**, and **the window**: the sidebar row with its count, the view, the dialog,
**Schedule…** in the three places **Run…** is, and the mark on scheduled runs in a workflow's
runs and in Tests → Runs.

What building it settled or changed:

- **`refused` does not pause a schedule**, except when its workspace is gone. An archived
  workspace restored, or a file fixed, is all it takes for the next time to run. It is counted
  on the sidebar row and, since the review below, told once while it lasts.
- **A schedule's standing says a going run first.** A `once` that has fired reads _running_ or
  _waiting for approval_ until its run ends, and only then _done_. Found by looking at the real
  list, where a fired `once` whose test file needed approval read as done and was not counted.
- **Approval at saving has no run to hang the file's text on.** An approval is a hash; the text a
  changed file is compared against comes from the newest run that carried an approved hash. A
  file approved in the schedule dialog and changed before its first run is therefore shown
  against the version of the last approved _run_, or as new. It shows more than changed, never
  less.
- **The run tables were left alone**, as decided: a scheduled run is `requested_by: app`, and
  `schedule_runs.run_id` is what marks it.
- **An `every` keeps to whole steps from when it last came due**, so being acted on a second late
  does not drift it a second later each time, and a machine that slept through four steps runs
  one, not four.
- **The Run dialog's input fields became a component** (`InputField`), since the schedule dialog
  asks the same questions.

What review of the pull request found, and what was done (2026-11-05):

- **A scheduled workflow could wait for ever for a person, and its schedule never said so.**
  Only a schedule whose own target was a test was checked for a `manual` step. A workflow's
  `run_test` queued its test unchecked: an unapproved file left it _awaiting approval_, the
  step waited with it, with no timeout unless the file gave one, the run stayed active, and
  every later time was skipped. Now refused at saving and at each time, and failed at the step
  for a test named by an input (decision 8). The check for the step is inside
  `smoke::runs::queue`, on the one reading of the file that the run would use, so there is no
  moment between looking and queueing. **Run now** is a person asking, and is not held to it.
- **One `gh` that did not answer held every schedule.** The pull request lookup at queueing
  had no limit. It has 20 seconds now, for every caller, `ys` and the Run dialog included.
- **A refusal is told**, once while it lasts, which settles open question 3.
- **A `once` that was missed and skipped read _done_**, as did one that was refused, and a
  repeating schedule whose next time could not be worked out was switched off and read _done_
  too. The first two now read _missed_ and _refused_; the last stays on and reads _refused_.

Found the day after it merged, before any release had it (2026-11-06):

- **A schedule forgot its run was waiting once a later time was skipped.** Where a schedule
  stands was read from the newest line of its history alone. A run still going, or still
  waiting for approval, makes the next time a _skipped_ line, and from then on the schedule
  read _on_ and left the sidebar's count, while every time after went on being skipped. It is
  now read from the run's own line as well: a run that has not ended is said first, however
  many lines have been written since.
- **And could lose the run altogether.** A history keeps 200 lines, so a schedule that fires
  every minute pushed the line of a run waiting for approval out in a little over three hours.
  With the line went the way to the run: its ending was never copied or told, and its mark in
  run lists went too. The line of a run that has not ended is now kept whatever its age.

Checked by hand on 2026-11-05, in a development build on Linux with the throwaway profile and
demo projects `scripts/screenshots.sh` sets up, schedules added with the build's own `ys`:

- An every-minute schedule of a one-step workflow fired at each minute; each run succeeded, and
  its ending was copied onto the history.
- A `once` for a smoke test nobody had approved fired at its time; the run waited for approval,
  the schedule read _waiting for approval_, and the sidebar row counted it.
- A daily schedule set to skip, back-dated two hours in the database, was recorded as _missed_
  and moved to the next morning. An every-minute one, back-dated an hour, ran once.
- The view listed them, and the dialog showed the next times for a daily time and for a cron
  expression, the reason a four-field expression is refused, and a test's steps.

Not checked by hand: a notification arriving on screen. The throwaway profile cuts the app off
from the session bus, which is what delivers one on Linux; the history rows show each was asked
for, through the call a workflow's `notify` step already uses. macOS and Windows have had the
tests in CI only.

The view's shot (`docs/images/schedules.png`) is taken on the same throwaway profile.
`scripts/screenshots.sh seed` writes its five schedules and their history into the database, as
it does the smoke test runs, with times worked out from the moment it is run. They are real
rows, so an app left open fires them.

## Not in version one, on purpose

- ~~**A fresh worktree per run.** The thing recurring agent chores want: fetch, branch from the
  base, run there. It needs a rule for the worktrees that pile up — keep the last N, archive on
  success, never touch one with uncommitted changes — and that rule deserves its own decision.~~
  Built on 2026-11-13, with the rule the user chose: see
  [A new workspace per run](#a-new-workspace-per-run).
- **Running while Yardsort is closed.** Two ways there: the app stays in the background (a tray
  icon, launch at login), or the drivers move into the daemon. The first is smaller and probably
  first. Until then a missed time is handled, not prevented.
- **Triggers from the forge** — a pull request opened, a review posted. A different producer of
  queued runs, with its own questions. Proposed in [36](36-triggers.md).
- **Schedules on the phone.** Remote exposes no workflows, tests or runs yet; schedules would
  follow them, not lead.
- **Schedules in a file.** A schedule names a workspace on this machine, so it is the profile's.
- **Time zones other than the machine's.**

## A new workspace per run

_Built 2026-11-13, after [a run from a project](21-workflows.md#a-run-from-a-project) gave runs
a door that makes their workspace._ The question that had put this off was put to the user:
what becomes of the worktrees that pile up. Of three answers — keep all with a cap, keep all
with none, archive the old ones — the user chose the first.

- **Nothing is removed, and there is a cap.** A schedule that makes its workspaces has `keep`,
  5 unless said, 1 to 50. When that many of the workspaces it made are still there — not
  deleted, not archived, not forgotten — its time is **refused**, with their names, and the
  user is told once, as for any refusal. Archiving or deleting one is all it takes. No schedule
  archives, deletes or reuses a workspace, so the rule against destroying work silently has
  nothing to make an exception for.
- **What it made is its own table.** `schedule_workspaces (id, schedule_id, workspace_id)`,
  both references cascading. Not a column of `schedule_runs`: that history is pruned at 200
  lines, and a workspace kept after its setup failed has no run to hang on.
- **A slot is taken before the workspace is made.** Review found that counting first and
  writing the row after the setup had run let two runs asked for at once — two `ys schedule run`,
  or one and the clock — both find the last slot free. So a row is written first, with no
  workspace, by one statement that also counts (`reserve_schedule_workspace`); a slot being
  filled counts like a workspace that is there. It is given back when nothing was made. One
  held for an hour with no workspace was left by a process that died, and is dropped.
- **The workspace is the one this attempt made.** `Workspaces::create_named_noting` says which
  workspace it made the moment that has a row, before the setup runs, and the slot is filled
  then. So a workspace whose setup failed is counted, or a setup that fails every night would
  leave a worktree every night with no cap at all; and nothing else made in the project
  meanwhile is. The first version compared the project's workspaces before and after, and
  review showed it claiming a workspace the user made during the setup.
- **The schedule's own row.** `new_workspace`, `base_branch` (NULL follows the repository's
  default branch at each run), `keep`; `workspace_id` is NULL and `workspace_name` holds what
  the workspaces are called, empty for "after the workflow". `NoWorkspace`, and the switching
  off that goes with a deleted workspace, apply only to a schedule that names one.
- **Workflows only.** A smoke test is a file in a workspace. A workflow the door would refuse
  whatever its inputs — about a tracked pull request, or needing an open one at the start — is
  refused when the schedule is saved (`runs::refused_in_new_workspace`).
- **Decision 6 does not apply.** "One run per workspace" never skips a time here, since each
  run has its own. The cap is what keeps things from piling up.
- **Who waits for a person** is asked of the project's own checkout, the only place the smoke
  test file can be read before the workspace is there.
- **`fire::tick` and `run_now` take what makes workspaces** (`Workspaces`: git, the worktree
  root, the settings). Making one runs the project's setup on the schedules thread, so other
  schedules wait for it. The time was already claimed, so nothing fires twice; a slow setup
  delays the others and the telling of endings. Moving it off the thread is the fix if that
  hurts in use.
- **The sidebar** reads the projects again when schedules change, since a run in a made
  workspace may start no agent, and an agent starting was what told it before.

Not built: archiving on success, a cap by age, cleaning up from the schedule's page, triggers
and the phone.

## Open questions

1. **Is `run once` the right default for a missed time?** It is the useful one on a laptop that
   sleeps, and it is one run at most. `skip` is the cautious one. To be judged in use.
2. **Should a schedule be able to follow a branch rather than a workspace** — "whichever
   workspace has `main` checked out"? Not until someone needs it.
3. ~~**How loud should `refused` be?** A count in the sidebar is proposed. A notification each
   time would be noise for a schedule firing every ten minutes at an archived workspace; one
   notification the first time may be right.~~ Settled in review: the count, and one
   notification when a schedule starts being refused, not again until it has started a run or
   been skipped or missed in between. A schedule that tells nothing tells nothing.

## Documentation this touches

- A new guide, `docs/guide/schedules.md`, linked from `docs/README.md` and
  `website/src/lib/docs.ts`.
- `docs/guide/cli.md` (`ys schedule`), `docs/guide/workflows.md` and `docs/guide/smoke-tests.md`
  (where **Schedule…** is, and what a scheduled run of each does).
- `README.md` and the landing page's copy, together; `CHANGELOG.md`.
- [21](21-workflows.md) and [32](32-smoke-tests.md), which both list schedules as not built, and
  the [roadmap](05-roadmap.md)'s Automations item.
