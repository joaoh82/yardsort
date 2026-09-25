# 12 — Agent events, stage 2: Codex

Status: **shipped** · 25 September 2026 · The second native adapter of
[09-agent-events-and-memory](09-agent-events-and-memory.md), after
[Claude Code](11-agent-events-stage-2-claude.md). Open question 20 said "Codex second, choosing
between its trust-gated hooks and its session files once samples exist"; the samples chose
**neither alone**: Codex's `notify` program as the trigger, its session file for the substance.

## 1 · What was recorded first

`scripts/record-codex.sh` runs the _installed_ `codex exec` in a throwaway repository under a
throwaway `CODEX_HOME` — the real one's `auth.json` symlinked in, nothing else — so the session it
writes lands there and not in the user's history, and a `hooks.json` there can point every hook
event at a capture script without touching the user's. It writes, redacted, under
`crates/core/fixtures/codex/<version>/`: the session's **rollout file** (`rollout.jsonl`, with
the model's instructions and reasoning stripped), the **hook payloads** (`hooks/`), and the
argument the **`notify`** program was given (`notify/`). The adapter's tests read them.

Recorded 2026-09-25 against **Codex CLI 0.156.1** (`gpt-6-astra`), one `exec` turn that used
`apply_patch`, three shell commands (one failing) and a reply.

| Surface                     | Seen                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Hooks** (12 payloads)     | Claude Code's shape and field names, to the letter: `SessionStart`, `UserPromptSubmit`, `PreToolUse` / `PostToolUse` ×4 (`tool_name: "Bash"`, `tool_input.command`, `tool_use_id`, `tool_response`), `Stop`, `SessionEnd`; plus `model`, `turn_id`, `permission_mode`. `YARDSORT_*` inherited. `PostToolUseFailure` did not fire for the failing command. Delivered **only** under `--dangerously-bypass-hook-trust`.                                                                                                                          |
| **`notify`** (1 call)       | `{"type":"agent-turn-complete","thread-id","turn-id","cwd","client":"codex_exec","input-messages":[…],"last-assistant-message"}` as the program's last argument; spawned as an argument list, no shell; `YARDSORT_*` and `CODEX_HOME` inherited.                                                                                                                                                                                                                                                                                               |
| **Session file** (25 lines) | `session_meta` (thread id, `cwd`, `cli_version`, `source`, `originator`); per turn `turn_context` (`model`, `effort`, `cwd`), `task_started` / `task_complete` (`duration_ms`, `time_to_first_token_ms`), `token_usage_record` (`turn_token_usage`: input, cached, output, reasoning, total); `event_msg item_completed` for `UserMessage`, `AgentMessage`, `CommandExecution` (`command`, `exit_code`, `status`, `duration`), `FileChange` (`changes`: path → `type`, `content`), `McpToolCall`. Every line timestamped and ordinal-numbered. |

And by experiment, not by reading: hooks given inline through `-c hooks.<Event>=[…]` are a
recognised config key (`--strict-config` accepts them) and are loaded — but **skipped in silence**
unless trusted. Trust is per hook definition, a hash Codex writes to the user's `config.toml`
(`[hooks.state."<file>:<event>:<n>:<m>"] trusted_hash = "sha256:…"`) once the user reviews it in
Codex's own `/hooks` UI. There is no non-interactive way to grant it, and the flag that bypasses
it bypasses it for every hook, including a repository's own `.codex/hooks.json`.

## 2 · Decisions

