# 16 — Agent events, stage 2: Cursor

Status: **shipped, unrecorded** · 25 September 2026 · The last built-in harness without a
native source. Cursor's agent CLI (`cursor-agent`) has hooks in Claude Code's image — commands
named per event in a `hooks.json`, given the event as JSON on stdin — and a per-process way to
add them that needs no file of ours in the user's home: `--plugin-dir <dir>` loads a plugin for
that one process, merged with the user's own hooks, with no trust prompt. This slice is
[11](11-agent-events-stage-2-claude.md) with a directory in place of a settings file.

**What is different about this note:** the adapter was built against Cursor's documentation,
not a recording. The Cursor agent on the machine this was written on was not logged in, and a
hook fixture cannot be made without a turn. `scripts/record-cursor.sh` is ready; the day it runs,
its fixtures replace the documented shapes in `activity/cursor.rs`'s tests, and this note is
updated with what was actually seen. Until then the mapping is honest about its source, and a
payload whose shape differs from the documented one loses the fields that differ, never the
launch.

## 1 · What was read, and what could not be recorded

`cursor-agent 2026.09.23-86fc751` (`agent --help`, `agent --version`, and the hooks reference at
`cursor.com/docs/hooks`).

| Surface              | Documented                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Hook locations       | `~/.cursor/hooks.json` (user), `<project>/.cursor/hooks.json` (project), and plugins: a directory holding `.cursor-plugin/plugin.json` (`name`, optional `hooks` path) and `hooks/hooks.json`. Every location's hooks run; there is no override.                                                                                                                                                                                                                                                                                                                                                                                                       |
| `--plugin-dir <dir>` | Loads one plugin directory for that process. Repeatable. No trust prompt in the CLI.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `hooks.json`         | `{"version": 1, "hooks": {"<event>": [{"type": "command", "command": "<shell string>", "timeout": <seconds>}]}}`. **The command is a shell string**: `$SHELL -c` on POSIX, PowerShell on Windows, where Cursor prefixes `& ` when the command starts with a quote — so a single-quoted executable path reads the same on all three.                                                                                                                                                                                                                                                                                                                    |
| Payload, every event | `conversation_id`, `generation_id`, `model`, `hook_event_name`, `cursor_version`, `workspace_roots[]`, `transcript_path`, on stdin.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Passive events       | `sessionStart {session_id, composer_mode, is_background_agent}`, `sessionEnd {reason, duration_ms, final_status}`, `postToolUse {tool_name, tool_input, tool_output, tool_use_id, cwd, duration}`, `postToolUseFailure`, `afterShellExecution {command, output, duration, sandbox}`, `afterMCPExecution {tool_name, mcp_server_name, …}`, `afterFileEdit {file_path, edits[]}`, `subagentStart`, `subagentStop {subagent_type, status, task, summary, duration_ms}`, `stop {status, loop_count, tokens}`, `afterAgentResponse {text, tokens}`, `afterAgentThought`, `preCompact`, `beforeSubmitPrompt {prompt, attachments}` (its answer is optional). |
| Deciding events      | `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `beforeTabFileRead`: their stdout is read as a permission decision.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Known gap            | In this build, `stop`, `afterAgentResponse` and `afterAgentThought` fire for hooks from the user's and the project's files but **not from a plugin's** (noted in the research for this slice, not verified here). Subscribed to all the same, for the build where they do.                                                                                                                                                                                                                                                                                                                                                                             |
| Environment          | The documentation does not say whether hooks inherit the agent's environment. The recording script prints whether `YARDSORT_*` reached them.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |

## 2 · Decisions

| Question           | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **The channel**    | **`--plugin-dir <data-dir>/activity/hooks/cursor-plugin`**, inserted before any `--`. The directory is written before each launch: `.cursor-plugin/plugin.json` naming `yardsort-activity`, and `hooks/hooks.json` whose every command is `'<exe>' --yardsort-hook cursor '<inbox>'`, ten-second timeout. Nothing in `~/.cursor` is touched, and the user's own hooks keep running.                                                                                                                              |
| **A shell string** | The one place in the tree where a program is named in a shell string rather than an argument list, because that is the only form Cursor takes. The two paths are single-quoted with `'` doubled inside, which `sh -c` and PowerShell both read literally, and nothing else in the string is variable.                                                                                                                                                                                                            |
| **Which events**   | The passive ones and `beforeSubmitPrompt`, whose answer is optional. **Never a deciding event**: a deciding hook that printed nothing might be read as an invalid answer and stop the action, and Yardsort observes. The hook mode prints nothing on stdout for Cursor, as for every harness.                                                                                                                                                                                                                    |
| **Correlation**    | By the launch environment alone — `YARDSORT_RUN_ID` and the rest, if the hook inherits them. The payload's `conversation_id` is kept as the native session id but Yardsort did not choose it (`--new-session-id` exists but is undocumented), so it links nothing by itself. **`workspace_roots` is never a key**: a path is not an identity ([10 §4](10-agent-events-stage-1.md)). If the environment turns out not to reach the hooks, the entries land unlinked and counted, and the fix is a launch-side id. |
| **Metadata only**  | The prompt becomes `chars`; `tool_input` yields only a `file_path` / `path`, made workspace-relative; `tool_output`, `command`, `output`, `edits[].old_string / new_string`, `text`, `task`, `summary` and `transcript_path` are never read.                                                                                                                                                                                                                                                                     |

