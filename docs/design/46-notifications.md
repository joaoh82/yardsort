# 46. Notifications

Status: proposed in Figma and answered by the owner on 2026-11-17. Being built, a slice at a
time.

## The problem

Someone running several agents wants one answer to two questions: what is waiting for me, and
what finished while I was looking elsewhere. Today the answer is spread out, and part of it is
lost:

- An agent that finished shows as a ringed dot on its tab and its workspace row. That mark lives
  in the window's memory. Restarting the app loses it.
- A workflow's question, a smoke test waiting for a yes, and a schedule that could not start each
  have a count on their own row in the sidebar. Nothing adds them up.
- A system notification is sent for some of these. Once it is dismissed there is no list to go
  back to.
- The phone gets a push for some of this and keeps no list. Nothing tells it that an agent
  finished.

## What was decided

Proposed in Figma first: the page **Notifications** of the desktop's file (frames D1 to D3) and
the page **Notifications · proposal** of the phone's file (frames N1 to N4). The owner's answers,
on 2026-11-17:

1. **One list, in three parts.** _Needs you_, _Finished, not seen_, and _Earlier_.
2. **Grouped by what it is by default** (D1). Grouped by project (D2) ships too, and the choice
   is remembered.
3. **On the phone the list is called Notifications.** Settings → Notifications, which chooses
   what is pushed, keeps its place and its name.
4. **A push when an agent finishes, on from the start**, with a switch of its own.
5. **The desktop is built first, then the phone**, which reads the computer's list.
6. **A permission is answered in the agent's terminal.** Its row opens that terminal, where the
   agent's own question waits. **Allow** and **Deny** on the row come later, as a slice of their
   own: only Claude Code's hook can take an answer from outside its terminal today
   ([43](43-phone-chat.md)), so the buttons would be on some rows and not on others.
7. **Pull requests and tasks are not in the list.** They keep their own views and counts. The
   computer learns of them only when it fetches those lists, and every failed check would be a
   row among the agents' rows.

Proposed in the notes of both pages and not objected to:

- A schedule or a trigger that could not start is listed under _Needs you_.
- _Earlier_ is kept for a week.
- On the phone, the bell counts what needs you only, and a computer that is offline shows a line
  and no rows.

## What a notification is

One row is one thing that happened in a workspace. It is made by the core when the thing
happens, and it leaves its part of the list by itself.

| Kind                    | Made when                                                                   | Part     | Leaves when                                 |
| ----------------------- | --------------------------------------------------------------------------- | -------- | ------------------------------------------- |
| An agent asks           | Its hook reports `approval.requested`                                       | Needs    | The wait is answered, as the phone's notice |
| An agent finished       | Its turn ended: `turn.completed`, or a quiet terminal after a stretch of it | Finished | Its terminal is looked at, or Mark seen     |
| An agent stopped        | Its process ended with an error                                             | Finished | The same                                    |
| A workflow asks         | A question or an approval starts waiting                                    | Needs    | It is answered, or the run ends             |
| A workflow run ended    | It succeeded or failed, and no person ended it                              | Finished | The run is looked at, or Mark seen          |
| A smoke test waits      | A run is `awaiting_approval`                                                | Needs    | It is let run, or refused                   |
| A schedule or a trigger | It could not start, lost its workspace, or a trigger reached its limit      | Needs    | It can again, or is switched off or removed |

A row that has left its part is under _Earlier_ for a week, then deleted.

Each row says the workspace, its project and its branch, what happened, and one line of
substance. That line is read when the list is asked for and is never copied into the row:

- The command an agent wants to run, from its open question
  (`crates/core/src/activity/asks.rs`). Only Claude Code posts one. Another agent's row says
  only that it asks.
- A workflow's question, from the wait.
- Why a workflow run failed: the run's own error, or else the note of the step that failed.
- Why a schedule or a trigger could not start, from its newest time or fire, and only while it
  still cannot: once something has happened since, that line is about something else.
- What an agent said last, from its own session file, with the readers Chat uses
  ([29](29-conversation-context.md)). An agent that keeps no such file has no such line.

Nothing is read from a terminal. `AGENTS.md`'s _never parse agent output_ holds: a quiet
terminal says only that a turn ended.

## How it is kept

**The core keeps the list**, in one table of the store (`notifications`, migration `0046`;
`crates/core/src/notifications.rs`). The window reads it and is told when it changes. The phone
and another computer will read the same list over Remote.

- A row is about an agent's **conversation**, by its session record. Runs are recorded only
  when activity recording is on; a conversation always has a record.
