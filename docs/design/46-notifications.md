# 46. Notifications

Status: proposed in Figma and answered by the owner on 2026-11-17. Built: all five slices.

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
`crates/core/src/notifications.rs`). The window reads it and is told when it changes. Another
computer reads the same list over Remote, and the phone will.

**Whatever runs the terminals keeps it**, with a `Keeper` of the core: the app, and a host with
no window. The keeper is told what only that process hears (a terminal going quiet, an agent's
process ending, the activity inbox drained), holds the fold of turns and the lines rows are
shown with, and says when the list changed. It began in the app and moved into the core with
slice 3, so `yardsort-host` keeps a list as the app does.

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
3. **Other computers on the desktop. Built on 2026-11-17.** Protocol minor 15: the list, word
   when it changes, and marking a row seen. Their rows join the list, each naming its computer.
4. **The phone. Built on 2026-11-17.** The bell and its count, the list, the counts on a
   computer's row and the marks on its workspaces, from the same protocol minor.
5. **The push for an agent that finished. Built on 2026-11-17.** A new kind of notice on the
   server, and its switch under Settings → Notifications ([40](40-mobile-notifications.md)).

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

### Slice 3, as built and as checked

**The protocol, minor 15** (`remote_proto::notification`), carried by the server unread. A host
that keeps a list says so in its capabilities.

- `notification.list`: the rows the grant is shown, what still waits first, then the newest of
  what left, with how many need their person and how many finished unseen over all of them. A
  reply holds at most 100 rows and what one frame carries, and says when it was cut.
- `notification.seen`: finished rows by their ids, or all of them. A row that waits is not
  marked. An id the grant is not shown is passed over without a word, as one that is gone.
- `notification.follow` and `notification.unfollow`: from then on the host sends
  `notifications.changed` on that connection whenever its list is not what the device was last
  told of. The event carries nothing: the device reads the list again.

**All four need `terminal:read`.** A row carries the start of what an agent said and the
command it asks to run, which is what watching its terminal shows. Rows are given only for the
projects the grant reaches. What a workflow asks, and why its run failed, is said only to a
grant that also has `workflow:read`. Nothing in this minor answers what waits: a permission, a
workflow and a smoke test each have their own operation, under their own scope.

**Following is one remembered state of the list per channel.** The channel's poll, which already
looks for new activity each second, compares how the list stands (`notifications::stamp`: how
many rows, how many settled, the newest of each) with what the device was told of. On a host
with no window nothing else looks at workflows, tests, schedules and triggers, so the poll has
the keeper look first, at most every five seconds.

**The desktop as a client** (`remote_host::computer::notifications`). A host's list is read
with everything else it shows, kept in memory with it, and read again when the host says it
changed, as well as when its activity moves and every 20 seconds. A host that is away shows no
rows. Marking a row seen is said to its host, and shown here at once; the host's next word puts
right anything it did not take.

**The window.** The list the window asks for is this computer's rows and then each reachable
host's (`notification_list`). A row from another computer carries its host's id and name; its
own id is that host's, so a row is told apart by both. It shows **on** and the computer's name,
groups under `project on computer` when grouped by project, and counts on the Notifications
row. It does not ring a tab or count on a local project's row.

- **Pressing it** opens its workspace there (`selectRemote`), with the agent's terminal in front
  for a row about an agent. A workflow, a smoke test, a schedule or a trigger there has no view
  on this computer ([44](44-other-computers-like-local.md) leaves them out), so its row goes as
  far as the workspace, and one with no workspace opens nothing and says where it is changed.
- **Seen** follows the rule for local rows: its agent's terminal there in front, in a window
  that has the focus (`lookingThere`, told by `RemoteWorkspaceView`), or **Mark seen**, or
  **Mark all seen**, which is also said to every host that can be told.

**Its tests.** The wire and its bounds in `remote-proto`, pinned by five fixtures that the
TypeScript client reads too; the host against a real store (what a grant reaches, a device that
may not watch, seen that touches only what the grant is shown, following, and a workflow's
question kept from a grant without `workflow:read`) in
`crates/core/src/remote/serve/tests/notification.rs`; two computers through the real relay
(read, told of a change well before the next reading, seen there, all seen, away) in
`crates/server/tests/computer.rs`; the TypeScript client's calls in
`packages/remote-client/test/channel.test.ts`; the window's store and view.

**Its smoke test.** `notifications-there.yaml`, new, with the window steps `lists-there`,
`marks-seen-there` and `opens-there-from-the-list`, and `support/work.sh`, an agent that prints
for ten seconds and then waits.

**Its steps run by hand on 2026-11-17**, on Linux: a `yardsort-host` on the stand-in server,
and the real window on a profile of its own, kept on a monitor nobody sees. The file itself was
checked with `ys test validate`; it has not been run through `ys test run`, which needs its
owner's approval first.

