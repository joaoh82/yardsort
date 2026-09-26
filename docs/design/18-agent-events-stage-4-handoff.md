# 18 — Agent events, stage 4: handoffs

Status: **first slice shipped** · 27 September 2026 · Stage 4 of
[09](09-agent-events-and-memory.md), and the user story the whole design opened with: _start
Claude in a workspace, then launch Codex with a previewed, edited snapshot of the available
Claude and worktree context; missing conversation content is labeled; Codex receives the packet
through normal prompt transport; no automatic agent switching._ This slice is that story, with a
deterministic packet and no model in the loop.

## 1 · What there was to build on

- **The second agent already has a door.** The tab bar's agent buttons start a bare harness in
  the workspace, with no prompt; the composer, opened with `composeIn(workspace)`, starts one
  _with_ a prompt through the same `ptySpawn` and the same `HarnessRequest` as a new
  workspace. So "the normal prompt transport" is one field, and argv or a paste over stdin is
  already decided per harness in `resolve_launch`, with the size limit that flips a long prompt
  to stdin.
- **The task is on record.** Every conversation's first message is its session record's
  `prompt`, and `session_prompts` is what Assist reads as "what this workspace was asked".
- **The evidence is in the store.** Runs, events and the provenance join of stage 3 give, per
  run: how it reported, its turns, its tools by name, what failed and on which file, what it
  reported writing, permissions asked, notifications raised; per changed file: who reported
  writing it, or who was running a command when it was last written.
- **Git's part lives in the app.** Branch, base and the change list are `Changes::list`; the
  commits on the branch since its base are `Git::commits_since`. The CLI has neither today.
- **What does not exist:** the conversation. Yardsort records metadata only, by the contract of
  stage 1, so "what Claude tried and rejected" is not anywhere to be assembled from.

## 2 · Decisions

- **The packet is prose from facts, and says what it lacks.** `activity::handoff::render`
  writes Markdown in five parts: what the workspace was asked (each first message, whole, up
  to a limit), where the work stands (branch, base, commit subjects, changed files with the
  Changes list's word on who wrote each), what the agents did (one entry per run: harness,
  model, when, how long, how it ended, then turns, tools with counts, failures with their
  files, files reported written, permissions, notifications — or, for a run that was not
  reporting, that only its start and end are known), and what is not here: the conversations
  themselves, the commands and past file contents, and how many events stand behind the
  summary with the `ys activity list` line that shows them. A workspace with nothing recorded
  says so at every turn. Nothing in it comes from a model; Jev's stage-4 row — ranking which
  facts matter — is deferred until there is a packet in use to rank.
- **Preview and edit are the composer.** **Hand off…** in the tab bar asks the core for the
  packet and opens the composer to run in this workspace with the packet as the message. The
  user reads it, changes it, picks the agent, model and effort there, and starts it with Enter
  as always. No new dialog: the composer is where a first message has always been written, and
  the packet is a first message.
- **A handoff is not the task.** The session's record keeps no prompt for a handoff
  (`HarnessRequest.handoff`), and its title is "Handoff". Otherwise the packet — which quotes
  the original task and adds pages of status — would become part of `session_prompts`, and
  Assist would judge every later change against it, and the next packet would quote it back.
  The task on record stays the user's own words. The packet still goes over the wire exactly
  as any prompt, argv or paste.
- **UTC in the packet.** It may be read on another machine or another day; a bare time is a
  guess. The timeline on screen stays local.
- **The change list's words, not new ones.** "Reported written by claude", "last written while
  claude ran a command; not reported", "no agent reported writing it" are the sentences the
  panel and Assist already use; a reader who has seen one has seen them all. With no run
  reporting, no writer is named, and the closing section says why.
- **Not built:** a `ys` command for the packet. The core has the store half; the git half
  (`Changes`) is the app's. `ys workspace handoff <workspace>` printing the packet is a small
  follow-up once the change list moves to the core, and is noted in [05](05-roadmap.md).

## 3 · What shipped

- `crates/core/src/activity/handoff.rs`: `Facts`, `ChangedFile`, `RunSummary`;
  `from_store(store, workspace, files)` for the store's half, `summarize(runs, sessions,
events)` per run, `render(&Facts)` for the text.
- `HarnessRequest.handoff` (serde default false): the record keeps no prompt and is titled
  "Handoff".
- `workspace_handoff` command: git's half — head, base, `commits_since`, the change list with
  each file's modification time — then `render`. `HandoffPacket { text, runs, events }`.
- Tab bar: **Hand off…**, before the agent buttons. Composer: the message starts as the packet,
  the heading reads "Hand off in <project>", a line above the box says what the text is and is
  not, the box is taller, and the spawn carries `handoff: true`. `composeIn(workspace, prompt)`
  and `composingPrompt` on the projects store, cleared with the composer.
- Guides: [terminals and sessions](../guide/terminals-and-sessions.md#handing-work-to-another-agent),
  [workspaces](../guide/workspaces.md#starting-one-the-composer); checklist
  [08 §19](08-manual-checklist.md#19--handing-off).

## 4 · Verification

- `activity::handoff` tests (3): a recorded Claude run renders every part — the task whole,
  the branch line, the commit, each file with its writer, the run with its model, duration,
  exit, turns, tools by count, failure with file, permission — and leaks no id, path or
  mechanism; an empty workspace and a silent run are described as such; dates and durations
  read plainly. `launch` tests pass with the flag.
- `Composer.test.tsx`: the packet fills the box, the heading and the note say handoff, an
  edit is kept, and the spawn carries the edited packet with `handoff: true` and creates no
  workspace. `WorkspacePanel.test.tsx`: **Hand off…** asks the core for the packet and opens the
  composer here with it, spawning nothing.
- Live: owed — [08 §19](08-manual-checklist.md#19--handing-off), the story itself: Claude, then
  Codex from the packet.

## 5 · What this slice does not do, and the next

- **Jev ranking** of which facts matter (09's stage-4 row): after use, not before.
- **Test and error evidence** beyond failed tool calls: the adapters record no output, so a
  failing test run is a `Bash` that completed. If a run-output source ever exists (the daemon's
  run terminal for a project's run command is one), it belongs here.
- **Excerpts with event ids.** The packet cites the count and the CLI line rather than ids per
  line: an agent cannot open an id, and a person has the timeline.
- **`ys workspace handoff`.** See §2.
- **Stage 5**, reviewed memory, is the next stage of [09](09-agent-events-and-memory.md); the
  packet is a one-off, and that document says a one-off handoff is useful before a memory
  engine exists. It is.