- A row has a stable key, so the same event told twice is one row. A conversation has at most
  one row waiting for an answer and one finished row not seen: a second request before the
  first is answered adds nothing, and finishing again before anyone looked moves the unseen row
  to the later time.
- _Seen_ and _answered_ are one time on the row, `settled_at`. The ring on a tab and on a
  workspace row is drawn from the list (`ringOnly` in `src/stores/terminals.ts`), so the two
  agree and the ring survives a restart. Before, the ring was a flag in the window's memory,
  set again on every waiting agent at each start.
- "Looked at" is the window's to say, since only it knows what is on screen. The center panel
  tells the list which terminal it shows (`looking` in `src/stores/notifications.ts`); a
  finished row whose terminal is in front, in a window that has the focus, is marked seen. So is
  one made while its terminal is in front. Coming back to the window counts.
- One turn is one row. An agent's turn ends in up to two signals, the terminal's quiet after
  eight seconds of printing and its harness's own `turn.completed`; the fold triggers use
  (`crates/core/src/trigger/turns.rs`) makes them one.
- Hook events are read from `agent_events` from where the list last stopped, as triggers read
  them. The first read starts at the newest event: what happened before there was a list is not
  news.
- A message typed to the agent (`prompt.submitted`) marks what it finished seen: whoever typed
  it saw where it stood.
- An exit with an error is a row. An exit by a signal, an exit with `0`, and the exit of a
  terminal the window was asked to close are not: on Windows a closed terminal ends with a
  failing code and no signal.
- **Caught up at start.** The app writes down, every few seconds, that it is hearing from the
  terminals. At start, an agent's terminal that is alive and quiet, and last printed after that
  time, stopped printing while the app was closed: it gets a finished row at the time it
  stopped, unless its harness reported the turn itself.

### What else waits: read, not told

A workflow's wait, a smoke test's run, a schedule and a trigger are not told to the list as they
happen. The app and `ys` are two processes that both open a wait, queue a test, approve, answer
and refuse; what both leave is the store. So the core reads where each stands and makes the list
agree (`standing::read` in `crates/core/src/notifications/standing.rs`):

- Everything that waits on a person now is worked out from the rules those features already
  have: `workflow::waits::list_open`, a test run that is `awaiting_approval`,
  `schedule::standing` and `trigger::standing_now`. A row is added for what is not listed, and
  a listed row whose thing waits no more leaves, as answered. Reading twice changes nothing.
- What waits at the first read is listed: it needs its person now, whenever it began.
- A workflow run that succeeded or failed since the last read gets a finished row. The reader
  keeps the newest ending it has taken in, among the event readers' cursors; the first read
  starts from now.
- The app reads beside each word it sends the window that runs, tests, schedules or triggers
  changed, and whenever the list is asked for. Removing a workspace or a project has the window
  read the list again.

Decided while building it, none of it in the Figma frames:

- **A trigger held by its limit is listed** with the refused ones. It is counted on the Triggers
  row as needing attention, and nothing fires for it until someone resets it.
- **A schedule or a trigger whose run waits for a smoke test's approval is not a row of its
  own.** The test's run is, and it is the thing to answer.
- **A run a person ended is not news to them.** A cancelled run is not listed. Neither is one
  that failed because its approval was rejected or sent back for changes: the engine ends such
  a run as `failed`, and the decision on the failed step is what tells it apart.
- **A failed run that is continued** takes its finished row with it, and a second ending is a
  second row.
- **An agent a running workflow started is the workflow's.** Its turns ending are not finished
  rows and its tab gets no ring, while the run goes: the run reads the turn and moves on. A
  `start_session` step's `session` output is how the agent is known, as triggers know it
  (`started_by_workflow`). A permission it asks for is still a row, and so is its stopping with
  an error. An agent a run only writes to (`session: origin`) is the person's own, and is
  listed as ever.

### What needs the agent's own report

Hooks are off until the person turns an agent's **Capture what it reports** switch on, in
Settings → General. Without it the core hears only the terminal: it knows a stretch of work ended and
that a process stopped. It cannot know that an agent is asking for a permission, so there is no
such row. The guide says so.

## The slices

Each comes with its tests, its guide and changelog lines, and a smoke test.

1. **The list, for agents on this computer. Built on 2026-11-17.** The table and the core's
   rules for an agent that asks, finishes or stops. The Notifications row in the sidebar with
   its two counts, the view with its three parts, the filters, both groupings, Mark seen and
   Mark all seen, a count on each project row, and Mod+I. The ring on tabs and rows drawn from
   the list. A new guide, `docs/guide/notifications.md`.
