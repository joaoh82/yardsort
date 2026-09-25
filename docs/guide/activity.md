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
[Claude Code](#what-claude-code-reports), [Codex](#what-codex-reports),
[OpenCode](#what-opencode-reports) and [Grok](#what-grok-records). Every event names its source
(`yardsort/lifecycle`, `claude/hook`, `codex/session_file`, `opencode/plugin`,
`grok/session_file`) so you can tell "Yardsort saw the process end" from "the agent says it
wrote a file".

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

## What Codex reports

Switch on **Capture what Codex reports** in Settings → General (also needs **Record when agents
start and exit**, also off by default) and every Codex that Yardsort starts reports each of its
turns. Codex works differently from Claude Code, so the mechanism does too: Codex is given, for
that launch only, a `notify` program — the Yardsort executable — that it runs at the end of every
turn with the turn's ids. Yardsort then reads what the turn did from **Codex's own session file**,
the one Codex keeps under `~/.codex/sessions/` and that `codex resume` reads. Rows appear when a
turn ends, not while it runs:

| Timeline row                                          | What it comes from                                                                                                              |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| _agent session started_                               | The session file's header: which Codex version, and that it was started from the CLI.                                           |
| _prompt submitted · 214 characters_                   | Your message. The length, never the text.                                                                                       |
| _shell done · exit 0 · 3 ms_, _shell failed · exit 1_ | Each command Codex ran, with its exit code and how long it took. The command itself is not recorded.                            |
| _file added · hello.txt_, _file changed · …_          | Each file Codex wrote through its patch tool: the path relative to the workspace, and whether it was added, changed or deleted. |
| _mcp:server/tool done_                                | A tool call to one of your MCP servers.                                                                                         |
| _tokens used · 29,842 total · 138 out_                | What the turn cost. Codex records this; Claude Code's hooks do not.                                                             |
| _agent finished its turn · 11.0 s_                    | The turn ended, and how long it took.                                                                                           |

Why not Codex's hooks, which look just like Claude Code's? Because Codex, rightly, refuses to run
a hook until you have reviewed it in its own **/hooks** screen, and the only way past that is a
flag that also waives review for every other hook, including a cloned repository's. Yardsort will
not pass that on your behalf. `notify` needs no review, is run as an argument list rather than
through a shell, and touches nothing in `~/.codex`. If your `config.toml` has a `notify` of your
own, Yardsort's runs first and then yours, with the same argument, so nothing you set up stops
working.

**What is kept, and what is not.** Ids, exit codes, durations, counts, relative paths, kinds of
change. Not a command, not a file's contents, not your message, not Codex's answer, not its
reasoning, not a path outside the workspace. A turn that `notify` reports before Codex has finished
writing it to the session file is picked up on a later look — the app looks again every few
seconds while a turn is waiting, `ys` on its next command — for up to five minutes; after that,
or if the file cannot be read at all, the timeline still shows _agent finished its turn_, marked
as coming from `notify` alone, and Settings → General counts what was missed
(`codex_turn_incomplete`, `codex_session_file`). If Codex's automatic reviewer is on, its review runs as a second thread, and its turns show as
_agent finished its turn_ from `notify` alone. Recorded against Codex **0.156.1**; the session
file is Codex's own and undocumented, so a newer version may change a detail, and a detail this
build does not recognise is skipped rather than guessed. A Codex harness whose arguments already
set `notify` is left alone.

## What OpenCode reports

Switch on **Capture what OpenCode reports** in Settings → General (also needs **Record when
agents start and exit**, also off by default) and every OpenCode that Yardsort starts reports as
it works. OpenCode has _plugins_: small programs it loads and calls at points in its own life.
Yardsort gives its launches one, for that launch alone, through OpenCode's environment
(`OPENCODE_CONFIG_CONTENT`, which OpenCode merges with your own configuration), kept under
`activity/hooks/` in your data directory. Your `opencode.json` is not edited, and your own
plugins keep running beside it.

| Timeline row                              | What OpenCode reported                                                                                                  |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| _agent session started_                   | A session began, and which OpenCode version.                                                                            |
| _prompt submitted · 216 characters_       | Your message. The length, never the text.                                                                               |
| _write started_, _write done · hello.txt_ | A tool ran: its name and, for a file tool, the path relative to the workspace. A `bash` done row carries the exit code. |
| _read failed · missing.txt · 5 ms_        | A tool failed, and how long it took. No error text.                                                                     |
| _file changed · hello.txt_                | OpenCode edited a file.                                                                                                 |
| _permission asked for bash_               | It is waiting for your yes.                                                                                             |
| _tokens used · 10,814 total · 2 out_      | What each of its replies cost, with the model. OpenCode reports this per step, so a turn has several.                   |
| _agent finished its turn_                 | It went idle: your turn.                                                                                                |

**What is kept, and what is not.** Names, ids, relative paths, exit codes, counts, the failed
tool's duration. Not your message, not a command, not a tool's output, not a file's contents,
not an error message, not OpenCode's reply, not a path outside the workspace. The plugin keeps
only those fields before anything leaves OpenCode's process, and hands them to the Yardsort
executable, which writes one small file to the inbox. Recorded against OpenCode **1.18.31**. An
OpenCode harness whose arguments carry `--pure` — OpenCode's own "no external plugins" — is left
alone, and the timeline's settings count it under `hooks_not_armed`.

## What Grok records

Switch on **Read what Grok records** in Settings → General (also needs **Record when agents start
and exit**, also off by default) and every Grok that Yardsort starts has its turns read from the
log Grok keeps of its own accord. Grok writes, for each session, a directory under
`~/.grok/sessions/` with an event log that holds no commands, paths, prompts or replies — only
which tool ran, how long it took, how it came out, which permissions you were asked for, and
when each turn began and ended — plus a file of tokens and cost per turn. Yardsort chooses
Grok's session id when it starts it, so it knows which directory is which conversation's, and
reads it as it grows: every few seconds while the agent is running, once more after it exits.
Nothing is given to Grok, and nothing in `~/.grok` is written.

| Timeline row                                               | What Grok recorded                                                                                                               |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| _agent session started · grok-4.7_                         | The session, its model and reasoning effort.                                                                                     |
| _agent started a turn · turn 0 · grok-4.7_                 | You sent a message. Grok's log does not say how long it was.                                                                     |
| _read_file started_, _read_file done · 8 ms_               | A tool ran, and how long it took. Grok's own tool names: `run_terminal_command`, `read_file`, `search_replace`…                  |
| _read_file failed · 8 ms_                                  | A tool failed.                                                                                                                   |
| _run_terminal_command allow · you took 4.2 s_              | A permission you were actually asked for, and how long you took; or _deny_. The ones Grok granted itself at once are not listed. |
| _tokens used · 57,107 total · 464 out_                     | What the turn cost. `costUsdTicks` is kept as Grok writes it; its unit is not documented.                                        |
| _agent finished its turn_ / _agent's turn was interrupted_ | The turn ended, and how.                                                                                                         |

**What is kept, and what is not.** Tool names, durations, outcomes, decisions, waits, turn numbers,
models, token counts. Not a command, not a path, not your message, not Grok's reply — the log
never had them. Recorded against Grok **1.0.41**; the event log is Grok's own and not in its
documented file list, so a newer version may change it, and a line this build does not recognise
is skipped rather than guessed.

## What OMP and pi report

Switch on **Capture what OMP reports** or **Capture what pi reports** in Settings → General
(both also need **Record when agents start and exit**) and every OMP or pi that Yardsort starts is
given a small extension, on its command line for that launch alone, that reports what it does.
OMP is a fork of pi and the two share one extension API, so it is the same file for both. Your
own extensions keep running; nothing under `~/.pi` or `~/.omp` is edited. Passing
`--trusted-extension` yourself, in the harness's arguments, means that launch gets no extension.

| Timeline row                                         | What the agent reported                                                                                               |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| _agent session started · openai-codex/gpt-5.3-codex_ | The session and its model. On pi, the session id is one Yardsort chose; OMP picks its own.                            |
| _prompt submitted · 184 characters_                  | You sent a message, and how long it was. Never the message.                                                           |
| _agent started a turn · turn 0 · openai-codex/…_     | A turn began.                                                                                                         |
| _write started_, _write done · src/a.rs · 12 ms_     | A tool ran, on which file, how long it took. Tool names are the agent's own: `write`, `read`, `bash`, `edit`, `grep`… |
| _read failed · missing.txt_                          | A tool failed. Not why.                                                                                               |
| _permission asked for bash_, _bash allow_            | OMP asked you for permission, and what you answered (`allow` / `deny`). pi has no such events.                        |
| _agent finished its turn · 6.1 s · 19,963 tokens_    | The turn ended, how long it took and what it cost. The cost in the agent's currency is in the payload (`cost`).       |
| _agent switched model · anthropic/opus_              | pi's model was changed mid-session. OMP has no such event.                                                            |
| _agent session ended_                                | The agent shut down.                                                                                                  |

**What is kept, and what is not.** Tool names, file paths relative to the workspace (a path
outside it is marked as such, not shown), durations, outcomes, decisions, turn numbers, models,
token counts and cost, the length of each prompt. Not the prompt, not a command (a `bash` call's
command line stays in the agent), not a file's contents, not the reply — the extension copies
only the fields listed before anything leaves the agent's process. Recorded against **OMP
18.2.11**; pi **0.87.1** was recorded only as far as its first prompt, because no model provider
was configured on the recording machine, and the rest of its events are mapped from OMP's
identical shapes. The opening message Yardsort passes on the command line is not an input
event to OMP, so its length is not reported; the messages you type afterwards are.

## What Cursor reports

Switch on **Capture what Cursor reports** in Settings → General (also needs **Record when agents
start and exit**) and every Cursor agent that Yardsort starts is given a small plugin, on its
command line for that launch alone, whose hooks report what it does. Cursor runs the plugin's
hooks beside your own from `~/.cursor/hooks.json` and the project's; nothing of yours is edited.
Only hooks Cursor does not wait on for a decision are used, so nothing is ever blocked.

| Timeline row                                     | What Cursor reported                                                                                               |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| _agent session started · composer-2_             | The session and its model.                                                                                         |
| _prompt submitted · 13 characters_               | You sent a message, and how long it was. Never the message.                                                        |
| _Read done · src/a.rs · 12 ms_, _Read failed_    | A tool ran or failed, on which file, how long it took. Cursor's own tool names.                                    |
| _shell done · 30 ms_, _mcp:search done_          | A shell command ran, or an MCP tool. Never the command line.                                                       |
| _file changed · src/b.rs_                        | Cursor edited a file. How many edits is in the payload, never their text.                                          |
| _subagent started · explore_, _subagent stopped_ | A subagent ran.                                                                                                    |
| _agent finished its turn · 1,500 tokens_         | The turn ended and what it cost — when Cursor fires that hook for plugins, which this version was reported not to. |

**What is kept, and what is not.** Tool names, file paths relative to the workspace, durations,
outcomes, edit counts, token counts, prompt lengths. Not the prompt, not a command, not a tool's
output, not an edit's text, not the reply. **Built from Cursor's documentation, not a
recording**: the Cursor agent on the machine this was written on was not logged in. A field
named differently in your version is left blank on the timeline rather than guessed; the
[design note](../design/16-agent-events-stage-2-cursor.md) says how to record it.

Every built-in agent now reports natively when asked; a custom harness stays at what Yardsort
itself sees. The [design notes](../design/16-agent-events-stage-2-cursor.md#4--coverage-honestly)
say exactly what each one can and cannot report.

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
