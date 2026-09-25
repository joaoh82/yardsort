# 14 — Agent events, stage 2: Grok

Status: **shipped** · 25 September 2026 · A fourth native adapter, past the three the plan
named. [09](09-agent-events-and-memory.md) expected Grok to stay lifecycle-only until a stable
surface turned up; [10 §6](10-agent-events-stage-1.md#6--stage-0-baseline-and-the-stage-2-coverage-matrix)
saw only a `plugin` subcommand. What turned up is the lightest source of the four: Grok keeps
a metadata-only event log of its own for every session, and Yardsort chooses Grok's session id,
so it knows where that log is. Nothing is given to Grok, nothing is installed, nothing in the
user's home is written.

## 1 · What was recorded first

`scripts/record-grok.sh` runs the _installed_ `grok -p` in a throwaway repository under a
throwaway `GROK_HOME` (the real one's `auth.json` symlinked in, nothing else), with the session
id chosen up front as Yardsort does, and a hooks file there pointing every hook event at a
capture script. It writes, redacted, under `crates/core/fixtures/grok/<version>/`: the session
directory's `events.jsonl`, `usage.json` and `summary.json` — what the adapter reads — and the
hook payloads under `hooks/`, kept as the record of the alternative. Lines naming the machine's
MCP servers are dropped from the event log; the summary is stripped of remotes, ids and the
session's own summary text.

Recorded 2026-09-25 against **Grok 1.0.41** (`grok-4.7`), one headless turn with a `write`, a
`read_file`, a `read_file` of a missing file and a `run_terminal_command`.

| Surface                            | Seen                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sessions/<url-encoded cwd>/<id>/` | `events.jsonl`, `usage.json`, `summary.json`, plus `chat_history.jsonl`, `updates.jsonl`, `system_prompt.txt`, `prompt_context.json`, `rewind_points.jsonl`, `signals.json`, `tool_definitions.json` and more. Only the first three are read.                                                                                                                                                                                                                                                                                                                             |
| `events.jsonl` (140 lines)         | `{ts, type, …}` per line, ISO timestamps: `turn_started {session_id, turn_number, model_id, yolo_mode, session_relationship}`, `turn_ended {outcome: completed / interrupted}`, `tool_started {tool_name}`, `tool_completed {tool_name, tool_call_id, duration_ms, outcome: success / error}`, `permission_requested {tool_name}`, `permission_resolved {tool_name, decision, wait_ms}`, `first_token`, `loop_started`, `phase_changed` (the bulk, streaming phases), `mcp_*` (server names). **No command, path, prompt or output anywhere.**                            |
| `usage.json`                       | `{sessionId, updatedAt, session {…}, turns: [{turnNumber, endedAt, inputTokens, outputTokens, cachedReadTokens, cacheCreationTokens, reasoningTokens, totalTokens, modelCalls, costUsdTicks, primaryModelId, modelUsage}]}`.                                                                                                                                                                                                                                                                                                                                              |
| `summary.json`                     | `created_at`, `current_model_id`, `reasoning_effort`, `agent_name`, counts, `git_root_dir`, `grok_home`, and the machine's git remotes and the session's summaries, which are not read.                                                                                                                                                                                                                                                                                                                                                                                   |
| Hooks (13 payloads)                | Claude Code's shape, camelCase keys with snake_case aliases (`hookEventName`/`hook_event_name`, `sessionId`, `cwd`, `workspaceRoot`, `timestamp`, `promptId`, `permissionMode`, `toolName`, `toolUseId`, `toolInput`, `transcriptPath`); `SessionStart {source: new}`, `UserPromptSubmit {prompt}`, `PreToolUse`/`PostToolUse` ×4, `Stop {reason: end_turn}` ×2, `SessionEnd {reason: shutdown}`. `YARDSORT_*` inherited, plus `GROK_SESSION_ID`, `GROK_HOOK_EVENT`, `GROK_WORKSPACE_ROOT`, `CLAUDE_PROJECT_DIR`. `PostToolUseFailure` did not fire for the failing read. |

And from Grok's own shipped user guide (`~/.grok/docs/user-guide/`), which is more complete than
the web docs: hooks load from `$GROK_HOME/hooks/*.json` (always trusted), from `config.toml`
tables, from a project's `.grok/hooks/` (folder trust required) and from plugins; the
`GROK_CONFIG` / `GROK_CONFIG_PATH` overlay is confined to an allowlist of soft settings and
"cannot spawn commands … or add a discovery source"; `--plugin-dir`, the one per-process way to
add trusted hooks, exists only on `grok agent … stdio`, not on the TUI Yardsort runs.

## 2 · Decisions

| Question                       | Decision                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Hooks, or not?**             | **Not.** They would work — the recording shows the payloads, the environment reaching them, and that a file under `~/.grok/hooks/` needs no trust — but there is no per-launch way to give them to the TUI, so using them would mean writing a file of ours into the user's home, one that fires for every Grok session on the machine. The session directory says the same things without that.                        |
| **The source**                 | **The session directory**, read as it grows. `events.jsonl` is metadata by Grok's own design: it never held a command, a path or a message, so the privacy rule is met before any code runs. `usage.json` gives tokens and cost per turn without spawning `grok usage`. `summary.json` gives the session's start, model and effort.                                                                                     |
| **Finding it**                 | By the session id alone: `sessions/*/<id>/`. Yardsort chose the id (`--session-id`, `session_id_mode: Assigned`) and recorded it on the run; the directory's own encoding of the working directory is Grok's business.                                                                                                                                                                                                  |
| **When to read**               | Whenever the drain runs: at start, on every exit, and — the part the inbox never needed — on a five-second tick while any Grok run is going. The app's inbox watcher keeps a cursor per run (lines already read) across ticks; `ys` reads the whole log each time and lets the unique source keys make it duplicates. A run that ended is read for ten more minutes, for the lines that land after the process is gone. |
| **Which runs**                 | Recorded `grok` runs, while `capture_grok` is on. The launcher gives Grok nothing; it marks the run's start with `capture: "session_file"` so a reader knows the files will be read.                                                                                                                                                                                                                                    |
| **What is a permission event** | `permission_resolved` with `wait_ms > 0` or a decision other than `allow`: the user was really asked, and how long they took, or they refused. The many resolved at once by Grok itself — every tool call has one — are not events to a reader. `permission_requested` is never recorded; the resolution says everything.                                                                                               |
| **Keys**                       | `grok:<session>:<line>:<kind>` for the log, `grok:<session>:usage:<turn>` for usage, `grok:<session>:<run>:session` for the start — a resumed session says so once per run.                                                                                                                                                                                                                                             |
| **Cost**                       | `costUsdTicks` is kept as Grok writes it; its unit is not documented, and the guide says so rather than guess a conversion.                                                                                                                                                                                                                                                                                             |
| **Bounds**                     | A log over 64 MiB is not read. A session that ended more than ten minutes ago is not looked at again.                                                                                                                                                                                                                                                                                                                   |

## 3 · Event mapping

Producer `grok`, method `session_file`, fidelity `reported`; `occurred_at` from the files' own
timestamps.

| Source line                                    | Kind                             | Payload                                                                                                                                                                          |
| ---------------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `summary.json`                                 | `session.started`                | `sessionId`, `model`, `reasoningEffort`, `agent`                                                                                                                                 |
| `turn_started`                                 | `turn.started`                   | `sessionId`, `turnNumber`, `model`, `relationship`                                                                                                                               |
| `turn_ended`                                   | `turn.completed` / `turn.failed` | `sessionId`, `turnNumber`, `outcome`                                                                                                                                             |
| `tool_started`                                 | `tool.started`                   | `tool`, `sessionId`, `turnNumber`                                                                                                                                                |
| `tool_completed`                               | `tool.completed` / `tool.failed` | `tool`, `toolUseId`, `sessionId`, `turnNumber`, `durationMs`, `outcome`                                                                                                          |
| `permission_resolved` (waited, or not allowed) | `approval.resolved`              | `tool`, `sessionId`, `turnNumber`, `decision`, `waitMs`                                                                                                                          |
| `usage.json` turns                             | `usage.reported`                 | `sessionId`, `turnNumber`, `model`, `inputTokens`, `outputTokens`, `reasoningOutputTokens`, `cachedInputTokens`, `cacheWriteTokens`, `totalTokens`, `modelCalls`, `costUsdTicks` |

`first_token`, `loop_started`, `phase_changed`, `yolo_toggled` and the `mcp_*` lines are skipped.
`turn.started` is a new kind: Grok's log knows a turn began but not what was typed, and a reader
should not be told "prompt submitted" by a source that did not see the prompt.

## 4 · Coverage, honestly

| Harness             | Lifecycle | Native                                                                                                                                                                                                                                                                    |
| ------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280 | yes       | Hooks, opt-in: [11](11-agent-events-stage-2-claude.md).                                                                                                                                                                                                                   |
| Codex 0.156.1       | yes       | `notify` + session file, opt-in: [12](12-agent-events-stage-2-codex.md).                                                                                                                                                                                                  |
| OpenCode 1.18.31    | yes       | Plugin, opt-in: [13](13-agent-events-stage-2-opencode.md).                                                                                                                                                                                                                |
| Grok 1.0.41         | yes       | **Session directory, opt-in**: turns with model and outcome, tools with durations and outcomes, permissions the user really answered with how long they took, tokens and cost per turn. **Not available**: file paths, prompt sizes, subagents (the log is the parent's). |
| OMP, Cursor, Pi     | yes       | Lifecycle only. Four patterns now exist — a settings file, a program, a plugin, a directory read — for whichever grows a stable surface.                                                                                                                                  |

## 5 · What shipped

- `crates/core/src/activity/grok.rs`: `grok_home()`, `find_session_dir()`, `read_events()` with
  a cursor, `read_usage()`, `read_summary()`, `import()`; `Store::live_runs`; `iso_to_ms` moved
  to `activity` and widened to `+00:00` and long fractions.
- The app's inbox watcher reads Grok's directories on its tick, keeps cursors, and ticks while
  a Grok run is going; a drain from the exit path wakes it. `ys` reads with the spool.
- Settings → General **Read what Grok records**; `[activity] capture_grok`; `capture:
"session_file"` on the run's start; timeline words for `turn.started`, waits and outcomes.
- `scripts/record-grok.sh`, `crates/core/fixtures/grok/1.0.41/`.

## 6 · Verification

| Case                                                                                                                                                                                                                                 | Where                                                                                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| The log reads as tools, turns and real permissions; nothing leaked; a cursor reads only what is new                                                                                                                                  | `grok::tests::the_event_log_reads_as_tools_turns…`, `…a_permission_that_waited…`                                            |
| Usage per turn and the session's start, with the files' own times                                                                                                                                                                    | `grok::tests::usage_and_the_summary_read…`                                                                                  |
| The directory is found by id; `GROK_HOME` resolution                                                                                                                                                                                 | `grok::tests::the_session_directory_is_found…`                                                                              |
| A live run is read once, then only what is new; without a cursor it is duplicates; a run that just ended is read again, one long ended is not                                                                                        | `grok::tests::a_live_run_is_read_into_the_store…`                                                                           |
| Only `grok`, only with its switch, and nothing on the command line                                                                                                                                                                   | `launch::tests::a_grok_launch_is_marked…`                                                                                   |
| The real `ys` binary reading a fixture directory under a throwaway `GROK_HOME`                                                                                                                                                       | `cli::groks_session_directory_is_read…` (Linux, macOS, Windows in CI)                                                       |
| The watcher ticks while a run is followed; settings switch; timeline words                                                                                                                                                           | `activity.rs` unit test, `ActivitySettings.test.tsx`, `ActivityPanel.test.tsx`                                              |
| By hand: a real Grok, launched through `ys` on Linux, its turn read from its own directory — session, turn start, `write`, the failed `read_file`, `run_terminal_command`, the turn's end and its usage — with nothing given to Grok | [08 §15](08-manual-checklist.md#15--grok-reporting) has the app-window rows; Linux via `ys` done, macOS and Windows pending |

## 7 · Risks and the next slice

- **`events.jsonl` is not in Grok's documented file list** (the guide names `updates.jsonl`,
  `signals.json`, `summary.json` and others); it is observed in 1.0.41 across every local session.
  If it goes, `usage.json` and `summary.json` still carry usage and the start, and the hooks
  remain the fallback with an install step.
- **Latency is the tick**: up to five seconds, and the last lines of a turn can land after the
  process exits, which the ten-minute grace covers.
- **The log is the parent session's**; a subagent's own session has its own directory, not
  followed.
- **Next:** with four sources in, use them — a per-run token total and a permission-wait total on
  the workspace row, and the sidebar badge saying what the agent is waiting for.