| Question            | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Hooks, or not?**  | **Not.** A hook Yardsort gives Codex per launch is untrusted until the user reviews it, and passing `--dangerously-bypass-hook-trust` on their behalf would run a cloned repository's hooks unreviewed too. If Codex ever lets a launch pre-trust a single hook, the adapter is a normalizer away: the payloads are Claude Code's.                                                                                                                                                                                      |
| **The trigger**     | **`notify`**, given per launch as `-c notify=[<exe>, "--yardsort-hook", "codex", <inbox>]`. Argument list, no shell, no trust review, environment inherited, nothing of the user's edited. A `notify` of the user's own in `config.toml` is chained after ours (`--then <their program> …`), with the same payload, so it still runs. A harness whose arguments already set `notify` is left alone (`hooks_not_armed`). Codex marks `notify` as legacy in its source; if it goes, the hooks path above is the fallback. |
| **The substance**   | **The session file**, read at drain time, not by the hook. The trigger names the thread and turn; the drain finds `<CODEX_HOME>/sessions/<y>/<m>/<d>/rollout-*-<thread>[_<fork>].jsonl` and reads that turn's lines. Cursors were not needed: a turn is keyed, and every derived event has its own source key, so re-reading is free of duplicates.                                                                                                                                                                     |
| **Not yet written** | `notify` may fire before the file has the turn's `task_complete`. A trigger whose turn is not ended is left in the inbox for the next drain, up to five minutes; then, or if the file cannot be found or read, the turn is recorded from the trigger alone (`turn.completed`, method `notify`) and counted (`codex_turn_incomplete`, `codex_session_file`).                                                                                                                                                             |
| **Correlation**     | `YARDSORT_RUN_ID` from the hook's environment — verified to reach `notify` — then the workspace id from the same environment. Never the `cwd` the payload carries. The thread id is a fallback key only where a run recorded it, which for Codex is never today (it chooses its own and prints none to the TUI's parent); recording it from the first trigger, so Resume can use it, is a follow-up. The `session.started` event's key includes the run: a resumed thread says so once per run.                         |
| **Bounds**          | Session file read whole (they are megabytes), refused above 64 MiB. Old rollouts are compressed to `.zst` after a week; a thread that old is not one a live turn belongs to.                                                                                                                                                                                                                                                                                                                                            |
| **Privacy**         | From the payload: ids only — the messages it carries are dropped in the hook. From the file: `tool: "shell"` with exit code, status and duration, never the command; a file change's relative path and kind (`add`/`update`/`delete`), never content; the prompt's character count; token counts; durations. Agent messages and reasoning are not read.                                                                                                                                                                 |
| **Version**         | No probe. The fixture version is named in the code and the guide; an unknown item type is skipped, an unknown line ignored.                                                                                                                                                                                                                                                                                                                                                                                             |

## 3 · Event mapping

Producer `codex`; method `session_file` (or `notify` for a fallback); fidelity `reported`;
`occurred_at` from the file's own line timestamps.

