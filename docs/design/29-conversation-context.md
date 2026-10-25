# 29 — Conversation context between a workspace's agents

_Proposal, 2026-10-25. Decisions from the design pass on 2026-10-24 and 2026-10-25 are recorded;
nothing here is built yet._

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

Not measured yet: the same for Codex, Grok, Cursor, OpenCode and Pi. Slice 1 repeats it for
Codex before the budget is fixed.

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

| Harness               | Where                                                                        | How Yardsort finds it                                                                                                                                                               | State                                                                                          |
| --------------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Claude Code 2.1.302   | `<CLAUDE_CONFIG_DIR or ~/.claude>/projects/<encoded cwd>/<session id>.jsonl` | The session id is assigned by Yardsort; find the file by id under any `projects/` dir.                                                                                              | Format known: `activity::context` reads it for usage today. First reader.                      |
| Codex 0.163.0         | `<CODEX_HOME>/sessions/<y>/<m>/<d>/rollout-*-<thread>[_<fork>].jsonl`        | `activity::codex::find_rollout` by the `threadId` on the run's events — present only with Codex capture on; otherwise the newest rollout whose `session_meta.cwd` is the workspace. | `UserMessage` and `AgentMessage` items already parsed for metadata. First reader.              |
| Grok 1.0.50           | `$GROK_HOME/sessions/<encoded cwd>/<id>/chat_history.jsonl`                  | `activity::grok::find_session_dir` by session id.                                                                                                                                   | The file is listed in [14](14-agent-events-stage-2-grok.md) but never read; needs a fixture.   |
| Cursor 2026.10.16     | The hook payload's `transcript_path`                                         | Not kept today: the adapter would record it, so only with Cursor capture on.                                                                                                        | Format unknown; needs a fixture. [16](16-agent-events-stage-2-cursor.md) never reads the path. |
| OpenCode 2.0.32       | OpenCode's own storage                                                       | By the session id the plugin reports.                                                                                                                                               | Unknown. The plugin could report message text itself instead; decided on a fixture.            |
| OMP 18.11.3, Pi 1.4.0 | Their session files                                                          | Pi's id is assigned; OMP's is latest-in-cwd.                                                                                                                                        | Needs fixtures.                                                                                |
| A custom harness      | —                                                                            | —                                                                                                                                                                                   | No reader. Listed as unreadable, with its turn count when it reports events.                   |

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
  [open question 41](06-open-questions.md#conversation-context).

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
- The default is undecided — [open question 42](06-open-questions.md#conversation-context). The
  lean: off until asked, and asked once per project, the first time a second agent starts in a
  workspace that has a conversation, with the sentence above.
- Clearing activity does not touch this, because there is nothing of it to clear. Deleting the
  workspace ends it.

## Three platforms

- **Paths:** each reader takes the harness's home from the launch environment
  (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME`) before the home directory, as the adapters do,
  so a Windows profile and a relocated home both work.
- **Files being written:** the transcript of a running agent is read while it grows. A last line
  without its newline is a line not yet written, as `activity::grok` treats it, and is skipped.
- **Size:** a transcript is read whole and refused above 256 MiB; the largest here is 155 MB.
  Reading from the end until the budget is full is the follow-up if that is slow — not before a
  measurement says so.
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