## 3 · Event mapping

Producer `cursor`, method `hook`, fidelity `reported`; `occurred_at` is the hook's clock.

| Hook event            | Kind                             | Payload                                                                                                                         |
| --------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `sessionStart`        | `session.started`                | `sessionId`, `model`, `composerMode`, `background`                                                                              |
| `sessionEnd`          | `session.ended`                  | `sessionId`, `reason`, `durationMs`, `status`                                                                                   |
| `beforeSubmitPrompt`  | `prompt.submitted`               | `conversationId`, `generationId`, `chars`, `attachments` (a count)                                                              |
| `postToolUse`         | `tool.completed`                 | `tool`, `toolUseId`, `conversationId`, `generationId`, `durationMs`, `path` (workspace-relative), `pathOutsideWorkspace`        |
| `postToolUseFailure`  | `tool.failed`                    | the same                                                                                                                        |
| `afterShellExecution` | `tool.completed`                 | `tool: "shell"`, `conversationId`, `generationId`, `durationMs`, `sandbox`                                                      |
| `afterMCPExecution`   | `tool.completed`                 | `tool: "mcp:<name>"`, `server`, ids, `durationMs`                                                                               |
| `afterFileEdit`       | `file.reported_write`            | `path`, `kind: "changed"`, `edits` (a count), ids                                                                               |
| `subagentStart`       | `agent.subagent_started`         | `agentId`, `agentType`, `conversationId`, `model`                                                                               |
| `subagentStop`        | `agent.subagent_stopped`         | `agentType`, `conversationId`, `status`, `durationMs`                                                                           |
| `stop`                | `turn.completed` / `turn.failed` | `conversationId`, `generationId`, `status`, `loopCount`, `inputTokens`, `outputTokens`, `cachedInputTokens`, `cacheWriteTokens` |
| `afterAgentResponse`  | `usage.reported`                 | ids, `model`, `chars`, the same token fields                                                                                    |

`preCompact` and `afterAgentThought` are not subscribed to: a compaction has no metadata worth
a row, and a thought is content.

## 4 · Coverage, honestly

| Harness                   | Lifecycle | Native                                                                                                                                                                                                                                                                                                                            |
| ------------------------- | --------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280       | yes       | Hooks, opt-in: [11](11-agent-events-stage-2-claude.md).                                                                                                                                                                                                                                                                           |
| Codex 0.156.1             | yes       | `notify` + session file, opt-in: [12](12-agent-events-stage-2-codex.md).                                                                                                                                                                                                                                                          |
| OpenCode 1.18.31          | yes       | Plugin, opt-in: [13](13-agent-events-stage-2-opencode.md).                                                                                                                                                                                                                                                                        |
| Grok 1.0.41               | yes       | Session directory, opt-in: [14](14-agent-events-stage-2-grok.md).                                                                                                                                                                                                                                                                 |
| OMP 18.2.11, Pi 0.87.1    | yes       | Extension, opt-in: [15](15-agent-events-stage-2-pi-omp.md).                                                                                                                                                                                                                                                                       |
| Cursor 2026.09.23-86fc751 | yes       | **Hooks through a per-launch plugin, opt-in, built from the documentation**: sessions, prompt sizes, tools with paths and durations, shell and MCP calls, file edits, subagents, and — where the build fires them for plugins — turns with tokens. **Unverified live** until `cursor-agent login` and `scripts/record-cursor.sh`. |

Every built-in harness now has a native source. What stays lifecycle-only is a custom harness.

## 5 · What shipped

- `crates/core/src/activity/cursor.rs`: `plugin_dir`, `shell_word`, `hooks_json`, `arm`,
  `normalize`; the hook mode accepts `cursor`; the launcher arms it when **Capture what Cursor
  reports** is on, with `capture: "hook"` on the run's start.
- Settings → General **Capture what Cursor reports**; `[activity] capture_cursor`.
- `scripts/record-cursor.sh`: a capture plugin subscribing to every documented event, deciding
  ones included (they answer nothing), and a note of whether the environment reached the hooks.

## 6 · Verification

- Rust: `activity::cursor` tests map every documented shape and check that the prompt, the
  tool's output, the command, the edit's text, the reply, the subagent's task and summary and
  the transcript path never reach a payload; that paths come out workspace-relative or marked
  outside; that arming writes the manifest and a `hooks.json` naming this executable for the
  passive events and none of the deciding ones, before `--`. `launch::tests` arms it through a
  plan-catching host only when its switch is on. `crates/cli/tests/cli.rs` runs the real `ys`
  as the hook with a documented `afterFileEdit` on stdin and lists `file.reported_write`,
  `cursor/hook`, the path, and not the edit's text.
- By hand: owed. See [08 §17](08-manual-checklist.md#17--cursor-reporting), which starts with
  the recording.

## 7 · Risks and the next slice

- **Unrecorded.** A field named differently from the documentation is a missing field on the
  timeline, and a hook event the build does not fire for plugins is a missing row; neither
  touches the launch. The first recording will say which.
- **Environment.** If hooks do not inherit the launch environment, every Cursor entry is
  unlinked; the fix is a session id Yardsort chooses (`--new-session-id`, if it is what
  `conversation_id` reports), recorded on the run as Grok's is.
- **The shell string.** A data directory containing a single quote is handled; one containing a
  newline is not tested.
- The next slice is not another adapter: stage 3 of [09](09-agent-events-and-memory.md).
