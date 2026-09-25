# 11 — Agent events, stage 2: Claude Code

Status: **shipped** · 25 September 2026 · The first native adapter of
[09-agent-events-and-memory](09-agent-events-and-memory.md), on the stage 0–1 base recorded in
[10-agent-events-stage-1](10-agent-events-stage-1.md). Claude Code first, as
[open question 20](06-open-questions.md#agent-events) leaned; Codex and OpenCode are not started.

## 1 · What was recorded first

Stage 0's rule was "no adapter before fixtures". `scripts/record-claude-hooks.sh` runs the
_installed_ `claude` non-interactively in a throwaway repository, with every hook event Yardsort
cares about pointed at a capture script, and writes each payload — redacted of the machine's paths
and ids — under `crates/core/fixtures/claude-hooks/<version>/`. The adapter's tests read those
files, so a re-recording for a new version that changes a shape fails a test rather than a user.

Recorded 2026-09-25 against **Claude Code 2.1.280**, model `haiku`, three runs on one
conversation: fresh with `--session-id`, then `--resume`, then `--resume --fork-session
--session-id`. Twenty payloads:

| Seen                                    | Fields that matter                                                                                                                                                                                              |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SessionStart` ×3                       | `source`: `startup`, `resume`, `fork`. On resume and fork also `context_tokens`, `seconds_since_last_response`, `estimated_cache_write_usd`.                                                                    |
| `UserPromptSubmit` ×3                   | `prompt` (the text), `prompt_id`, `permission_mode`.                                                                                                                                                            |
| `PostToolUseFailure` ×1                 | `tool_name`, `tool_input`, `tool_use_id`, `error` (a sentence naming the working directory), `is_interrupt`, `duration_ms`. From a `Read` of a file that does not exist.                                        |
| `PreToolUse` ×4 / `PostToolUse` ×3      | `tool_name`, `tool_input` (for `Write`/`Read`: `file_path` and content; for `Bash`: `command`, `description`), `tool_use_id`, and on Post: `tool_response` (file content, stdout) and `duration_ms`.            |
| `Stop` ×3                               | `stop_hook_active`, `last_assistant_message` (the text), `background_tasks`, `session_crons`.                                                                                                                   |
| `SessionEnd` ×3                         | `reason` (`other`, in print mode).                                                                                                                                                                              |
| Common to all                           | `session_id` (the id Yardsort chose — the join key), `transcript_path`, `cwd`; `prompt_id` and `permission_mode` on everything after the prompt.                                                                |
| The hook process's environment (`.env`) | `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID`, `YARDSORT_SESSION_RECORD_ID` **inherited** from the launch; plus `CLAUDECODE=1`, `CLAUDE_PROJECT_DIR`. The launcher's correlation survives; no fallback was needed. |

Also learned by recording, not by reading: a hook given `"args"` is spawned as an argument list
with no shell (the settings file uses that form, and a path with spaces is nobody's problem);
`--settings <file>` works in print mode alongside `--session-id`, `--resume` and `--fork-session`;
and the variadic `--allowedTools` swallows a trailing prompt unless `--` precedes it, which is why
Yardsort's own `prompt_args` for Claude are `["--", "{prompt}"]`.

**Not recorded**, because the scripted run never triggered them: `PermissionRequest`, `PermissionDenied`, `Notification`, `StopFailure`, `SubagentStart`,
`SubagentStop`, `PostCompact`, `PostModelSwitch`. They are subscribed to and mapped from the
public reference, with the fields it names, and marked _documented_ in the table below. A payload
of theirs that carries a field under a different name loses that field, never the event.

## 2 · Decisions

| Question                                 | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Install** hooks, or not?               | **Not.** Each launch of Claude Code from Yardsort gets `--settings <data-dir>/activity/hooks/claude.json`, rewritten before every launch. Claude Code merges it with the user's settings, so their hooks still run and none of their files is edited. Off, and the next launch has no hooks. There is no uninstall because there was no install. 09's "opt-in, reversible" is met by never touching the user's configuration in the first place.                           |
| A **shell string** for the hook command? | **No.** The hook is `{"type":"command","command":"<exe>","args":["--yardsort-hook","claude","<inbox>"],"timeout":5}` — an argument list, spawned directly. The project rule about argv holds even inside Claude Code's config, and Windows' Git-Bash-or-PowerShell choice for shell hooks does not apply.                                                                                                                                                                  |
| Which **executable**?                    | The one that launched Claude Code — the app's binary or `ys` — both of which have the `--yardsort-hook` mode before Tauri or clap see the arguments. For a Linux AppImage, `$APPIMAGE` rather than the mounted path, which is gone once the app quits while hooks keep firing.                                                                                                                                                                                             |
| Where does the **receiver** live?        | **Nowhere.** The hook writes one file to `<data-dir>/activity/inbox/` (temporary name, atomic rename, 10 000 cap, drops counted) and exits 0. The app drains it on start, on every live exit, and whenever the directory changes (a `notify` watch, 200 ms debounce); `ys` drains it before any command that would ask the daemon, and before listing activity. This is the exit spool's pattern again, and it answers 06's question 20: not the daemon, and not a socket. |
| Where is the payload **reduced**?        | **In the hook**, before anything touches disk. The raw payload — prompt text, commands, file contents, the assistant's words — exists only in the hook process's memory. The inbox holds metadata.                                                                                                                                                                                                                                                                         |
| **Correlation**                          | `YARDSORT_RUN_ID` from the hook's environment, which Claude Code inherits from the launch and passes on. Then the agent's `session_id`, which for Claude is the id the launcher chose and recorded on the run. Then `YARDSORT_WORKSPACE_ID` alone. Never the working directory. An entry none of those place is counted (`inbox_unlinked`) and dropped.                                                                                                                    |
| **Dedupe**                               | Source key `inbox:<entry id>`; the entry id is a UUID the hook chose. Two clients draining the same file make one row. Claude Code's own repeats (a tool called twice) are two entries, as they should be.                                                                                                                                                                                                                                                                 |
| **Blocking**                             | Never. The hook exits 0 whatever happens — a 2 would block the tool it was told about, and any other non-zero is an error Claude Code shows. Problems go to stderr, which Claude Code keeps. Stdout stays empty: some events read it back as instructions. Timeout 5 s per hook; the real cost is one small file write.                                                                                                                                                    |
| **Which runs**                           | Only a run that was recorded (so `record_lifecycle` on) for harness id `claude`, whatever command that id is overridden to. A harness whose arguments already carry `--settings` or `--bare` is left alone and the refusal counted (`hooks_not_armed`). The run's `process.started` event says `capture: "hook"`, which is the per-run coverage indicator 09 asked for.                                                                                                    |
| **Version gating**                       | None: Yardsort does not probe Claude Code's version. The fixture version is named in the code and the guide. Unmapped events are not subscribed to, so a newer Claude Code adds nothing unasked; a renamed field falls out of a payload rather than breaking it.                                                                                                                                                                                                           |
| **Privacy class**                        | Every reported event is `metadata`. Paths are made relative to the workspace, and a path outside it is replaced by `pathOutsideWorkspace: true`. The prompt is a character count. `Bash` has no command; `Agent` has its `subagent_type` only.                                                                                                                                                                                                                             |
| A per-run **capability token**?          | Not now. The inbox is a directory in the user's own data directory; whoever can write to it can write to the database beside it. A token would add a check without adding a boundary. Revisit if the receiver ever becomes a socket.                                                                                                                                                                                                                                       |

## 3 · Event mapping

Producer `claude`, method `hook`, fidelity `reported`, schema 1. `occurred_at` is the hook
process's clock: Claude Code sends no time of its own.

| Hook event           | Kind                                                | Payload                                                                                              | Source     |
| -------------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ---------- |
| `SessionStart`       | `session.started`                                   | `source`, `contextTokens`                                                                            | fixture    |
| `SessionEnd`         | `session.ended`                                     | `reason`                                                                                             | fixture    |
| `UserPromptSubmit`   | `prompt.submitted`                                  | `promptId`, `chars`, `permissionMode`                                                                | fixture    |
| `PreToolUse`         | `tool.started`                                      | `tool`, `toolUseId`, `path`?, `pathOutsideWorkspace`?, `subagentType`?, `promptId`, `permissionMode` | fixture    |
| `PostToolUse`        | `tool.completed`                                    | as above, plus `durationMs`                                                                          | fixture    |
| `PostToolUseFailure` | `tool.failed`                                       | as `tool.started`, plus `durationMs`, `interrupted`; no error text                                   | fixture    |
| `PermissionRequest`  | `approval.requested`                                | as `tool.started`                                                                                    | documented |
| `PermissionDenied`   | `approval.resolved`                                 | as `tool.started`, plus `decision: "denied"`                                                         | documented |
| `Notification`       | `agent.notified`                                    | `type` (`permission_prompt`, `idle_prompt`, …)                                                       | documented |
| `Stop`               | `turn.completed`                                    | `promptId`, `stopHookActive`, `backgroundTasks` (a count)                                            | fixture    |
| `StopFailure`        | `turn.failed`                                       | `promptId`, `errorType`                                                                              | documented |
| `SubagentStart/Stop` | `agent.subagent_started` / `agent.subagent_stopped` | `agentId`, `agentType`                                                                               | documented |
| `PostCompact`        | `session.compacted`                                 | `trigger`                                                                                            | documented |
| `PostModelSwitch`    | `agent.model_switched`                              | `model`, `previousModel`                                                                             | documented |

`agent.notified` with `permission_prompt` or `idle_prompt` is the signal open question 16 wanted
to infer from the screen — for Claude Code, reported instead of guessed. Nothing consumes it yet.

## 4 · Coverage, honestly

| Harness             | Lifecycle (stage 1) | Native (stage 2)                                                                                                                                                                                                                                                                       |
| ------------------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280 | yes                 | **Hooks, opt-in**: sessions, prompts, tools, permissions, turns, subagents, compaction, model switches. **Not available** through hooks: token usage per turn (only `context_tokens` at a resume), diffs, command strings, the assistant's text — the last three by Yardsort's choice. |
| Codex               | yes                 | **Shipped**, differently: [12](12-agent-events-stage-2-codex.md).                                                                                                                                                                                                                      |
| OpenCode            | yes                 | **Shipped**: [13](13-agent-events-stage-2-opencode.md).                                                                                                                                                                                                                                |
| Grok                | yes                 | **Shipped**: [14](14-agent-events-stage-2-grok.md).                                                                                                                                                                                                                                    |
| OMP, Pi             | yes                 | **Shipped**: [15](15-agent-events-stage-2-pi-omp.md).                                                                                                                                                                                                                                  |
| Cursor              | yes                 | **Shipped**, from the documentation: [16](16-agent-events-stage-2-cursor.md).                                                                                                                                                                                                          |
| Custom harnesses    | yes                 | Lifecycle only — unless the custom harness _is_ Claude Code under id `claude`, in which case it gets the hooks too.                                                                                                                                                                    |

## 5 · What shipped

- `crates/core/src/activity/claude.rs` — the settings file, `arm()`, `normalize()`; tests over
  every fixture, the documented-only events, path relativity, and the arming rules.
- `crates/core/src/activity/hook.rs` — the `--yardsort-hook` mode, bounded stdin, always exit 0.
- `crates/core/src/activity/inbox.rs` — the inbox, the spool's twin.
- `activity::import_inbox` and `Store::run_by_harness_session` — the drain and the fallback key.
- `Launcher` takes `data_dir` and arms a recorded `claude` launch when `capture_claude` is on;
  `process.started` carries `capture`.
- App: `--yardsort-hook` before Tauri; drain on start, on exit, and on inbox change (`notify`);
  `ActivityChanged` event; Settings → General switch and inbox diagnostics; the timeline's words
  for reported events, and a live reload.
- `ys`: `--yardsort-hook` before clap; drains with the spool; `ys doctor` shows the inbox;
  `ys activity list` details for tools, paths, durations.
- Settings: `[activity] capture_claude` (default `false`).
- `scripts/record-claude-hooks.sh` and `crates/core/fixtures/claude-hooks/2.1.280/`.

## 6 · Verification

| Case                                                                                                                                                                                                                                                                                                                          | Where                                                                                                                              |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Every recorded payload maps, in Claude's order; content never survives                                                                                                                                                                                                                                                        | `claude::tests::every_recorded_payload_maps…`, `…what_is_kept_is_metadata…`                                                        |
| Documented-only events map by their documented fields                                                                                                                                                                                                                                                                         | `claude::tests::events_documented_but_not_recorded…`                                                                               |
| Paths relative, outside-workspace flagged, Windows drives                                                                                                                                                                                                                                                                     | `claude::tests::paths_are_relative…`                                                                                               |
| Settings file: exec form, every event, no matcher, timeout                                                                                                                                                                                                                                                                    | `claude::tests::the_settings_file_runs_this_executable…`                                                                           |
| Arming: file written, `--settings` added; user's `--settings`/`--bare` win                                                                                                                                                                                                                                                    | `claude::tests::arming_writes_the_file…`, `launch::tests::a_claude_launch_is_given_hooks…`                                         |
| Only `claude`, only recorded runs, only with the switch on                                                                                                                                                                                                                                                                    | `launch::tests::a_claude_launch_is_given_hooks…`                                                                                   |
| The hook: fixture in, one entry out with the launcher's ids; garbage refused; stdin bounded                                                                                                                                                                                                                                   | `hook::tests::*`                                                                                                                   |
| Inbox: ordering, cap, dropped counter, newer version refused                                                                                                                                                                                                                                                                  | `inbox::tests::*`                                                                                                                  |
| Drain: by run, by native session, by workspace, stray dropped and counted; drained twice is one row; bad file dropped                                                                                                                                                                                                         | `activity::tests::inbox_entries_are_placed…`, `…drained_twice…`                                                                    |
| The real `ys` binary as a hook, then `ys activity list`, `ys doctor`                                                                                                                                                                                                                                                          | `cli::the_binary_is_a_hook…` (Linux, macOS, Windows in CI)                                                                         |
| Settings switch saves; capture disabled without recording; diagnostics                                                                                                                                                                                                                                                        | `ActivitySettings.test.tsx`                                                                                                        |
| Timeline words for reported events; reload on `ActivityChanged`                                                                                                                                                                                                                                                               | `ActivityPanel.test.tsx`                                                                                                           |
| By hand: a real Claude Code, launched through `ys` on Linux, reporting live: session, prompt, a failed `Read`, a `Bash` call, the turn — and the bug that found: `--settings` appended _after_ the `--` that precedes the prompt was read as prompt text, so no hook ran; `arm()` now inserts it before the `--`, with a test | [08 §12](08-manual-checklist.md#12--claude-code-reporting) has the app-window rows; Linux via `ys` done, macOS and Windows pending |

## 7 · Risks and the next slice

- **A hook that outlives its executable.** After an update on Linux (not AppImage) or a `just
dev` rebuild, a running Claude Code still names the old path; a missing executable is an error
  Claude Code shows once per hook event until the conversation is restarted. AppImage users are
  covered by `$APPIMAGE`; macOS and Windows installs replace in place.
- **Print mode and interactive mode differ** in what fires. The fixtures are print-mode; the
  interactive session seen by hand produced the same shapes for what both have, but
  `Notification` and `PermissionRequest` are interactive-only and documented-only here. The
  first pass through 08 §12 should re-record from an interactive session.
- **Volume.** A busy conversation is two entries per tool call; the inbox cap is 10 000 with a
  counted overflow, and retention (20 000 events, 90 days) prunes at startup. Both may need
  raising once real usage is seen.
- **Next:** Codex. Record its hook trust prompt and what its hooks deliver, or its session
  JSONL, with the same script shape; decide between them on fixtures, as 06 §20 says. Then use
  `agent.notified` in the sidebar for Claude Code — the badge can say _waiting for permission_
  instead of _quiet_, which is question 16 answered for one harness without a model.