- The host listed its agent that had finished before anything was paired with it.
- Signed in as a client and let watch at the host's terminal, the window's Notifications row
  counted the host's row, and the list showed it with **on Smoke box**.
- A second agent started on the host showed in the window fourteen seconds after it was
  started, ten of them its work.
- **Mark seen** on a row in the window settled it in the host's own records, and only it.
- The other row opened its workspace on the host. With the focus given to the window, that
  agent's terminal there in front settled its row on the host.
- With the host stopped, the window showed it offline and no rows of it. An agent that finished
  on the host meanwhile was listed by the host when it was started again, at the time it had
  stopped printing, and the window had its rows again once it reached it.

What the run found: nothing in the app. The first `marks-seen-there` looked for a row by the
start of its text, which is the agent's letter, and now finds it by the button that opens it.

Not run: macOS and Windows, a second real computer, another desktop app as the host (the relay
test and the core's tests cover a host that keeps workflows' rows), and a host on an earlier
Yardsort beside this one.

### Slice 4, as built and as checked

Nothing new on the wire: the phone speaks minor 15 as the desktop does, through the TypeScript
client's `notifications`, `notificationsSeen` and `followNotifications`.

**One reading of every computer's list** (`apps/mobile/src/lib/notifications.ts`, `Lists`). It is
kept in memory for the profile in use, and never on the phone's disk. A screen hands it the
channels it holds; a computer's list is read when its channel opens and again when the computer
says it changed (or every 20 seconds from one that does not say). A computer that is offline,
refuses the phone or is gone loses its rows at once; one between two channels keeps them, so a
dropped connection does not empty the screen. A computer that does not answer
`notification.list` is not asked, and the list says it runs an earlier Yardsort.

**Channels.** The account's screen already holds one to each computer, and reads the lists
through them. The list's own screen holds one to each. A computer's screen holds its own and,
for the bell's count to be of every computer, one to each of the others: with one computer,
which is the common case, that is nothing more.

**Seen.** Pressing a finished row says so to its computer and opens what it is about. So does
looking: the conversation's screen and the run's screen mark what finished in what they show,
when they hold a channel, which they do only while in front. A row moves to Earlier on the
phone at once. A reading asked for before the computer took the word can still say
_finished_; the phone keeps the row under Earlier until the computer agrees, or the word fails.
Without that, opening a conversation said seen twice, and the row flickered back.

**What a row opens** is what the phone already had: `/terminal` by the conversation's record
for an agent, `/decision` for a workflow that waits, `/workflow` for a run that ended, and the
workspace for the rest. A smoke test's card was only in Chat, so a test asked for outside any
conversation had nowhere to be answered; the workspace's screen now shows the card too.

**Differences from the Figma frames**, all in words the list cannot back:

- An answered permission says _asked for a permission_, not _and you allowed it_: the list does
  not know how it was answered. Its command is not shown once answered, since the computer
  keeps a command only while its question is open.
- A smoke test's row says it _waits for you to let it run_, not that it _has changed_: a new
  file waits too.
- With no computer reached, **Nothing needs you** is not said.
- A computer on an earlier release gets a line of its own.

Not in the frames and not built: **Mark all seen**, grouping by project, and filters.

**Its tests.** The reading, the three parts and every word against a hand-made channel
(`test/lib/notifications.test.ts`); the same reading against the real server, relay and host
(`test/lib/notification-list.test.ts`), for which the stand-in host now keeps a list and has two
commands, `agentFinished` and `agentWaits`; and the screens through the real router and the real
protocol client against the fake server (`test/screens/notifications.test.tsx`): the bell and
the row's counts, the three parts, seen on opening and on looking, a change told, an offline
computer, two computers, the marks, and the test's card.

**Run by hand on 2026-11-17**, on Linux: the development build on a headless Android emulator,
with this branch's code served to it, signed in to the stand-in server and paired with its host.

- The account's screen: the bell with **1**, and the computer's row with _Online · 1 needs you,
  1 finished_.
- The list: a row under each heading, each with its workspace, _demo · Stand-in host_, its
  words and how long ago.
- Pressing the finished row opened that agent's terminal. Back in the list, read anew from the
  host, the row was under **Earlier**.
- With the list on screen, the agent finishing again showed a new row within three seconds,
  with nothing pressed.
- The computer's screen: **finished** on the workspace, then **needs you** and the bell's count
  within three seconds of the agent asking.
- With the host offline: the computer's row said so, the bell lost its count, and the list
  showed the line and no rows. Within twenty seconds of its coming back, the rows were back.

What the run found, both put right: the count was cut off by Android's header, which clips what
a header button draws outside its own bounds; and the list said **Nothing needs you** while its
only computer was offline.

