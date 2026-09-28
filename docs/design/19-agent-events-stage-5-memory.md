# 19 — Agent events, stage 5: reviewed memory

Status: **first slice shipped** · 27 September 2026 · Stage 5 of
[09](09-agent-events-and-memory.md): _candidate extraction, user approval/edit/reject, scoped
project knowledge, local search and read-only MCP_, with the exit gate _unapproved candidates
never become agent instructions; revocation/update works; injection is opt-in_.

## 1 · What the fit pass found

- **The record cannot supply a lesson.** Stage 1 made it metadata only: it knows that Claude ran
  Bash twice, not that "the tests need `TZ=UTC`". So "candidate extraction" from events has
  nothing to extract. A lesson comes from a person, from an agent that knows it, or from a
  model reading diffs.
- **Agents differ in how they take a tool.** Claude takes an MCP server per launch
  (`--mcp-config`); Codex and OpenCode can through per-launch configuration; Cursor possibly
  through its plugin directory; Grok only has a command that edits its own configuration, which
  Yardsort never touches; pi and OMP have no MCP. But every agent can run a shell command, and
  `ys` ships with the app.
- **A launch already has a place for context that is not the task.** Stage 4's handoff sends a
  packet as the first message and keeps no prompt on the record, so `session_prompts` — what
  Assist and the next packet read — stays the user's words.
- **09's guard rails:** project scope first; approved entries only, with citations; revocation;
  opt-in injection; no agent approves its own candidate; local search before embeddings; Jev
  ranks, never writes.

## 2 · Decisions

Four put to the user on 2026-09-27; three answered as proposed, one otherwise.

- **Candidates: the user writes, agents propose.** An entry the user writes is approved as
  written. An agent proposes with `ys memory propose "…"`: always a candidate. The command line
  has no approve, edit, reject or revoke, because an agent in a Yardsort terminal can run
  anything `ys` offers — that is how "no agent approves its own candidate" is enforced rather
  than asked for. Model extraction is not built.
- **Delivery: the first message and `ys memory`.** With the project's **share** switch on, a
  launch with a first message gets a _Project memory_ section after it — at launch
  (`resolve_launch_with`), never in the record — and a handoff packet carries the same section.
  `ys memory list` and `ys memory search` read approved entries for any agent, on demand, under the same switch. No MCP
  server: every agent runs a command, and not every agent takes MCP.
- **Review: a Memory view per project**, from the project menu, with a count on the row.
- **Jev now** (the user's choice over "later"): two typed questions per candidate against the
  approved entries — does it repeat one, does it contradict one — one criterion each, behind
  their own Assist switch (`check_memory`), since memory text leaves the machine.

Further decisions made in building:

- **One paragraph, 500 characters, folded.** `memory::clean` folds control characters and line
  breaks into spaces, so an entry quoted as one list item cannot leave it, and a note stays a
  note.
- **Framed as notes, cited.** The section says the entries are context, not instructions, and
  that the user's request wins; each entry carries `(memory <id8>, from <who>)`.
- **Rejected entries are kept**, so the same proposal (case-insensitive) is recognised, not
  re-queued; **revoked entries are kept** with their history, and restorable.
- **A bounded queue**: 50 candidates per project; an agent proposing in a loop is refused with a
  reason, and the user's queue stays reviewable.
- **Attribution from the launch environment**: `YARDSORT_RUN_ID` names the run, its harness and
  workspace; `YARDSORT_WORKSPACE_ID` the workspace; failing both, the workspace whose folder holds
  the working directory. Names are stored beside ids, so a citation outlives its workspace.
- **Per-launch opt-out**: the composer shows the section (**Show**) and a box to leave it out of
  one launch (`HarnessRequest.skip_memory`). A bare launch, with no message, gets nothing.
- **Checked and inserted in one transaction.** Duplicate detection and the queue bound are
  decided inside the immediate transaction that inserts a proposal (`Store::propose_memory`):
  found in review, eight `ys` processes racing could otherwise all pass the checks. A test races
  eight connections to one file.
- **Reading follows the switch too.** `ys memory list` and `search` refuse while the project does
  not share its memory, as a launch adds nothing then; `propose` works either way, since a
  proposal reaches no agent. Found in review: the first cut read regardless, which contradicted
  the premise the user answered the delivery question on.
- **Jev's tags follow the approved list.** A verdict depends on the approved entries as much as on
  the proposal, so the view asks again when either changes and shows an answer only while it
  still matches what it was judged against.
- **Waiting counts are polled**, on focus and on every activity event: `ys` writes the database
  without the window hearing of it.

## 3 · What shipped

- Migration `0010_project_memory.sql`: `memory_entries` (text, state, author, source), an
  append-only `memory_history` (action, by, the text an edit replaced), `project_memory` (share).
- `crates/core/src/memory.rs`: `clean`, `write`, `propose`, `decide` (approve, reject, revoke,
  restore, each only from the states it fits), `edit`, `approved`, `search`, `prompt_section`,
  `render`, `locate`. Launch: `resolve_launch_with` and `skip_memory`. Handoff: `Facts.memory`.
- `ys memory list | search | propose`.
- App: `memory_get | write | edit | decide | share | waiting | check`; the Memory view; **Memory…**
  in the project menu and a count on the row; the composer's memory line; the Assist switch;
  `assist::memory` for Jev's two questions, cached per candidate and approved list.
- Guide: [Memory](../guide/memory.md); the CLI, Assist, projects and handoff guides; checklist
  [08 §20](08-manual-checklist.md#20--project-memory).

## 4 · Verification

- `memory` (7): a user's entry is approved and an agent's waits, with its history; racing
  proposals from separate connections neither duplicate nor overflow; revocation
  and restoration, and decisions refused out of state; an edit keeps the replaced text, and an
  entry is one bounded paragraph; the candidate queue is bounded; the section is cited, framed,
  newest first, and only when shared; a proposal is located by run, workspace or folder.
- `launch`: memory reaches the agent after the message and never the record; a handoff, an
  opt-out, a bare launch and an unshared project get nothing. `handoff`: the packet carries it.
- `ys` end to end: an agent's proposal is attributed to its run, invisible until approved,
  recognised when repeated; `ys memory approve|edit|reject|revoke` do not exist; search finds
  it after approval, cited.
- `assist::memory`: the two questions per candidate, the flag threshold, the cache, and nothing
  sent with nothing approved. Frontend: the Memory view's lists, decisions, writing, editing,
  sharing, refusal and Jev tags; the row count and menu label; the composer's line and opt-out.
- Live, on Linux: writing an entry, an agent's proposal through `ys`, approving it. Owed — the
  rest of [08 §20](08-manual-checklist.md#20--project-memory), and macOS and Windows.

## 5 · Not in this slice

- **Model extraction** of candidates from a finished workspace — generative, its own opt-in.
- **A read-only MCP server**, for agents that take one — `ys memory` covers every agent today.
- **Directory and global scopes** — project scope first, as 09 says.
- **Jev's relevance ranking** of approved entries for a given task — the section is newest first,
  40 at most, and the rest a search away.
- **Stage 6**, outcome intelligence, is next in 09.
