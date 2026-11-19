# Notifications

**Notifications**, the first row of the sidebar, is one list of two things: what is waiting for
you, and what finished while you were looking somewhere else. With agents, workflows and smoke
tests at work in several workspaces and projects, it is where you find out which of them to go
to.

The row carries two numbers. The filled one is how many things **need you**. The outlined one is
how many **finished and were not seen**. `Mod+I` opens the list from anywhere
([shortcuts](shortcuts.md)).

## What is in the list

The list is in the center panel, in three parts:

| Part                   | What is in it                                 | When a row leaves                |
| ---------------------- | --------------------------------------------- | -------------------------------- |
| **Needs you**          | Something that cannot go on without you       | When it is answered or put right |
| **Finished, not seen** | Something that ended while you were not there | When you look at it, or say so   |
| **Earlier**            | What was answered, put right or seen          | After a week                     |

What can be in **Needs you**:

- **An agent asks for a permission.** It leaves when the agent gets its answer, or is gone.
- **A workflow waits for your approval, or asks you a question.** It leaves when you decide or
  answer, here or from `ys` or the phone, or when the run ends.
- **A smoke test waits for you to approve its file.** It leaves when you let it run or refuse.
- **A schedule or a trigger cannot do its job.** Its time came, or it fired, and the run could
  not be started; its workspace was deleted; or a trigger has had all the runs its limit allows.
  It leaves when that is no longer so: its next run starts, you give it a workspace again, you
  reset a trigger's limit, or you switch it off or remove it.

What can be in **Finished, not seen**:

- **An agent** finished a stretch of work, its turn failed, or it stopped with an error.
- **A workflow run** succeeded or failed. One you ended yourself is not listed, because you
  know: a run you cancelled, and one that stopped because you rejected its approval or sent it
  back for changes.

Each row names the workspace, its project and its branch, then what it is about there: the
conversation, the workflow, the test, the schedule or the trigger. It says what happened and how
long ago, and gives one line of substance where there is one:

- the command an agent wants to run, while it waits for a yes;
- the start of what the agent said last, for one that finished;
- the exit code, for one that stopped with an error;
- the start of a workflow's question;
- why a workflow run failed, or why a schedule or a trigger could not start, while it still
  cannot.

A schedule or a trigger whose workspace is gone leads with its own name, and says which
workspace it was in.

## Other computers

