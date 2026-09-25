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

What is **not** recorded here, on purpose: your first message, the command line, anything the
agent printed, the files it touched, tool calls, tokens. Those need the agent's own cooperation,
which Yardsort asks for per agent and only when you turn it on — today for
[Claude Code](#what-claude-code-reports), nothing else. Every event names its source
(`yardsort/lifecycle`, `claude/hook`) so you can tell "Yardsort saw the process end" from "the
agent says it wrote a file".

Activity is on by default, because it is only what Yardsort already knew; switch it off in
[Settings → General](settings.md#general) and nothing new is written. What is already recorded
stays until you clear it.

## The timeline (experimental)

Turn on **Show the activity timeline** in Settings → General and every workspace gains an
**Activity** button at the left of its footer. It opens a list, newest first, of that
workspace's events, with **Show earlier** to page back, **Refresh**, **Clear** for that
workspace, and a note on what the list covers.

It is experimental: the words and the layout will change as later stages add more to show.
While Claude Code is reporting, the list moves as the agent works; you do not have to press
**Refresh**.

## What Claude Code reports

Switch on **Capture what Claude Code reports** in Settings → General (it needs **Record when
agents start and exit**, and is off by default) and every Claude Code that Yardsort starts —
from the composer, the tab bar, Resume, Fork or `ys` — reports what it does, in its own words:

| Timeline row                                                                     | What Claude Code said                                                                                                      |
| -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| _agent session started_, _agent resumed its session_, _agent forked its session_ | Its session began, and how. On a resume, how many tokens of context it picked up.                                          |
| _prompt submitted · 147 characters_                                              | You sent a message. The length, never the text.                                                                            |
| _Edit started_, _Edit done · src/app.rs · 12 ms_                                 | A tool ran: its name, the file's path relative to the workspace (or _a file outside the workspace_), and how long it took. |
| _Bash started_, _Bash done_                                                      | A command ran. The command itself is not recorded.                                                                         |
| _Agent (Explore) started_                                                        | It started a subagent, of that type.                                                                                       |
| _Read failed_, _Bash denied_, _permission asked for Bash_                        | A tool failed, was refused, or is waiting for your yes. No error text, no command.                                         |
| _agent raised a notification · permission_prompt_                                | It wants you: a permission prompt, or it has been idle waiting for input.                                                  |
| _agent finished its turn_ / _agent's turn failed · rate_limit_                   | The turn ended, or ended in an error of that kind.                                                                         |
| _agent compacted its context_, _agent switched model_                            | Housekeeping it did.                                                                                                       |
| _agent session ended_                                                            | It is shutting down, and why.                                                                                              |

The run's own _claude started_ row says _reporting through hooks_ when this was on for it, so a
quiet timeline means the agent had nothing to say, not that nobody was listening.

**How it works, and what it touches.** Claude Code has _hooks_: commands it runs at points in its
own life, handing them a description of the moment. Yardsort gives its own launches of Claude
Code one extra settings file (`claude --settings <file>`, kept under `activity/hooks/` in your
data directory) whose hooks run the Yardsort executable itself, as an argument list, never
through a shell. Claude Code merges that file with your settings, so **your own hooks and
settings are untouched** — nothing is written to `~/.claude`, and a launch made while the switch
is off has none of ours. The hook keeps the moment's metadata, drops the rest before anything
reaches disk, leaves one small file in `activity/inbox/`, and exits — with a success code
whatever happened, because a hook that fails can make Claude Code stop and ask, and Yardsort
only watches. The app takes the inbox in as files land, on start, and on every exit; `ys` takes
it in before it lists anything. It works while the window is closed, exactly like the exit
spool.

**What is kept, and what is not.** Names, ids, relative paths, durations, counts, kinds. Not the
prompt, not a command, not a tool's input or output, not what Claude wrote back, not a path
outside the workspace. The recorded shapes come from Claude Code **2.1.280**; a newer version
that renames a field loses that detail, not the event. Claude Code started by any other means —
from your own terminal, say — reports nothing to Yardsort. A Claude Code harness whose arguments
already carry `--settings` or `--bare` is left alone, and the timeline's settings show a
`hooks_not_armed` counter so you know.

Other agents stay at what Yardsort itself sees. Which ones could report, and how, is in the
[design note](../design/11-agent-events-stage-2-claude.md#4--coverage-honestly).

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
The agents' inbox holds at most 10 000 reports waiting to be taken in; past that, reports are
dropped and counted.
**Clear all recorded activity** in Settings → General forgets everything at once; the timeline's
**Clear** forgets one workspace. Deleting a workspace, or forgetting one without keeping its
history, takes its activity with it, exactly as it takes the conversations. Runs still going are
never cleared, so their exit can still be matched when it comes.

Settings → General also shows how much is recorded, where the spool and the inbox are and whether
anything is waiting in them, where the Claude Code hooks file is, and counters for anything that
went wrong while recording — a write that failed, a spool file that could not be read, a report
that named a run this database does not have (`inbox_unlinked`: it is dropped rather than
guessed at). Recording is never allowed to get in the way: if the database cannot take a row,
the agent starts anyway and the failure is counted here.

## From the command line

`ys activity list` prints the newest events (`--workspace` for one workspace, `--limit N`,
`--json`), and `ys activity export` writes every event as NDJSON — one JSON object per line, the
same fields the app sees — for whatever you want to do with it. `ys doctor` reports the counts,
the spool and the inbox. See [The `ys` command line](cli.md#ys-activity-list).