| Source line                                   | Kind                                                | Payload                                                                                    | Key                              |
| --------------------------------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------ | -------------------------------- |
| `session_meta`                                | `session.started`                                   | `threadId`, `cliVersion`, `source`, `originator`                                           | `codex:<thread>:<run>:session`   |
| `item_completed` / `UserMessage`              | `prompt.submitted`                                  | `turnId`, `chars`                                                                          | `codex:<thread>:<item>:prompt`   |
| `item_completed` / `CommandExecution`         | `tool.completed` / `tool.failed` (`status: failed`) | `tool: "shell"`, `toolUseId`, `exitCode`, `status`, `durationMs`                           | `codex:<thread>:<item>:tool`     |
| `item_completed` / `FileChange`, one per path | `file.reported_write`                               | `path` (relative) or `pathOutsideWorkspace`, `kind`, `status`, `toolUseId`                 | `codex:<thread>:<item>:file:<n>` |
| `item_completed` / `McpToolCall`              | `tool.completed` / `tool.failed`                    | `tool: "mcp:<server>/<tool>"`, `status`                                                    | `codex:<thread>:<item>:tool`     |
| `token_usage_record` (the turn's last)        | `usage.reported`                                    | `inputTokens`, `cachedInputTokens`, `outputTokens`, `reasoningOutputTokens`, `totalTokens` | `codex:<thread>:<turn>:usage`    |
| `event_msg` / `task_complete`                 | `turn.completed`                                    | `turnId`, `model`, `durationMs`, `timeToFirstTokenMs`                                      | `codex:<thread>:<turn>:turn`     |
| `event_msg` / `turn_aborted`                  | `turn.failed`                                       | `turnId`, `reason`                                                                         | `codex:<thread>:<turn>:turn`     |
| the trigger alone                             | `turn.completed`                                    | `threadId`, `turnId`, `detail: "notify"`                                                   | `codex:<thread>:<turn>:turn`     |

`usage.reported` is the first token accounting on the timeline; Claude Code's hooks have none.

## 4 · Coverage, honestly

| Harness               | Lifecycle | Native                                                                                                                                                                                                                                                        |
| --------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280   | yes       | Hooks, opt-in: [11](11-agent-events-stage-2-claude.md).                                                                                                                                                                                                       |
| Codex 0.156.1         | yes       | **`notify` + session file, opt-in**: turns, prompts, shell commands with exit codes and durations, file changes with paths, MCP calls, token usage. **Not available**: anything mid-turn (the file is read at the turn's end), permission prompts, subagents. |
| OpenCode              | yes       | Not started.                                                                                                                                                                                                                                                  |
| Grok, OMP, Cursor, Pi | yes       | Lifecycle only.                                                                                                                                                                                                                                               |

## 5 · What shipped

- `crates/core/src/activity/codex.rs`: `arm()`, `users_notify()`, `normalize_notify()`,
  `find_rollout()`, `expand()`; tests over the fixtures and the arming rules.
- `hook.rs`: the payload as the last argument, `--then` chaining, the Codex branch.
- `activity::import_inbox`: a Codex trigger is expanded into the turn's events; not-yet and
  cannot-read handling; `workspace_relative` shared with the Claude adapter.
- `Launcher`: arms a recorded `codex` launch when `capture_codex` is on; `capture: "notify"`.
- Settings → General: **Capture what Codex reports**; `[activity] capture_codex`.
- Timeline words for `file.reported_write`, `usage.reported`, exit codes and turn durations;
  `ys activity list` details; the `ys` binary end to end.
- `scripts/record-codex.sh`, `crates/core/fixtures/codex/0.156.1/`.

## 6 · Verification

| Case                                                                                                                                                                                                                                                                                                                                                                                  | Where                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| The `notify` payload becomes a trigger with ids only                                                                                                                                                                                                                                                                                                                                  | `codex::tests::the_notify_payload_becomes_a_trigger…`, `hook::tests::a_codex_notify_payload…`                                |
| The fixture turn expands to the expected events, times from the file, nothing leaked                                                                                                                                                                                                                                                                                                  | `codex::tests::a_turn_is_read_from_the_session_file…`                                                                        |
| An unfinished turn waits; an unknown thread errors; the fallback                                                                                                                                                                                                                                                                                                                      | `codex::tests::a_turn_the_file_has_not_finished…`                                                                            |
| The session file is found by thread id, fork suffix included                                                                                                                                                                                                                                                                                                                          | `codex::tests::the_session_file_is_found…`                                                                                   |
| Arming: argv, chained `notify`, refusal; `CODEX_HOME` resolution; timestamps                                                                                                                                                                                                                                                                                                          | `codex::tests::arming_adds_notify…`, `…codex_home_follows…`, `…timestamps_parse…`                                            |
| Drain: expansion linked to the run, drained twice is one row per fact; waits then falls back                                                                                                                                                                                                                                                                                          | `activity::tests::a_codex_turn_trigger_is_expanded…`, `…the_file_has_not_finished_waits…`                                    |
| Only `codex`, only with its switch on                                                                                                                                                                                                                                                                                                                                                 | `launch::tests::a_codex_launch_is_given_notify…`                                                                             |
| The real `ys` binary as `notify`, then `ys activity list` reading a fixture session file                                                                                                                                                                                                                                                                                              | `cli::the_binary_as_codex_notify…` (Linux, macOS, Windows in CI)                                                             |
| Settings switch; timeline words                                                                                                                                                                                                                                                                                                                                                       | `ActivitySettings.test.tsx`, `ActivityPanel.test.tsx`                                                                        |
| By hand: a real Codex, launched through `ys` on Linux, its turn read from its own session file — `session.started`, `prompt.submitted`, `file.reported_write`, `tool.completed`, `turn.completed`, `usage.reported` — and the bug that found: `-c notify=…` appended after the `--` before the prompt was a usage error (exit 2); `arm()` now inserts it before the `--`, with a test | [08 §13](08-manual-checklist.md#13--codex-reporting) has the app-window rows; Linux via `ys` done, macOS and Windows pending |

## 7 · Risks and the next slice

- **`notify` is legacy** in Codex's source. When it goes, the hooks path is the replacement and
  needs either a way to trust one hook per launch or the user trusting Yardsort's hook once in
  `/hooks`; the payloads are already understood.
- **Codex's automatic reviewer** (`approvals_reviewer = "auto_review"`) runs as a second thread, and `notify` fires for its turns too, with a thread id that has no session file of its own at that moment. Seen live: it lands as the fallback `turn.completed` (method `notify`) under the same run. Honest, if terse; a later slice could name it a review.
- **Interactive mode** was recorded through `exec` only; the session file format is the same
  writer, and the hands-on row confirms the TUI. `turn_aborted` and `McpToolCall` are mapped from
  the survey of existing local sessions, not from the fixture.
- **The session file is undocumented.** Field names are those of 0.156.1; the mapping skips what
  it does not recognise rather than failing, and the fixture pins what is relied on.
- **Next:** OpenCode's plugin, or the timeline's use of what is now there: `agent.notified` for
  Claude Code in the sidebar, and a per-run token total from `usage.reported`.