The list also holds the rows of each [other computer this one reaches](remote.md#reaching-another-computer):
what waits there, and what finished there unseen. Each such row says **on** and that computer's
name, and they count in the numbers on the Notifications row. Grouped by project, a project
there is a group of its own, named with its computer.

- **The other computer keeps its own list.** This one reads it, and is told at once when it
  changes. A computer that is away shows no rows here, and has them again when it is back.
- **Seen here is seen there.** Mark a row seen, or look at its agent's terminal there from this
  window, and it moves to **Earlier** on that computer too, for its own window and your phone.
- **Pressing a row** opens its workspace on that computer, with the agent's terminal in front
  for a row about an agent. What a workflow, a smoke test, a schedule or a trigger there waits
  for is answered or changed on that computer: its row here tells you so, and takes you as far
  as the workspace.
- **A permission** is answered in the agent's terminal there, once this computer has control.

This needs Yardsort 0.36.0 or later on the other computer, and that computer letting this one
watch its terminals. A computer with no screen, running [`yardsort-host`](headless-host.md),
keeps a list too. It carries out no workflows, smoke tests, schedules or triggers, so its rows
are its agents'.

The dot is the one the sidebar and the tabs use
([status dots](terminals-and-sessions.md#status-dots)): solid amber needs you, ringed is finished
and not seen, red went badly, grey has left.

## On the phone

The [phone app](remote.md#the-phone-app) has the same list, called **Notifications**, over every
computer the phone is paired with. Each computer keeps its own list, the one its window shows;
the phone reads it while that computer can be reached, and is told when it changes.

- **The bell** in the corner of the account's screen and of a computer's screen opens it. The
  number on the bell is how many things need you, on all your computers together. What only
  finished is not counted there.
- **A computer's row** on the account's screen says what waits there: _Online · 2 need you, 1
  finished_.
- **A computer's screen marks its workspaces**: **needs you** on one where something waits for
  you, **finished** on one where something finished unseen. A workspace with neither keeps
  saying how many of its conversations are running. A folded project says what is inside it.
- **The list** has the same three parts. Each row names the workspace, its project and its
  computer, says what happened, and shows the command asked for, the question, the reason or the
  start of what the agent said. Rows are not grouped by project on the phone, and there are no
  filters.
- **Pressing a row** opens what the phone has for it. An agent's row opens its conversation, in
  Chat or its terminal. A workflow's question or approval opens the decision, and a run that
  ended opens the run. A smoke test's row opens its workspace, where
  [the card to answer it](remote.md#when-a-smoke-test-waits-for-you) is. A schedule's or a
  trigger's row opens its workspace and nothing more: those are changed at the computer. One
  that has no workspace opens nothing, and says so.
- **Seen is one fact, kept on the computer.** Opening a finished row, or looking at that agent's
  conversation or that run on the phone however you got there, moves the row to **Earlier** on
  the computer too. Looking at it at the computer moves it on the phone. There is no **Mark all
  seen** on the phone.
- **A computer that is offline shows a line and no rows**, and its count leaves the bell. Its
  rows are back when it is. Offline does not mean its agents stopped. While no computer can be
  reached, the list does not say that nothing needs you: it does not know.
- **A computer on an earlier Yardsort** keeps no list. The phone says so on the list, and shows
  that computer's workspaces without marks.

The list is separate from the notifications the phone's system shows. Those are sent through the
server and are chosen under Settings → Notifications on the phone, which the foot of the list
also opens ([Remote](remote.md#notifications)). The list is read from the computers, and nothing
of it passes through the push service.

**An agent that finished and went unseen is one of those notifications.** Within a minute
of an agent finishing, if its row is still under **Finished, not seen**, the phone is told:
_An agent finished_. It says nothing more; opening it opens that agent's conversation and
marks the row seen. An agent you were looking at when it finished, at the computer or on the
phone, sends nothing. Turn it off with **Agents that finish**, under Settings → Notifications
on the phone. It needs this version on the computer and on the server.

## Opening a row

Pressing a row closes the list and opens what the row is about:

| The row is about        | Pressing it opens                                                         |
| ----------------------- | ------------------------------------------------------------------------- |
| An agent                | Its workspace, with that agent's terminal in front                        |
| A workflow              | The run, in the workflow's view ([Workflows](workflows.md))               |
| A smoke test that waits | Its workspace, where the approval card is ([Smoke tests](smoke-tests.md)) |
| A schedule              | **Schedules** ([Schedules](schedules.md))                                 |
| A trigger               | **Triggers**, on that workspace's triggers ([Triggers](triggers.md))      |

There you answer, read what happened, or change what could not start.

**Looking is what marks a finished row seen.** An agent's row moves to **Earlier** when its
terminal is in front of you in a window that has the focus: because you opened the row, because
you went there some other way, or because you came back to Yardsort with that terminal showing.
Typing a message to the agent does it too. A workflow run's row moves when that run is the one
on screen under **Runs** in its workflow's view. An agent that finishes, or a run that ends,
while you are watching it never shows as unseen.

To clear a row without going there, point at it and press **Mark seen**. **Mark all seen** in the
header moves everything that finished to **Earlier**. A row under **Needs you** has no such
button: it leaves when what it waits for is done.

## Filters and grouping

The tabs in the header show one part at a time: **All**, **Needs you**, **Finished** or
**Earlier**.

**Group by** chooses how the rows are arranged, and Yardsort remembers the choice:

- **What it is** (the default): everything that needs you, then everything that finished.
- **Project**: the same rows under their projects, with what needs you first in each. Use it when
  the question is where, not what.

**Earlier** is always its own part, at the end.

## In the sidebar

A project's row shows how many of its workspaces' rows are still in the list, so a folded project
still says that something in it wants you. Pressing the number opens the list.

The ringed dot on a tab and on a workspace is drawn from this list, so it and the list always
agree. The list is kept in Yardsort's database: a mark you have not cleared is still there after
Yardsort is closed and opened again.

**Workflows**, **Tests**, **Schedules** and **Triggers** keep the numbers on their own rows. The
Notifications row adds them up with the agents', with one difference: a schedule or a trigger
whose run waits for a smoke test's approval counts once here, as the test.

## What Yardsort can know

Nothing here is read from what an agent prints. About an agent, Yardsort knows two kinds of
thing.

**From the terminal itself, for every agent.** A terminal that goes quiet after at least eight
seconds of printing has finished a stretch of work, and a process that exits with an error has
stopped. An agent you close yourself, or one that leaves normally, is not listed.

An agent that stopped printing while Yardsort was closed is listed when you open it again, at the
time it stopped. [Terminals keep running without the window](terminals-and-sessions.md), so this
is common.

**From the agent's own report, when you have turned that on.** In **Settings → General**, each
agent has a **Capture what it reports** switch, off until you turn it on
([Activity](activity.md)). With it on:

- an agent that **asks for a permission** is listed under **Needs you**. Claude Code, OpenCode,
  pi and OMP report that they ask; Codex, Grok and Cursor do not, and have no such row. Without
  the switch Yardsort cannot know that any agent is asking. An agent waiting for your answer has
  gone quiet too, and is not listed as finished for it: its question stays under **Needs you**
  until it is answered. Without the switch, or for an agent that does not report, a wait looks
  like a finished stretch of work, and is listed as one;
- a finished turn is known the moment the agent reports it, however short it was, and a row for
  a minute or more of work says how long the agent worked since your last message;
- the command it wants to run is shown for Claude Code. Other agents' rows say only that they
  ask.

What the agent said last is read from the agent's own conversation file when the list is shown.
An agent that keeps no such file has no such line. Neither that text nor the command is stored by
Yardsort.

**Workflows, smoke tests, schedules and triggers** are read from Yardsort's own records of
them, whoever changed them: the window, `ys` in a terminal, or the phone. They need no switch.
What already waits when Yardsort starts is listed. A workflow run that ended before this version
was installed is not.

**An agent a workflow started is the workflow's.** While its run is going, that agent finishing
a turn is not listed, and its tab gets no ring: the run reads the turn and goes on by itself.
The run's question, its approval and its end are what is listed. A permission that agent asks
for is still listed, and so is the agent if it stops with an error. Once the run is over, the
agent is listed like any other.

## What is not here yet

- **The phone is told that an agent finished, and of nothing else the list adds.** A turn that
  failed and an agent that stopped with an error are listed and not sent. A workflow's
  question, an agent's permission and a failed run already had notifications of their own.
- An agent that finished while Yardsort was closed is listed when it starts again, and is not
  sent to the phone.
- On the phone, the list shows a computer's newest hundred rows. One with more says so.
- A row of another computer does not ring a tab or a workspace in the sidebar's list of that
  computer. The Notifications row and the list are where it shows.
- **Pull requests and tasks** are not part of this list. They have their own views.
- Pressing a **system notification** does not open its row. The count on the Notifications row
  is where to look.
