# Notifications

**Notifications**, the first row of the sidebar, is one list of two things: what is waiting for
you, and what finished while you were looking somewhere else. With agents at work in several
workspaces and projects, it is where you find out which of them to go to.

The row carries two numbers. The filled one is how many things **need you**. The outlined one is
how many **finished and were not seen**. `Mod+I` opens the list from anywhere
([shortcuts](shortcuts.md)).

## What is in the list

The list is in the center panel, in three parts:

| Part                   | What is in it                                                       | When a row leaves                         |
| ---------------------- | ------------------------------------------------------------------- | ----------------------------------------- |
| **Needs you**          | An agent asked for a permission                                     | When it is answered, or the agent is gone |
| **Finished, not seen** | An agent finished a stretch of work, its turn failed, or it stopped | When you look at its terminal, or say so  |
| **Earlier**            | What was answered or seen                                           | After a week                              |

Each row names the workspace, its project, its branch and the conversation, says what happened
and how long ago, and gives one line of substance where there is one:

- the command an agent wants to run, while it waits for a yes;
- the start of what the agent said last, for one that finished;
- the exit code, for one that stopped with an error.

The dot is the one the sidebar and the tabs use
([status dots](terminals-and-sessions.md#status-dots)): solid amber needs you, ringed is finished
and not seen, red went badly, grey has left.

## Opening a row

Pressing a row opens its workspace with that agent's terminal in front, and closes the list.
There you answer the agent's question, read what it did, or go on with it.

**Looking at the terminal is what marks a finished row seen.** It moves to **Earlier** when its
terminal is in front of you in a window that has the focus: because you opened the row, because
you went there some other way, or because you came back to Yardsort with that terminal showing.
Typing a message to the agent does it too. An agent that finishes while you are watching it
never shows as unseen.

To clear a row without going there, point at it and press **Mark seen**. **Mark all seen** in the
header moves everything that finished to **Earlier**. A row under **Needs you** has no such
button: it leaves when the agent gets its answer, in its terminal.

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

## What Yardsort can know

Nothing here is read from what an agent prints. Yardsort knows two kinds of thing.

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
  the switch Yardsort cannot know that any agent is asking;
- a finished turn is known the moment the agent reports it, however short it was, and a row for
  a minute or more of work says how long the agent worked since your last message;
- the command it wants to run is shown for Claude Code. Other agents' rows say only that they
  ask.

What the agent said last is read from the agent's own conversation file when the list is shown.
An agent that keeps no such file has no such line. Neither that text nor the command is stored by
Yardsort.

## What is not here yet

- **Workflows, smoke tests, schedules and triggers** that wait for you keep their own counts in
  the sidebar for now. They are coming to this list.
- **Other computers'** agents and **the phone** are coming too.
- **Pull requests and tasks** are not part of this list. They have their own views.
- Pressing a **system notification** does not open its row. The count on the Notifications row
  is where to look.