2. **Everything else that waits on this computer. Built on 2026-11-17.** Workflows that ask,
   runs that end, smoke tests that wait, and schedules and triggers that could not start.
3. **Other computers on the desktop.** A new protocol minor: the list, word when it changes, and
   marking a row seen. Their rows join the list under the computer's name.
4. **The phone.** The bell and its count, the list, the counts on a computer's row and the marks
   on its workspaces, from the same protocol minor.
5. **The push for an agent that finished.** A new kind of notice on the server, and its switch
   under Settings → Notifications ([40](40-mobile-notifications.md)).

Later, if wanted: **Allow** and **Deny** on a permission's row.

### Slice 1, as built and as checked

Two things differ from the Figma frames. **Group by** has a row of its own under the header:
with the changes panel open the header did not fit on one line. And a row names its
conversation after the branch, since two agents in one workspace otherwise read the same.

Checked by hand on 2026-11-17, in a development build on a throwaway profile with a stand-in
agent that prints for ten or forty seconds and then waits:

- The agent finishes in a workspace that is not in front: a row under _Finished, not seen_, the
  row's count in the sidebar, the project's count, and the ring on the workspace.
- Pressing the row opens the workspace with the agent's tab, and with the window focused the
  row moves to _Earlier_ and the ring goes.
- The agent exits with `3`: a red row with `exit 3`. An agent whose terminal is closed from the
  window, which ends with `1`: no row.
- The app is restarted while the agent works: the row comes when it finishes. The app is
  stopped while the agent works and started after it finished: the row is there at start, at
  the time the agent stopped printing.
- A permission request and then the tool's result, written to the inbox as a hook writes them:
  a row under _Needs you_ and the filled count, then gone.
- Grouping by project, and the choice kept in the profile.

Not done: a smoke test file for this slice, the command and the last words with a real Claude
Code (the core's test reads a recorded conversation), macOS and Windows beyond CI, and the
screenshots, which all show the sidebar without its new first row.

### Slice 2, as built and as checked

The table was given its final shape before slice 1's commit, since it had not shipped: a row
may have no workspace (a schedule or a trigger that lost its own), and names what else it is
about (`subject_id`, `step_id`).

A row leads with its workspace and then names the workflow, the test, the schedule or the
trigger. One with no workspace leads with its own name and says which workspace it was in.
Pressing a row opens the run, the workspace with the approval card, Schedules, or Triggers on
that workspace's triggers. A run's row is seen when that run is on screen under **Runs**, in a
window that has the focus.

Checked by hand on 2026-11-17, in a development build on a throwaway profile seeded with the
demo projects, off-screen:

- At start the seeded smoke test waiting for approval and the seeded trigger held by its limit
  are listed, with the trigger's reason. The seeded schedule whose run waits for that test adds
  no row.
- The trigger's row opens Triggers on its workspace; **Reset** there moves the row to Earlier.
  The test's row opens its workspace; **Don't run** on the card moves the row to Earlier.
- A workflow with an approval step, started with `ys workflow run`: a row with its question;
  the row opens the run; **Approve** moves it to Earlier and the run's ending is listed. With
  the window not in front the ending stays unseen; with the focus back and the run on screen it
  is seen.
- **Reject** on the same step: the approval's row leaves and the run's ending is not listed.
  Before the rule for it, it was listed as a failure.
- A run that failed because its agent is not configured: a red row with the step's reason, and
  **Mark seen**.
- A workflow that starts a stand-in agent, waits for it to settle and then for an approval: the
  agent's turn ending is no row; the approval is. Typed to after the run ended, the same agent
  finishing is a row.
- The demo workspace deleted through the app's own command: its rows go, and its three
  schedules and three triggers are listed at once as having no workspace, under their own names.
- Grouping by project, and **Mark all seen**.

Not done: a smoke test file for this slice, macOS and Windows beyond CI, and the screenshots.
Not tried by hand: a workflow's question (the core's and the window's tests cover it), a
schedule refused at its time (the core's test), and an answer given from the phone.

## Limits, said plainly

- **A system notification cannot open its row.** The notification plugin the app uses does not
  report a click on the desktop. The count on the Notifications row is where to look.
- An agent that finished within a few seconds of the app closing may not be caught up with:
  the app's last word that it was listening is up to five seconds old.
- A computer with no window (`yardsort-host`) does not keep the list yet. It runs the same
  core, and is wired when other computers' rows are (slice 3).
- The phone's list covers the computers it can reach. One that is offline shows no rows.
- A computer on an earlier release sends no list.