**Its smoke test.** `remote-phone-sees-notifications.yaml`, new, with the phone step
`sees-notifications` and the window step `seen-from-the-phone`. It is the half the emulator run
did not cover: the real desktop app as the computer. A pretend phone, which is the TypeScript
client the phone app uses, follows the app's list and starts an agent that prints for ten
seconds and then waits. It is told that the list changed, reads the agent's row as finished
and not seen, and marks it seen; the real window then has nothing unseen and the row under
**Earlier**.

Its steps were run by hand on 2026-11-17, on Linux, against a fresh debug build on a profile
of its own, with the window on a monitor nobody sees, and all passed: the phone was told
fourteen seconds after it started the agent, ten of them the agent's work. The file was
checked with `ys test validate`. It has not been run through `ys test run`, which needs its
owner's approval first.

Not run: an iPhone, where the header is the system's glass; a real computer with a workflow, a
smoke test or a schedule waiting (the screens' tests cover their rows against the fake server);
two real computers; the light colours on a device.

### Slice 5, as built and as checked

The whole of it is in [40](40-mobile-notifications.md#an-agent-that-finished). In short: the
`agent` notice has a second kind, `finished`; the computer queues one for a row of the list
that it found unseen twice running, ten seconds or more apart; the server has a switch for it,
`agentFinished`, on from the start; and the phone shows that switch as **Agents that finish**.

Decided while building it, none of it asked of the owner:

- **A kind of the `agent` notice, not a category of its own.** A phone app from before it then
  opens the notice as it opens an agent's, by its terminal. As a new category that app would
  have shown the banner and failed to open it.
- **Only what went unseen, so it arrives late, within a minute.** The alternative, a push
  the moment an agent finishes, tells someone sitting in front of that agent. The computer
  looks every 20 seconds, so the wait is between 20 and 40 seconds after the row is made.
- **Only an agent that finished.** A turn that failed and an agent that stopped with an error
  are listed and not sent: the words would be wrong for them, and nobody asked for those.
- **Not what is older than ten minutes when first found**: an agent that finished while
  Yardsort was closed, or before the computer was signed in to a server.
- **Each finish is told of**, including a second one before anyone looked at the first.

**Found on the way.** On PostgreSQL, a notice already waiting for a phone that could not be
reached was still sent after **Agents that need you** was turned off: the query that clears a
queue named five categories and not `agent`. It now covers every kind there is, from one list
(`remote_proto::notify::NOTICES`), and both stores are tested for it.

**Its tests.** The kinds and the two switches in `remote-proto`; the queueing against a real
store, with a row seen in time, one too old, one told once and a second finish told again
(`crates/core/src/notifications/tests.rs`); the server on both of its stores, PostgreSQL
included: the words, the switch by itself, an older phone's save, and what a switch turned off
drops (`crates/server/tests/notifications.rs`); the client library's reading of both; the
phone's settings screen with a server that knows the switch and one that does not.

**Its smoke test.** `remote-push-finished.yaml`, new, with the phone step `looks-in-time`: a
`yardsort-host` on the stand-in server, whose push provider records what it would have sent.
Its steps run by hand on 2026-11-17, on Linux, with no window:

- An agent that worked for ten seconds and then waited, with nobody looking: one push, 52
  seconds after it was started, saying _An agent finished_ and nothing else. Its notice named
  the terminal of that agent on that computer.
- A second agent, marked seen by the phone 96 ms after it finished: no push in the minute
  after.

The file was checked with `ys test validate`. It has not been run through `ys test run`.

Not run: a real phone receiving it from Apple's or Google's service, the real desktop window as
the computer (the core's tests and the headless host share the code), and macOS and Windows.

## Limits, said plainly

- **A system notification cannot open its row.** The notification plugin the app uses does not
  report a click on the desktop. The count on the Notifications row is where to look.
- An agent that finished within a few seconds of the app closing may not be caught up with:
  the app's last word that it was listening is up to five seconds old.
- A computer with no window (`yardsort-host`) keeps a list of its agents. It carries out no
  workflows, smoke tests, schedules or triggers, so it has no rows of those.
- A row of another computer that needs an answer cannot be answered from the list, and for a
  workflow, a test, a schedule or a trigger there, not from this computer at all.
- This computer asks another for `workspace:read`, `activity:read`, `memory:read`,
  `terminal:read` and `terminal:control`, not for `workflow:read`: a workflow's row from another
  computer says that it waits and names the workflow, without its question.
- The phone's list covers the computers it can reach. One that is offline shows no rows.
- The phone shows a computer's newest hundred rows, and has no **Mark all seen**.
- The phone is told that an agent finished within a minute of it, and only if nobody
  looked. It is not told of a turn that failed or an agent that stopped with an error.
- A computer on an earlier release sends no list, and one that lets this computer see its
  workspaces but not watch its terminals gives none.
