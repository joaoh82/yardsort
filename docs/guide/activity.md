# Activity

Yardsort keeps a small, local record of what it itself saw happen in a workspace: when an agent,
a shell or the project's run command was started, whether that was a fresh conversation or one
being resumed or forked, and how the process ended. It is called **activity**, it lives in your
Yardsort database, and it never leaves your machine.

This is the first piece of a longer plan — see the design note on
[agent events](../design/09-agent-events-and-memory.md) — and it is deliberately modest. Nothing
here reads what an agent prints or does. The terminal is still the truth; activity is the
bookkeeping around it.

## What is recorded

One **run** per process Yardsort starts in a workspace, and a few **events** for each:

| Event                  | When                                                                                                                                                                                                                                                                          |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `process.started`      | The process is up. Says which agent (or `shell`, or the run command's program), the model if one was chosen, whether it was fresh, resumed or forked, and whether the app or `ys` started it.                                                                                 |
| `process.exited`       | The process ended: the exit code and, where the platform reports one, the signal. Says how Yardsort learned of it — as it happened, from the [background process's spool](#exits-while-yardsort-is-closed), or because the process was simply gone when Yardsort next looked. |
| `process.spawn_failed` | The program could not be started at all — usually "not found on PATH".                                                                                                                                                                                                        |
| `session.resumed`      | A recorded conversation was continued in a new process.                                                                                                                                                                                                                       |
| `session.forked`       | A copy of a conversation was started, and which one it came from.                                                                                                                                                                                                             |

What is **not** recorded, on purpose: your first message, the command line, anything the agent
printed, the files it touched, tool calls, tokens. Those would need the agent's own cooperation,
which a later stage may add, per agent and only if you turn it on. Until then the timeline says
so: _nothing from inside the agent is recorded yet_. Every event names its source
(`yardsort/lifecycle`) so that, when agent-reported events do arrive, you can tell "Yardsort saw
the process end" from "the agent says it edited a file".

Activity is on by default, because it is only what Yardsort already knew; switch it off in
[Settings → General](settings.md#general) and nothing new is written. What is already recorded
stays until you clear it.

## The timeline (experimental)

Turn on **Show the activity timeline** in Settings → General and every workspace gains an
**Activity** button at the left of its footer. It opens a list, newest first, of that
workspace's events, with **Show earlier** to page back, **Refresh**, **Clear** for that
workspace, and a note on what the list covers.

It is experimental: the words and the layout will change as later stages add more to show.

## Exits while Yardsort is closed

Agents [keep working when you close the window](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window).
Before activity existed, an agent that then finished had nobody to tell: the next time Yardsort
started, the conversation was listed as _interrupted_ even though it had ended cleanly.

Now the background process keeps every exit in a small **spool** — one file per exit, under
`activity/spool/` in your data directory — and whichever client connects next, the app or `ys`,
takes them into the database and empties it. The conversation is then shown as ended with its
real exit code and the time it actually ended, on the background process's clock — not the time
you next opened Yardsort — and the timeline says the exit came `via spool`. The spool is capped at 2 000
entries; past that, exits are dropped and counted, never allowed to fill a disk.

## What the program is told

A process started in a workspace is given three environment variables, so a script — or, later,
an agent hook — can say which run it belongs to:

| Variable                     | Value                                                   |
| ---------------------------- | ------------------------------------------------------- |
| `YARDSORT_RUN_ID`            | The run's id, when activity is being recorded.          |
| `YARDSORT_WORKSPACE_ID`      | The workspace's id.                                     |
| `YARDSORT_SESSION_RECORD_ID` | The conversation's id, for an agent. Absent for shells. |

Ids only; nothing else is added and nothing is taken out of your environment. (Project setup
scripts get a different pair, `YARDSORT_PROJECT` and `YARDSORT_WORKSPACE`, which are paths — see
[Project automation](projects.md#project-automation).)

## Keeping it small

Yardsort keeps the newest 20 000 events and nothing older than 90 days, pruning when it starts.
**Clear all recorded activity** in Settings → General forgets everything at once; the timeline's
**Clear** forgets one workspace. Deleting a workspace, or forgetting one without keeping its
history, takes its activity with it, exactly as it takes the conversations. Runs still going are
never cleared, so their exit can still be matched when it comes.

Settings → General also shows how much is recorded, where the spool is and whether anything is
waiting in it, and counters for anything that went wrong while recording — a write that failed,
a spool file that could not be read. Recording is never allowed to get in the way: if the
database cannot take a row, the agent starts anyway and the failure is counted here.

## From the command line

`ys activity list` prints the newest events (`--workspace` for one workspace, `--limit N`,
`--json`), and `ys activity export` writes every event as NDJSON — one JSON object per line, the
same fields the app sees — for whatever you want to do with it. `ys doctor` reports the counts
and the spool. See [The `ys` command line](cli.md#ys-activity-list).
