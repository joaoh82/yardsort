# 29 — Conversation context between a workspace's agents

_Proposal, 2026-10-25. Decisions from the design pass on 2026-10-24 and 2026-10-25 are recorded.
Slices 1–3 are built — the digest for every built-in agent,
`ys workspace context`, the switch, telling every new agent, and *What was said* in the handoff
packet — and [§ slice 1](#slice-1-what-shipped), [§ slice 2](#slice-2-what-shipped) and
[§ slice 3](#slice-3-what-shipped) say where they differ from the proposal below, with
[§ after slice 3](#after-slice-3-grok-and-resume) for Grok and the note on resume, and
[§ Claude Code on resume](#claude-code-on-resume) for the last of it;
[§ checked in the app](#checked-in-the-app) records the pass on Linux._

A workspace can hold several conversations: Claude in one tab, Codex or Cursor in the next. They
share the worktree, so the second agent sees the same files and the same diff — and nothing of
what the first was asked, tried, decided or left to do. [Handoffs](18-agent-events-stage-4-handoff.md)
narrowed the gap with a packet built from the record, but the record is metadata by the contract
of [09](09-agent-events-and-memory.md), so the packet says what the last agent _did_ and, in its
own closing section, that the conversation is not there. [09](09-agent-events-and-memory.md)'s
summary named the gap when it was written: _complete conversational continuity requires an opt-in
content source or user-supplied summary._ This doc is that content source.

The idea: when a workspace already has a conversation, every agent started in it is told so and
can read a **conversation digest** — the user's messages and the agent's final reply of each
turn, taken from the harness's own transcript, with no model involved — through
`ys workspace context`. The digest is built when it is asked for and never stored, so it lasts
exactly as long as the workspace that links to it.

## What a second agent gets today

| How it is started        | What it gets                                                                                                                                            |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| An agent button, tab bar | Nothing. No first message, so not even the project's memory ([19](19-agent-events-stage-5-memory.md)).                                                  |
| The composer             | The user's message, then the _Project memory_ section.                                                                                                  |
| **Hand off…**            | The packet: each first message, branch and changed files with their writers, a summary per run, the memory, and _what is not here_ — the conversations. |
| A workflow step          | Its message and the approved memory, without the request to propose.                                                                                    |

What is in [project memory](19-agent-events-stage-5-memory.md) lasts beyond the workspace;
activity and conversation records go with it when it is deleted or forgotten without history
([10](10-agent-events-stage-1.md)). The digest belongs with the second group.

## Decisions

Settled with the user on 2026-10-24 and 2026-10-25:

1. **Agents share conversation content.** The metadata-only rule of
   [09 § privacy](09-agent-events-and-memory.md) is relaxed for this one purpose: an agent in a
   workspace may read what was said in that workspace's other conversations. Nothing else
   changes — the timeline, the inbox and the event store stay metadata, and `AGENTS.md`'s _never
   parse agent output_ still holds: the terminal is never read. What is read is the harness's own
   conversation file, which several adapters already open for metadata.
2. **Deterministic, with no model.** The digest is a selection from the transcript, not a
   summary written by a model. The same transcript gives the same digest, and fixtures test it
   as they test the adapters. This also means no key: Assist is optional and nothing may lean on
   it ([assist guide](../guide/assist.md): _nothing here is load-bearing_), and Jev could not do
   this anyway — it returns judgments, never prose. Condensing, where it is wanted, is done by the
   receiving agent, which is a model the user already runs.
3. **Not the outgoing agent's own note.** Asking each agent to keep a handoff note through `ys`
   was considered and set aside as the source: an agent may not do it, and when it does the note
   is a turn behind. See [open question 40](06-open-questions.md#conversation-context).
4. **Not the user's summary either.** The point is that nobody types the context again. The
   packet stays editable in the composer, as now, but nothing waits on it.
5. **Nothing new is stored.** The digest is read from the harness's file when an agent or the
   user asks for it. Yardsort keeps no copy, so there is no new retention, no new table and
   nothing to clear. Yardsort reaches a transcript only through a conversation's session record;
   when the workspace goes, the records go, and nothing in Yardsort can reach the transcript
   again. The file itself is the harness's, as it always was.
6. **Every way of starting an agent tells it.** A pointer, not the digest, so a new tab costs
   nothing until the user types:
   - started from the **tab bar**, the agent gets a short note as **background context** — the
     harness's system-prompt option — and no first message, so it does not start a turn;
   - started from the **composer**, the same note goes with the first message;
   - **Hand off…** puts the digest itself into the packet, where the user reads and edits it.
7. **The digest is for the workspace's other conversations.** An agent asking is never shown
   its own; Yardsort knows which one it is from `YARDSORT_SESSION_RECORD_ID`.

## What was measured

Eight Claude Code transcripts from this machine, the largest in the Yardsort projects, read on
2026-10-24 by a throwaway script: the user's typed messages, and the text of the last assistant
message before each next user message.

| Transcript | Turns | User's messages | Final replies | Share of the file |
| ---------- | ----: | --------------: | ------------: | ----------------: |
| 11.6 MB    |    14 |          8.4 kB |       10.0 kB |             0.16% |
| 13.0 MB    |    32 |         23.1 kB |       18.7 kB |             0.32% |
| 17.8 MB    |    33 |         21.0 kB |       30.5 kB |             0.29% |
| 32.8 MB    |    17 |         34.1 kB |       16.0 kB |             0.15% |
| 34.4 MB    |    25 |        105.1 kB |       51.5 kB |             0.45% |
| 51.3 MB    |    40 |          7.9 kB |        5.9 kB |             0.03% |
| 91.2 MB    |    70 |        115.9 kB |       91.4 kB |             0.23% |
| 155.5 MB   |    60 |        116.4 kB |       64.1 kB |             0.12% |

What it says:

- **Tool calls and their output are the transcript.** What a person would call the conversation
  is under half a percent of it.
- **Still too large to send whole.** 15–200 kB is 4k–50k tokens. The large ones are large
  because of pasted text in the user's messages — review comments, logs, documents. So the digest
  has a budget, and long messages are cut before recent replies are.

Codex was measured in slice 1, with the readers themselves:
[§ slice 1](#slice-1-what-shipped); Cursor in slice 3 ([§ slice 3](#slice-3-what-shipped)). Grok,
OpenCode, OMP and Pi are not measured yet.

## The digest

Per conversation in the workspace other than the caller's, newest activity first:

```text
## claude · 32 turns · started 2026-10-24 09:12 UTC · last turn 14:02 UTC · running
Asked: fix the login redirect after the session expires

### Turn 32 · 13:58 UTC
User: the redirect works now but the test still flakes on CI, look at the timeout
Claude: The flake was the 2 s timeout in auth.spec.ts racing the token refresh. I raised it to
5 s and made the refresh awaited … Left to do: the same pattern is in logout.spec.ts.

### Turn 31 · 13:40 UTC
…

Turns 1–20: 6 of 20 user messages kept, shortened; replies left out.
`ys workspace context --conversation 3f2a --turn 12` shows one turn whole.
```

- **In:** every message the user typed, and the text of the agent's last message in each turn —
  where agents say what they did, what they found and what is left. The conversation's agent,
  model, turn count, first and last turn times, and whether it is still running.
- **Out:** tool calls and their results, thinking, intermediate assistant text, subagent
  (sidechain) threads, images, and messages the harness itself injects — Claude Code's
  `isMeta` lines, local command output, hook text.
- **Out: what Yardsort added.** A first message carries the _Project memory_ section after the
  user's words; the digest drops it from the marker Yardsort wrote. A handoff conversation's first
  message is a packet, shown as one line, `[handoff packet from Yardsort]`, so packets never quote
  packets.
- **Budget:** 32,000 characters by default (about 8k tokens), `--budget` to change it. Filled
  newest turn first: the last five turns whole; then older user messages, each cut to 1,000
  characters; then older replies, cut to 600; what does not fit is counted, as above. The cut is
  marked with `…` and never splits a line in the middle of a code fence without closing it.
- **Times in UTC**, as the handoff packet's are: it may be read in another place and another day.
- **A conversation Yardsort cannot read** — no reader for its harness, the file gone, unreadable
  or over the size limit — is a header and one line saying why, so the reader knows a
  conversation existed and where its account stops.

## `ys workspace context`

```sh
ys workspace context                          # the digest, for the workspace this agent is in
ys workspace context <workspace>              # for a workspace by name, from any terminal
ys workspace context --conversation 3f2a      # one conversation only
ys workspace context --conversation 3f2a --turn 12   # one turn whole, still without tool output
ys workspace context --since 2026-10-25T13:00Z      # only turns after a time
ys workspace context --budget 64000 --json
```

- The workspace comes from `YARDSORT_WORKSPACE_ID` when no argument is given, as other `ys`
  commands find it; the caller's own conversation is left out by `YARDSORT_SESSION_RECORD_ID`.
- `--turn` with no `--conversation` is an error when there are several.
- `--json` gives `{ conversations: [{ id, harness, model, turns: [{ n, at, user, reply }],
omitted, unreadable }] }` for scripts and for the composer.
- It answers only while the project shares conversations (below), and otherwise says so — as
  `ys memory list` does for memory.
- No `--full` that includes tool output. Whole tool output is what makes transcripts megabytes,
  and an agent that needs a file's state reads the file.

## Where each harness keeps its conversation

Each needs a small reader in the core, beside its adapter. Versions are the ones installed here
on 2026-10-25.

| Harness               | Where                                                                                    | How Yardsort finds it                                                                                                                                                               | State                                                                             |
| --------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Claude Code 2.1.302   | `<CLAUDE_CONFIG_DIR or ~/.claude>/projects/<encoded cwd>/<session id>.jsonl`             | The session id is assigned by Yardsort; find the file by id under any `projects/` dir.                                                                                              | Format known: `activity::context` reads it for usage today. First reader.         |
| Codex 0.163.0         | `<CODEX_HOME>/sessions/<y>/<m>/<d>/rollout-*-<thread>[_<fork>].jsonl`                    | `activity::codex::find_rollout` by the `threadId` on the run's events — present only with Codex capture on; otherwise the newest rollout whose `session_meta.cwd` is the workspace. | `UserMessage` and `AgentMessage` items already parsed for metadata. First reader. |
| Grok 1.0.70           | `$GROK_HOME/sessions/<encoded cwd>/<id>/chat_history.jsonl`, timed by `events.jsonl`     | `activity::grok::find_session_dir` by session id.                                                                                                                                   | Read after slice 3, with fixtures.                                                |
| Cursor 2026.10.16     | `~/.cursor/projects/<cwd as dashes>/agent-transcripts/<chat>/<chat>.jsonl`               | The chat id its hooks report as `sessionId`, with capture on; otherwise by folder and first message's time.                                                                         | Read in slice 3, with a fixture.                                                  |
| OpenCode 2.0.32       | Its SQLite database, `opencode/opencode.db` under the XDG data folder                    | The session id the plugin reports, with capture on; otherwise by `session_v2.directory` and start.                                                                                  | Read in slice 3, with a fixture: the database, not the plugin.                    |
| OMP 18.11.3, Pi 1.4.0 | `<PI_CODING_AGENT_DIR or ~/.omp/agent, ~/.pi/agent>/sessions/<folder>/<time>_<id>.jsonl` | Pi's id is assigned; OMP's is the `sessionId` its extension reports, otherwise by the name's time and the `session` line's folder.                                                  | Read in slice 3, with fixtures: one format, a tree.                               |
| A custom harness      | —                                                                                        | —                                                                                                                                                                                   | No reader. Listed as unreadable, with its turn count when it reports events.      |

Transcript formats change between versions, as hook payloads do. Each reader is tested against
a recorded fixture under `crates/core/fixtures/<harness>/<version>/`, and a format it does not
recognise reads as _unreadable_, never as an empty conversation.

## Telling the agent

### The note

```text
Other agents have worked in this workspace: claude (32 turns, last 14:02 UTC, still running).
Before you answer the user's first message, run `ys workspace context` to read what they were
asked, what they decided and what they left to do. It changes as they work; run it again when
the user refers to their work.
```

About 60 words. It names the conversations as they stand at launch and is otherwise fixed
wording; the digest itself is always read live, so it is current when the agent reads it, even
while the other agent is still working. The note is given only when the workspace has another
conversation with at least one turn and the project shares conversations.

### Background context: a harness template

A new optional field on the harness definition, `context_args`, with a `{context}` placeholder,
beside `base_args`. Empty means the harness has no such option. Adding an agent still needs no
code ([01](01-vision.md)): it is configuration, editable in Settings → Harnesses, and **What will
run** shows it.

| Harness             | `context_args`                                   | Source                                                                                                                                          |
| ------------------- | ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.302 | `--append-system-prompt {context}`               | `--help`.                                                                                                                                       |
| Codex 0.163.0       | `-c developer_instructions={context}`, to verify | Not in `--help`; the key and its "cannot be blank" error are in the binary. Needs a live check.                                                 |
| Grok 1.0.50         | `--rules {context}`                              | `--help`: "Extra rules to append to the system prompt".                                                                                         |
| OMP 18.11.3         | `--append-system-prompt={context}`               | `--help`.                                                                                                                                       |
| Pi 1.4.0            | `--append-system-prompt {context}`               | `--help`; it also takes a file path.                                                                                                            |
| OpenCode 2.0.32     | through `OPENCODE_CONFIG_CONTENT`, to verify     | Its config's `instructions` takes files; Yardsort already passes config this way for the plugin. Not a template — code in the OpenCode adapter. |
| Cursor 2026.10.16   | none                                             | No system-prompt option in `--help`. Rules files live in the repository, which Yardsort will not write into.                                    |

The note is short so that it fits on any command line: Windows' limit is 32,767 characters, and
`launch::MAX_ARGV_PROMPT` already keeps Windows prompts under 24,000. That is also why the
background channel carries a pointer and never the digest.

### Each way of starting

- **Tab bar, harness with `context_args`:** the note through it; no first message; no turn.
- **Tab bar, harness without:** the button asks once, in place — **Start with the handoff**
  (the composer with the packet, as **Hand off…** does) or **Start fresh**. It asks only when
  the note would have been given.
- **Composer:** the note goes after the user's message as a _This workspace's other
  conversations_ section, after _Project memory_ and under the same rules: the agent gets it, the
  session record keeps the user's words only, so Assist and the next packet read the task, not
  the note. With `context_args` it goes there instead, and the message is left as typed.
- **Hand off…:** the packet gains a _What was said_ part between _What the agents did_ and _What
  is not here_, holding the digest within its budget; _What is not here_ then names only what the
  digest leaves out. A long packet already flips to the stdin transport by size.
- **Workflow steps:** the note, as the composer gives it. A workflow's reviewer is exactly the
  agent that should know what the author was asked.
- **Resume:** the note is given again, naming the conversations as they are now.

### Found while checking: Claude records its system prompt

Claude Code 2.1.302's `--system-prompt-snapshot` defaults to `on`: the system prompt, an
`--append-system-prompt` included, is recorded on the conversation's first request and sent
as recorded on every later request _and resume_, whatever a later launch passes, until the
conversation is compacted. So:

- A Claude conversation that began with the note keeps it on resume. Its wording is fixed and
  tells the agent to run the command again, so that is harmless.
- A Claude conversation that began **alone**, and is resumed after another agent has worked
  beside it, does not see a new note. Passing `--system-prompt-snapshot off` on such a resume
  would fix it at the cost of the cached system prompt on every request. See
  [open question 41](06-open-questions.md#conversation-context), and
  [§ Claude Code on resume](#claude-code-on-resume) for what was built: the cost turned out to
  be one cache write per told resume, not one per request.

Other harnesses are checked for the same behaviour when their template is verified.

## In the window

- **The tab says so.** A tab started with the note shows a line under its title for its first
  minute and a mark after: _told about claude's conversation_, with **Show**, which opens the note
  and the digest as they would be read now.
- **The composer says so**, under its pickers, as it does for memory: _the other conversations
  here are offered to this agent_, with **Show** and a box to leave them out of this launch.
- **One launch without it:** Alt-click on an agent button starts it bare. (Plain clicks start
  it with the note; Mod is for app shortcuts, and Alt-click is unused on those buttons today.)
- **The switch** is per project, beside memory's: **Share conversations between this project's
  agents**. See privacy below for its default.

## Privacy

Conversations move between **providers**, not only between tabs. Claude's conversation is
already Anthropic's; read by Codex, the digest goes to OpenAI too, and with Cursor, to Cursor and
whichever model it routes to. A user's message can hold whatever they pasted. That is the honest
reason this is a switch at all, and the setting says it in those words.

- The digest never leaves the machine by Yardsort's hand: it goes to an agent the user started,
  through that agent's own connection.
- ~~The default is undecided~~ — [open question 42](06-open-questions.md#conversation-context):
  first off with a question asked once per project (slice 2), then **on** from 2026-10-26, the
  user's call, with no question; the Memory view's switch turns it off, with the sentence above.
- Clearing activity does not touch this, because there is nothing of it to clear. Deleting the
  workspace ends it.

## Three platforms

- **Paths:** each reader takes the harness's home from the launch environment
  (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME`) before the home directory, as the adapters do,
  so a Windows profile and a relocated home both work.
- **Files being written:** the transcript of a running agent is read while it grows. A last line
  without its newline is a line not yet written, as `activity::grok` treats it, and is skipped.
- **Size:** ~~a transcript is read whole and refused above 256 MiB~~ — a Codex session file
  here is 2.4 GB, so transcripts are read line by line with no limit, and a line that cannot hold
  a message is skipped before it is parsed. Measured in [§ slice 1](#slice-1-what-shipped).
- **The command line:** the note is the only new argument, and it is short; see above.

## Slices

Each its own pull request, with its guides in the same one.

1. **The digest, for Claude Code and Codex.** The two readers with fixtures, the selection and
   budget in `crates/core/src/activity/digest.rs`, `ys workspace context`, the per-project
   switch, and the Codex measurement. Guides: [cli](../guide/cli.md),
   [terminals and sessions](../guide/terminals-and-sessions.md).
2. **Telling the agent.** `context_args` on the harness definition and its defaults (each
   verified live), the note, the composer section, the tab bar's line and **Show**, Alt-click,
   the question for harnesses without the option, and resume. Guides: terminals and sessions,
   [workspaces](../guide/workspaces.md), [settings](../guide/settings.md); checklist rows in
   [08](08-manual-checklist.md).
3. **The packet and the rest.** _What was said_ in **Hand off…** and `ys workspace handoff`;
   readers for Grok, Cursor, OpenCode, OMP and Pi as their fixtures arrive.

## Tests

- **Readers:** each fixture reads to the expected turns; tool output, thinking, sidechains and
  injected lines are absent; a half-written last line is skipped; an unknown format is
  _unreadable_, not empty.
- **The digest:** the memory section and a handoff packet are stripped from first messages; the
  caller's conversation is absent; the budget keeps the newest turns whole and counts the rest;
  the same input gives byte-identical output.
- **Launch:** with another conversation in the workspace, a tab-bar launch of a harness with
  `context_args` carries the note in argv and sends no prompt; one without it does not spawn
  before the user chooses, and **Start fresh** sends nothing; the composer's spawn carries the
  note while the session record keeps only the user's words; Alt-click and the switch turned off
  give nothing. Each test fails without the change it covers.
- **Live:** Claude then Codex in one workspace, from the tab bar, on each platform: Codex runs
  `ys workspace context` before its first answer and uses what it read. Recorded in 08.

## What this does not do

- **No summary by a model.** If a written summary is ever wanted, it is the receiving agent's
  job, or an opt-in with its own credentials ([09](09-agent-events-and-memory.md)); Assist does
  not write prose.
- **No automatic switching.** Nothing starts an agent or types to one because of a digest.
- **Nothing across workspaces.** A workspace's conversations are for its own agents. What should
  outlive it is memory, proposed and approved as it is now.
- **No transcript content in the timeline, the inbox or Assist.** Those stay as they are.

## Slice 1: what shipped

Built on 2026-10-25, after the default was settled: **off** until the user turns it on
([open question 42](06-open-questions.md#conversation-context)).

- **The readers**, `crates/core/src/activity/transcript.rs`. Claude Code: a `user` line is the
  user's when its content is text, it is not `isMeta`, a compact summary or a sidechain, its
  `origin.kind` — where the version writes one — is `human`, and it does not start with a tag
  Claude Code puts there itself (a slash command and its output, a background task's report,
  `[Request interrupted`). Assistant text is gathered per `message.id`, since a message is written
  once per content block; `<synthetic>` messages are not the model's. Codex: `UserMessage` and
  `AgentMessage` items of the file's own thread; the reply is the last agent text of the turn,
  and `task_complete`'s `last_agent_message` when no `final_answer` came. Both stream: a line
  that cannot hold anything wanted is skipped unparsed, and a last line without its newline is
  left for the next read.
- **Fixtures recorded for this**, by `scripts/record-conversations.sh`: a two-turn Claude Code
  2.1.302 conversation and its fork, and a two-turn Codex 0.163.0 thread across `exec resume`.
  The raw files carried the recording account's own setup — Claude Code's attachment and request
  lines hold the system prompt, the user's memory, skills and connectors; Codex's `world_state`
  lists skills and the time zone — so the script keeps only the conversation's lines whole and
  leaves the rest as their type alone. Claude Code's print mode marks the user's message
  `promptSource: "sdk"` with no `origin`, an interactive one `origin.kind: "human"`; the reader
  takes both, and a hand-written test covers the lines only an interactive session writes.
- **What the recordings showed.** A Claude Code fork repeats the conversation before it with the
  original times, so a fork's turns before its record began are dropped and the digest says whose
  they are. Codex appends a resumed thread to the same session file.
- **When a conversation began.** A session record's `started_at` moves to the latest start on
  every resume, which the sessions list wants. The digest needs the first — a fork's own turns are
  those after it, and a Codex file keeps the time it began — so migration 0017 adds `began_at`,
  which a resume leaves alone. Older records get their first capture run's start if it was
  earlier; a record resumed before this, without capture, has lost its first start.
- **Finding a Codex conversation.** By the `threadId` its run reported, when Codex capture was
  on; otherwise by a session file whose `session_meta` names the workspace's folder and began
  between a minute before the conversation began and its end. Each file goes to one conversation,
  never one whose thread another reported, and the closest in time is matched first: a
  conversation whose file is gone is listed as not found rather than given a newer one's — the
  asking agent's own takes part for that, though it is not shown. The day folders are Codex's
  local dates, so the days either side are looked in too, and only each file's first line is
  read.
- **The digest**, `crates/core/src/activity/digest.rs`, as proposed, with these differences:
  each message is quoted (`> `) under `User:` and `Agent:`, so a heading in a message cannot pass
  for one of the digest's; there is no _Asked:_ line — the first message is kept as the task,
  cut to 2,000 characters, and keeps that cut if it later gains its reply; a conversation with no
  turn yet is left out, and one that cannot be read is listed last with why.
- **`ys workspace context`**: as proposed, plus `--turn` with no `--conversation` when there is
  only one. It drains the inbox first, as every `ys` read does, so a Codex thread reported a
  moment ago is found by id.
- **The switch** lives in the Memory view, under memory's own: migration 0016
  (`project_conversations`, no row is off), `conversations_share`, and `conversationsShared` on
  `ProjectMemory`.

Measured with the readers on this machine's largest transcripts, release build: the user's
typed messages and each turn's reply.

| Agent  | Transcript | Turns |  Typed | Replies | Share of the file | Read in |
| ------ | ---------- | ----: | -----: | ------: | ----------------: | ------: |
| Claude | 155.5 MB   |    31 |  7,881 |  57,801 |            0.042% |   57 ms |
| Claude | 91.2 MB    |    41 |  5,785 |  74,703 |            0.088% |   39 ms |
| Claude | 49.4 MB    |    32 |  9,931 |  73,162 |            0.168% |   27 ms |
| Claude | 13.0 MB    |    14 |  2,070 |  16,887 |            0.145% |    7 ms |
| Codex  | 2,423.2 MB |   208 | 97,765 | 115,184 |            0.009% | 2132 ms |
| Codex  | 381.1 MB   |   240 | 90,379 | 128,200 |            0.057% |  162 ms |
| Codex  | 69.8 MB    |    50 | 21,994 | 107,363 |            0.185% |   25 ms |
| Codex  | 5.0 MB     |     9 | 37,552 |   3,647 |            0.827% |    2 ms |

Characters, not bytes. The Claude rows are smaller than the first measurement's because that one
counted every text `user` line — compact summaries and background-task reports among them —
where the reader keeps only what was typed. The 2.4 GB Codex file is the desktop app's, not one
Yardsort started; it is read in about two seconds, and nothing smaller comes near.

Not done in this slice, as planned: telling the agent (slice 2), and the packet and the other
harnesses (slice 3). Owed: the hands-on pass in [08](08-manual-checklist.md) on macOS and Windows.

## Slice 2: what shipped

Built on 2026-10-25.

- **`context_args`** on the harness definition, with `{context}`, between the session and prompt
  args; editable in Settings → Harnesses as **Context args**, and **What will run** shows it with
  a sample. Shipped for the two checked live, by giving a codeword through the option and asking
  for it: Claude Code 2.1.302, `--append-system-prompt {context}`, and Codex 0.163.0,
  `-c developer_instructions={context}` — accepted though not in `--help`, a value that is not
  TOML taken as the string itself, quotes and newlines included. **Not shipped** for Grok, OMP
  and Pi: their options are in `--help`, but no account was signed in on this machine to check
  them, so they are told after the first message; a user can set them. OpenCode's would be code
  in its adapter, and Cursor has none.
- **The note**, `digest::note`: as proposed, naming up to four conversations by agent, turn count
  and last turn, the rest counted. It is built from the same reading as the digest, at launch, so
  starting an agent beside other conversations reads their transcripts — milliseconds for
  ordinary ones, measured in [§ slice 1](#slice-1-what-shipped). Given only with a turn said; the tab gets
  a `toldAbout` label with what it was about.
- **Where it goes** is decided in `launch::resolve_launch_with`, so every way of starting a new
  agent — tab bar, the empty workspace's buttons, composer, workflow steps, the conflict helper's
  new conversation, `ys workspace new` — gets the same rule: background context when the harness
  has `context_args`, the message left as typed; otherwise a _This workspace's other
  conversations_ section after the message (after _Project memory_); otherwise nothing. The
  digest strips that section from first messages as it does memory's. A handoff gets it too: its
  packet still says the conversation is not in it, until slice 3.
- **Resume does not give the note** — a finding, checked live on both: Claude Code keeps the
  system prompt a conversation began with (its `--system-prompt-snapshot`, on by default), and
  Codex keeps its developer instructions the same way; a new one on resume is ignored, with
  `-c` before or after `resume --last`. A note there would be a line in **What will run** the
  agent never sees, so `continue_args` leaves `context_args` out.
  [Open question 41](06-open-questions.md#conversation-context) stays open for the
  snapshot-off option.
- **In the window:** a line under the tabs for a new told agent's first minute, a **⇄** mark on
  its tab after, and **Show** on both, opening the note and the digest as they read now; the
  composer's line with **Show** and a box (`skipContext`); **Alt-click** on any agent button;
  for a harness without `context_args`, the button asks — **Start with the handoff…** or
  **Start fresh** — and asks only when there is something to be told.
- **Asked once per project**, as [open question 42](06-open-questions.md#conversation-context)
  planned: before an agent starts beside a conversation that has said something, in a project
  that has never answered, a dialog asks whether to share, in the privacy section's words.
  Either answer writes the project's row in `project_conversations` — so "asked" needs nothing
  new: no row is the question not yet asked — and the launch goes on; closing it starts nothing.

Owed: the live check in [08 § 26](08-manual-checklist.md#26--conversation-context) — Codex started
from the tab bar beside Claude reads the digest before its first answer — on each platform.

## Slice 3: what shipped

Built on 2026-10-26.

- **_What was said_ in the packet.** `digest::packet_section` gives every conversation in the
  workspace — none is the asker's, since the packet is for an agent not started yet — within
  `PACKET_BUDGET`, 16,000 characters (half `ys workspace context`'s, since the user reads it in the
  composer), with the digest's headings a level down under the packet's `## What was said`. It goes
  after _What the agents did here_ and before _Project memory_, and only where the project shares
  conversations and one of them has said something. With it, the packet's opening says where its
  words come from, and _What is not here_ names what the part leaves out — reasoning, tool calls
  and their output, the turns it shortened — with the `--turn` line, instead of "the conversations
  themselves". `Facts.said` is filled by each caller — **Hand off…**, `ys workspace handoff`, a
  workflow's handoff — since it needs the launch environment's folders. The composer's line about
  the packet now says it may hold what was said.
- **Cursor**, 2026.10.16-e3cb207, recorded by `scripts/record-conversations.sh` (now four agents):
  `~/.cursor/projects/<folder>/agent-transcripts/<chat>/<chat>.jsonl`. Found in Cursor's own code
  rather than guessed: the folder is the working directory with every run of non-alphanumerics
  made one dash and trimmed, always under the home folder, whatever `CURSOR_CONFIG_DIR` says. (The
  folders with a hash on the end are a worker socket's, which has a path-length limit.) Each line
  is `{role, message: {content}}` with `text` and `tool_use` blocks; **no tool output is written at
  all**, so the 13 transcripts here are 1–170 KB and the conversation is 7–50% of each. The user's
  words are inside `<user_query>`, after a `<timestamp>` in local time to the minute —
  `Monday, Oct 26, 2026, 9:53 AM (UTC+1)` — the only time in the file; every one of the 13, the
  interactive ones from Yardsort included, has both. The fixture is recorded with `TZ=UTC`, so its
  times read as UTC; the offset form is tested on its own. A user line without `<user_query>`
  reads as a form not known, not as nothing.
- **OpenCode**, 2.0.32: not files but its SQLite database, `opencode/opencode.db` under
  `XDG_DATA_HOME` or `~/.local/share`, opened read-only. A conversation is a `session_v2` row
  with no parent (a subagent's has one, and reports its own session id, so a reported id is
  checked); its `session_message` rows in `seq` order are `user` — `text` — and `assistant` —
  `content` of `text`, `reasoning` and `tool` parts — and `idle`. Times are milliseconds. The
  fixture is the two tables as SQL, from a throwaway data folder and a free model, so no account
  was needed; `opencode run` keeps its message in quotes, and the fixture keeps them. A database
  without these tables, OpenCode 1's, is _a form Yardsort does not know_. Only `opencode run` was
  recorded; an interactive session goes through the same server.
- **Finding a conversation** is now one function for the three agents whose ids Yardsort does not
  choose: an id the conversation reported names its own — Codex's `threadId`, the `sessionId`
  Cursor's hooks and OpenCode's and OMP's extensions give (`Store::reported_ids`) — and the others are matched
  by folder and time, each transcript to one conversation, closest first. Cursor's slack is two
  minutes, for its minute-long times.
- **OMP 18.11.3 and pi 1.4.0**, recorded once the user signed both in, the same afternoon: one
  format, `sessions/<folder>/<time>_<session id>.jsonl` under the agent folder, read by one reader.
  The file is a **tree** — every entry has an `id` and `parentId`, and going back to an earlier
  point leaves the old branch in the file — so the conversation read is the branch ending at the
  last entry. pi records the **whole system prompt** as a `system` message, the user's own skills
  and their paths among it, and both keep the provider's own payload and the reasoning: the
  script strips all three from the fixtures. A pi fork is a new file with a copy of the branch at
  the original times, which the fork rule already handles. pi is found by the id Yardsort gives
  it; OMP by the `sessionId` its extension reports, or by the time in the file's name and the
  folder its `session` line names. `PI_CODING_AGENT_DIR`, which both read, is honoured.
- **Their `context_args`, checked live** with the note's own wording, a quote and a newline: OMP
  `--append-system-prompt={context}`, pi `--append-system-prompt {context}`. Unlike Claude Code
  and Codex, **both take a new system prompt on resume** — each launch builds it afresh — so a
  resume could tell them; that needs a per-harness switch and is left with
  [open question 41](06-open-questions.md#conversation-context).
- **Grok** still reads as _Yardsort cannot read grok's conversations yet_: it could not be signed
  in on this machine to record a fixture (the one Grok session here is a month old, from 1.0.41,
  and five lines long), and a reader without one is a guess.

Owed: the live check in [08 § 26](08-manual-checklist.md#26--conversation-context) for the packet
and for Cursor, OpenCode, OMP and pi beside Claude, and the readers on macOS and Windows — OpenCode's data
folder there is assumed, from its use of XDG folders everywhere, not checked.

## After slice 3: Grok and resume

Built on 2026-10-26, once Grok was signed in.

- **Grok 1.0.70.** `chat_history.jsonl` in the session folder has **no times**. A typed message is
  a `user` line with a `prompt_index`, inside `<user_query>`; the other `user` lines are Grok's —
  `<user_info>` with the OS, shell and path, and `system_reminder`s listing the user's skills and
  MCP servers — and the first line is the whole system prompt. Times come from `events.jsonl`: a
  `turn_started` whose `turn_number` is the prompt's index. A **fork** (`--resume
--fork-session`) copies the history but not the events, so a copied prompt has no start of its
  own: it reads as untimed and the fork rule leaves it out, while an untimed turn of a
  conversation that is not a fork takes its start. The fixtures keep only `turn_started` events,
  and the system prompt, Grok's own lines and the reasoning are stripped.
- **Grok's `--rules {context}`**, checked live with the note's wording, a quote and a newline. On
  resume it keeps the rules the conversation began with, as Claude Code and Codex do: asked to end
  every reply with a word, a resumed conversation did not, and a new one did.
- **The note on resume, for OMP and pi**
  ([open question 41](06-open-questions.md#conversation-context)): a new harness field,
  `resume_context`, puts `context_args` after the resume or fork args. It is on for OMP and pi,
  which build their system prompt at each launch, and editable in Settings → Harnesses. The app's
  resume and fork compute the note as a new start does, about every conversation but the one
  continued (`digest::note_on_resume`), and label the tab, so its line and mark show. (The
  field was then a switch, `resume_context`; it became a template of its own the same day, for
  Claude Code — [§ Claude Code on resume](#claude-code-on-resume).)

Owed: the rows for Grok and an OMP resume in
[08 § 26](08-manual-checklist.md#26--conversation-context), and macOS and Windows.

## The default: on

Changed on 2026-10-26, the user's decision: a project shares its agents' conversations unless it
is turned off ([open question 42](06-open-questions.md#conversation-context)). A project with no
row in `project_conversations` now shares — `Store::conversations_shared` reads a missing row as
on; migration 0016, already merged, still says off and is left as it was. The once-per-project
question is removed (`digest::unasked`, `conversations_unasked`, the dialog), since it asked to
turn on what is already on. A project whose user answered _Not in this project_ keeps its row,
and stays off.

## Claude Code on resume

Built on 2026-10-26, the last of
[open question 41](06-open-questions.md#conversation-context). Checked live with Claude Code
2.1.302 in print mode, on a throwaway repository, with a note asking every reply to end in one
word:

| Launch on a conversation that began with no note   | Note followed | Cache written | Cache read |
| -------------------------------------------------- | ------------- | ------------- | ---------- |
| `--resume` with `--append-system-prompt`           | no            | 26,943        | 14,430     |
| the same with `--system-prompt-snapshot off`       | yes           | 32,033        | 11,236     |
| the same again                                     | yes           | 1,901         | 41,421     |
| `--resume` with neither                            | no            | 3,081         | 40,265     |
| a fork (`--fork-session`) with both                | yes           | 1,205         | 43,297     |
| snapshot off, three requests in one launch (tools) | yes           | 148–242 each  | ~43,000    |

So snapshot off builds the system prompt afresh for that launch, and the build is stable: the
next request, and the next launch with the same note, read it from the cache. The cost the
question feared, a cached system prompt lost on every request, is not there. What remains is one
write of the conversation's cache on a told resume whose prompt differs from the last one sent,
which a resume after the cache has expired pays anyway. The snapshot itself is untouched: a later
launch without the option sends what the conversation recorded first.

- **`resume_context` became `resume_context_args`**, a template like `context_args`, placed after
  the resume or fork args. OMP and pi have their context args there; Claude Code has
  `--system-prompt-snapshot off --append-system-prompt {context}`; Codex and Grok have nothing.
  As with every group, it is dropped whole when there is no note, so a resume with nothing to
  tell is the plain `--resume` it was. Settings → Harnesses has it as **Resume context args** in
  place of the box. The field was not released, so nothing reads the old one.
- **Every told resume, not only one that began alone.** The lean was to turn the snapshot off only
  for a conversation that began without the note. Yardsort does not record what a conversation
  began with, and a conversation that began with it gains a note naming the conversations as they
  are now, for the same one write.
- **An older Claude Code refuses the option** (`error: unknown option`), and the resume fails to
  start. Built-in flags follow the version verified, as elsewhere in
  [04](04-harnesses.md); the settings guide says to clear the field or update Claude Code. When
  the option arrived is not known.

Owed: macOS and Windows.

## Checked in the app

On 2026-10-26 every row of [08 § 26](08-manual-checklist.md#26--conversation-context) passed on
Linux, in a development build on a throwaway profile with all seven agents signed in: Codex beside
Claude told without a question and reading the digest first; the composer's line and its box;
Alt-click; Grok without context args offering the handoff; a resumed Claude that began alone run
with `--system-prompt-snapshot off` and naming Codex's conversation, and run plainly once sharing
was off; the packet's _What was said_ with sharing on and off; Grok, OMP and pi told from the tab
bar, a resumed OMP told again and a resumed Grok not; every conversation in the digest, without
tool output. Found on the way:

- The handoff row and the row for the other five agents had been filed under § 21; they are in
  § 26 now.
- `ys attach` and `ys logs` took a workspace name or a terminal's own id, not the record id
  `ys session list` prints, so two agents in one workspace could not be told apart. They take the
  record's id too now, and a refusal lists each choice with an id to name it by.
- pi, on Kimi K2.6, was given the note and answered a question about the folder without running
  the command; the others ran it first. The note asks; it cannot make a model do it.
