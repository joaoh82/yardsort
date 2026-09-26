# 16 — Agent events, stage 2: Cursor

Status: **shipped, recorded 2026-09-26** · 25 September 2026 · The last built-in harness without a
native source. Cursor's agent CLI (`cursor-agent`) has hooks in Claude Code's image — commands
named per event in a `hooks.json`, given the event as JSON on stdin — and a per-process way to
add them that needs no file of ours in the user's home: `--plugin-dir <dir>` loads a plugin for
that one process, merged with the user's own hooks, with no trust prompt. This slice is
[11](11-agent-events-stage-2-claude.md) with a directory in place of a settings file.

**How this note came about:** the adapter was built against Cursor's documentation, because
the Cursor agent on the machine it was written on was not logged in. The recording came the
next day (`scripts/record-cursor.sh`, `crates/core/fixtures/cursor/2026.09.23-86fc751/`, 19
hooks over one headless turn) and its fixtures now drive `activity/cursor.rs`'s tests; the
documented shapes remain only for the hooks the recording did not deliver. What the recording
added, in §8.

## 1 · What was read, and what could not be recorded

`cursor-agent 2026.09.23-86fc751` (`agent --help`, `agent --version`, and the hooks reference at
`cursor.com/docs/hooks`).

| Surface              | Documented                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hook locations       | `~/.cursor/hooks.json` (user), `<project>/.cursor/hooks.json` (project), and plugins: a directory holding `.cursor-plugin/plugin.json` (`name`, optional `hooks` path) and `hooks/hooks.json`. Every location's hooks run; there is no override.                                                                                                                                                                                                                                                                                                                 |
| `--plugin-dir <dir>` | Loads one plugin directory for that process. Repeatable. No trust prompt in the CLI.                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `hooks.json`         | `{"version": 1, "hooks": {"<event>": [{"type": "command", "command": "<shell string>", "timeout": <seconds>}]}}`. **The command is a shell string**: `$SHELL -c` on POSIX, PowerShell on Windows, where Cursor prefixes `& ` when the command starts with a quote — so a single-quoted path reads the same on all three, as long as an apostrophe inside it is doubled for PowerShell and closed-and-reopened for POSIX.                                                                                                                                         |
| Payload, every event | `conversation_id`, `generation_id`, `model`, `hook_event_name`, `cursor_version`, `workspace_roots[]`, `transcript_path`, on stdin.                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Passive events       | `sessionStart {session_id, composer_mode, is_background_agent}`, `sessionEnd {reason, duration_ms, final_status}`, `postToolUse {tool_name, tool_input, tool_output, tool_use_id, cwd, duration}`, `postToolUseFailure`, `afterShellExecution {command, output, duration, sandbox}`, `afterMCPExecution {tool_name, mcp_server_name, …}`, `afterFileEdit {file_path, edits[]}`, `subagentStop {subagent_type, status, task, summary, duration_ms}`, `stop {status, loop_count, tokens}`, `afterAgentResponse {text, tokens}`, `afterAgentThought`, `preCompact`. |
| Deciding events      | `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `beforeTabFileRead`, `subagentStart`: the documented permission hooks, where "invalid JSON or a response that doesn't match the hook's schema blocks the action" — an exit of 0 with nothing on stdout included. `beforeSubmitPrompt {prompt, attachments}` answers with `continue`; what its silence means is not documented.                                                                                                                                                     |
| Known gap            | In this build, `stop`, `afterAgentResponse` and `afterAgentThought` fire for hooks from the user's and the project's files but **not from a plugin's** (noted in the research for this slice, not verified here). Subscribed to all the same, for the build where they do.                                                                                                                                                                                                                                                                                       |
| Environment          | The documentation does not say whether hooks inherit the agent's environment. The recording script prints whether `YARDSORT_*` reached them.                                                                                                                                                                                                                                                                                                                                                                                                                     |

## 2 · Decisions

| Question           | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **The channel**    | **`--plugin-dir <data-dir>/activity/hooks/cursor-plugin`**, inserted before any `--`. The directory is written before each launch: `.cursor-plugin/plugin.json` naming `yardsort-activity`, and `hooks/hooks.json` whose every command is `'<exe>' --yardsort-hook cursor '<inbox>'`, ten-second timeout. Nothing in `~/.cursor` is touched, and the user's own hooks keep running.                                                                                                                                                                                      |
| **A shell string** | The one place in the tree where a program is named in a shell string rather than an argument list, because that is the only form Cursor takes. The two paths are single-quoted, which `sh -c` and PowerShell both read literally; an apostrophe inside is doubled for PowerShell and closed-and-reopened (`'\''`) for POSIX, chosen by the platform the launching binary runs on, and a test runs the real shell on each. Nothing else in the string is variable.                                                                                                        |
| **Which events**   | Only hooks Cursor documents as observation-only, or whose sole output is an optional follow-up message (`stop`, `subagentStop`). **Never one whose silence decides**: Yardsort's hook prints nothing on stdout, and for Cursor's permission hooks nothing _is_ a block — `subagentStart` among them, so subscribing to it would have stopped every subagent. `beforeSubmitPrompt` is left out too, since the docs do not say what its `continue` defaults to; the prompt's length is the price. `DECIDING` in `cursor.rs` lists them, and a test holds the plugin to it. |
| **Correlation**    | By the launch environment alone — `YARDSORT_RUN_ID` and the rest, if the hook inherits them. The payload's `conversation_id` is kept as the native session id but Yardsort did not choose it (`--new-session-id` exists but is undocumented), so it links nothing by itself. **`workspace_roots` is never a key**: a path is not an identity ([10 §4](10-agent-events-stage-1.md)). If the environment turns out not to reach the hooks, the entries land unlinked and counted, and the fix is a launch-side id.                                                         |
| **Metadata only**  | `tool_input` yields only a `file_path` / `path`, made workspace-relative; `tool_output`, `command`, `output`, `edits[].old_string / new_string`, `text`, `task`, `summary` and `transcript_path` are never read.                                                                                                                                                                                                                                                                                                                                                         |

## 3 · Event mapping

Producer `cursor`, method `hook`, fidelity `reported`; `occurred_at` is the hook's clock.

| Hook event            | Kind                             | Payload                                                                                                                         |
| --------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `sessionStart`        | `session.started`                | `sessionId`, `model`, `composerMode`, `background`                                                                              |
| `sessionEnd`          | `session.ended`                  | `sessionId`, `reason`, `durationMs`, `status`                                                                                   |
| `postToolUse`         | `tool.completed`                 | `tool`, `toolUseId`, `conversationId`, `generationId`, `durationMs`, `path` (workspace-relative), `pathOutsideWorkspace`        |
| `postToolUseFailure`  | `tool.failed`                    | the same                                                                                                                        |
| `afterShellExecution` | `tool.completed`                 | `tool: "shell"`, `conversationId`, `generationId`, `durationMs`, `sandbox`                                                      |
| `afterMCPExecution`   | `tool.completed`                 | `tool: "mcp:<name>"`, `server`, ids, `durationMs`                                                                               |
| `afterFileEdit`       | `file.reported_write`            | `path`, `kind: "changed"`, `edits` (a count), ids                                                                               |
| `subagentStop`        | `agent.subagent_stopped`         | `agentType`, `conversationId`, `status`, `durationMs`                                                                           |
| `stop`                | `turn.completed` / `turn.failed` | `conversationId`, `generationId`, `status`, `loopCount`, `inputTokens`, `outputTokens`, `cachedInputTokens`, `cacheWriteTokens` |
| `afterAgentResponse`  | `usage.reported`                 | ids, `model`, `chars`, the same token fields                                                                                    |

`preCompact` and `afterAgentThought` are not subscribed to: a compaction has no metadata worth
a row, and a thought is content.

## 4 · Coverage, honestly

| Harness                   | Lifecycle | Native                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------- | --------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280       | yes       | Hooks, opt-in: [11](11-agent-events-stage-2-claude.md).                                                                                                                                                                                                                                                                                                                                                               |
| Codex 0.156.1             | yes       | `notify` + session file, opt-in: [12](12-agent-events-stage-2-codex.md).                                                                                                                                                                                                                                                                                                                                              |
| OpenCode 1.18.31          | yes       | Plugin, opt-in: [13](13-agent-events-stage-2-opencode.md).                                                                                                                                                                                                                                                                                                                                                            |
| Grok 1.0.41               | yes       | Session directory, opt-in: [14](14-agent-events-stage-2-grok.md).                                                                                                                                                                                                                                                                                                                                                     |
| OMP 18.2.11, Pi 0.87.1    | yes       | Extension, opt-in: [15](15-agent-events-stage-2-pi-omp.md).                                                                                                                                                                                                                                                                                                                                                           |
| Cursor 2026.09.23-86fc751 | yes       | **Hooks through a per-launch plugin, opt-in, recorded**: sessions, tools with paths and durations, shell and MCP calls, file edits, subagents' ends, and — where the build fires them for plugins — turns with tokens. **Not available**: prompt sizes and subagent starts, because the only hooks that carry them can block (see §2). **Unverified live** until `cursor-agent login` and `scripts/record-cursor.sh`. |

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
  passive events and none of the deciding ones, before `--`; that no subscribed event is in
  `DECIDING` and every documented permission hook is; and that the shell Cursor would use —
  `sh -c` on Linux and macOS, PowerShell on Windows — reads a quoted path with spaces and
  apostrophes back unchanged. `launch::tests` arms it through a
  plan-catching host only when its switch is on. `crates/cli/tests/cli.rs` runs the real `ys`
  as the hook with a documented `afterFileEdit` on stdin and lists `file.reported_write`,
  `cursor/hook`, the path, and not the edit's text.
- By hand: owed. See [08 §17](08-manual-checklist.md#17--cursor-reporting); the recording it
  used to start with is done.

## 7 · Risks and the next slice

- ~~**Unrecorded.**~~ Recorded; see §8. A hook event this build does not fire for plugins is
  a missing row, and the build did not fire three of them.
- ~~**Environment.**~~ The hooks inherit the launch environment: `YARDSORT_RUN_ID` and the
  rest reached every one of the 19, so every Cursor event links to its run with nothing more.
- **The shell string.** A data directory containing a single quote is handled; one containing a
  newline is not tested.
- The next slice is not another adapter: stage 3 of [09](09-agent-events-and-memory.md).

## 8 · What the recording added

Recorded 2026-09-26 with `scripts/record-cursor.sh`, Cursor agent 2026.09.23-86fc751, one
headless turn (`-p --output-format json`, the plugin subscribed to every documented hook): 19
hook calls. Against the documentation:

- **Durations are fractional milliseconds** (`"duration": 67.921`). The adapter read them as
  integers and dropped every one; it now rounds. `sessionEnd`'s `duration_ms` is an integer.
- **`sandbox` is a boolean**, not a string.
- **Every hook carries the account's `user_email`.** Never read by the adapter — the tests now
  assert no `@` in any payload — and redacted by the recording script, which also redacts the
  project slug Cursor derives from the recording directory's name.
- **`afterFileEdit` is delivered before the tool's own `postToolUse`**, so `file.reported_write`
  precedes `Write done` on the timeline.
- **Not delivered to a plugin's hooks in this build:** `stop`, `afterAgentResponse`,
  `subagentStop`, `beforeSubmitPrompt`. So a Cursor turn's end, its tokens and its prompt sizes
  are not on the timeline yet; the mapping for the first three stays, from the documentation,
  for a build that sends them. `afterAgentThought` is delivered and is not subscribed to.
- **`tool_use_id` can contain a newline** (`call-…-0\nfc_…_0`): two ids joined. Recorded as
  given; it is a key, not a display string.
- **The launch environment reaches the hooks.** The `--new-session-id` fallback in §7 is not
  needed.
