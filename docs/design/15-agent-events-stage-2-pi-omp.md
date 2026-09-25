# 15 — Agent events, stage 2: OMP and Pi

Status: **shipped** · 25 September 2026 · One adapter for two harnesses. OMP (Oh My Pi) is a
fork of Pi, and the two share an extension API to the letter: a module that exports a factory,
given `pi`, which subscribes with `pi.on(name, handler)`. Both take an extension for one launch
on the command line (`-e <file>`), beside whatever the user has installed, with no trust prompt
and no configuration file involved. So this slice is the plugin pattern from
[13](13-agent-events-stage-2-opencode.md) again, in TypeScript, given through an argument
rather than an environment variable.

## 1 · What was recorded first

`scripts/record-pi.sh pi|omp` runs the _installed_ agent headlessly (`-p --mode json`) in a
throwaway repository, with `--session-dir` pointed at a throwaway so the session lands there and
not in the user's history — credentials stay where they are, so the agent stays logged in — and
a capture extension, given with `-e`, that subscribes to every event the API names and writes
each one as it arrives with the context it came with (`sessionManager.getSessionId()`, `cwd`,
`model`). Writes, redacted and with strings cut at 200 characters, under
`crates/core/fixtures/<agent>/<version>/`: one file per event, and the session file the agent
wrote.

Recorded 2026-09-25 against **OMP 18.2.11** (`openai-codex/gpt-5.3-codex`): 44 events over one
headless turn with a `write`, two `read`s (one of a missing file) and a `bash`. **Pi 0.87.1**
had no provider configured on the machine, so its recording is three events long
(`session_start`, `input`, `session_shutdown`) — enough to see that the shapes are OMP's and that
`--session-id` is honoured — and the rest of Pi's mapping is from OMP's fixtures and Pi's
documented API. Re-run `scripts/record-pi.sh pi` on a machine with a provider to complete it.

| Surface                                                               | Seen                                                                                                                                                                                                                                                                                                                  |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `-e <file>`                                                           | Repeatable; merges with the user's extensions from `~/.pi/agent/extensions/` and the project's `.pi/extensions/`; loaded even under `--no-extensions`. OMP also spells it `--hook`. `--trusted-extension` is a different thing (a trust grant for a named installed extension) and does not carry `-e`.               |
| The handler's `ctx`                                                   | `sessionManager.getSessionId()` / `getSessionFile()`, `cwd`, `model {id, provider}`, `mode`. The environment is the process's own, `YARDSORT_*` included.                                                                                                                                                             |
| `session_start` / `session_shutdown`                                  | `{type, reason}`.                                                                                                                                                                                                                                                                                                     |
| `input`                                                               | `{type, text, source}` — the prompt's text, which the extension reduces to its length before anything leaves the process.                                                                                                                                                                                             |
| `turn_start` / `turn_end`                                             | `{turnIndex, timestamp}`; `turn_end.message` carries `model`, `provider`, `stopReason`, `duration`, `ttft`, `usage {input, output, cacheRead, cacheWrite, reasoningTokens, totalTokens, cost {total}}`, and the reply's content, which is not copied.                                                                 |
| `tool_execution_start` / `tool_execution_end`                         | `{toolCallId, toolName, args}` and `{…, isError, result {details {resolvedPath, wallTimeMs, meta {source {type, value}}}}}` — `args` holds the command for `bash` and the content for `write`; only `args.path` and `args.cwd` are kept. `resolvedPath` (write/read) or `meta.source.value` (bash's cwd) is the path. |
| `tool_call` / `tool_result`, `message_*`, `agent_*`, `before_agent_*` | Fire, and are not subscribed to: `tool_call` is the one whose handler can block a tool, and the rest repeat what the pairs above say.                                                                                                                                                                                 |
| `tool_approval_requested` / `_resolved`                               | OMP's; `{toolCallId, toolName, approvalMode}` and `{…, approved}`. Not seen in the headless run (nothing asked); mapped from the API's types.                                                                                                                                                                         |
| `model_select`                                                        | Pi's; `{model, previousModel, source}`. Mapped from the API's types.                                                                                                                                                                                                                                                  |
| Session ids                                                           | Pi takes `--session-id` (Yardsort assigns one, `session_id_mode: Assigned`); OMP has no such flag and chooses its own UUIDv7 (`LatestInCwd`).                                                                                                                                                                         |

## 2 · Decisions

| Question                        | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **The channel**                 | **`-e <data-dir>/activity/hooks/<harness>.ts`**, inserted before any `--` (what follows one is the prompt). One template in the binary (`activity/pi-extension.ts`), written before each launch with the executable, the inbox and the harness filled in, so it names the binary that launched. Never the user's extension directories, never their `settings.json`.                                                                                                    |
| **One adapter, two producers**  | The same source serves both; `HARNESS` in the written file, and the hook mode's first argument, say which one launched, and that is the producer (`omp` or `pi`). Two switches — **Capture what OMP reports**, **Capture what pi reports** — because a user may run one and not the other.                                                                                                                                                                              |
| **Refusals**                    | `--trusted-extension` among the user's arguments means the launch is refused an extension: not because the flags conflict, but because a user granting trust by hand is not a launch to add to. Counted under `hooks_not_armed`, as for the others.                                                                                                                                                                                                                     |
| **Which events**                | Notification events only: never `tool_call`, whose handler's failure or return value can block or rewrite a tool. Every handler swallows its own errors; registering an event the running agent does not have (`tool_approval_*` on Pi, `model_select` on OMP) is a caught failure, not a refused launch.                                                                                                                                                               |
| **Where the reduction happens** | In the extension, before the spawn: `reduce()` copies only whitelisted fields — the prompt becomes `chars`, `args` becomes `{path, cwd}`, the reply and the tool's output are not copied at all — so the metadata-only rule holds before the bytes reach the executable. The Rust side (`activity/pi.rs`) reads the reduced shape and maps it; `reduce` is exported and covered by a vitest over the OMP fixtures, so the two halves are tested against the same files. |
| **Delivery**                    | `spawn(EXE, ["--yardsort-hook", HARNESS, INBOX])` with the reduced event as JSON on stdin, fire-and-forget, stdout and stderr ignored. Bun and Node both have `node:child_process`.                                                                                                                                                                                                                                                                                     |
| **Correlation**                 | By the environment, which reaches the extension untouched; `sessionManager.getSessionId()` is kept as the native session id, which for Pi is the one Yardsort assigned (`Store::run_by_harness_session` finds the run when the environment did not), and for OMP is its own, recorded for the reader.                                                                                                                                                                   |

## 3 · Event mapping

Producer `omp` or `pi`, method `extension`, fidelity `reported`. `occurred_at` is the hook's
clock: the events carry no time of their own except `turn_start`'s.

| Extension event           | Kind                             | Payload                                                                                                                                                                                |
| ------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session_start`           | `session.started`                | `sessionId`, `reason`, `model` (`provider/id`)                                                                                                                                         |
| `session_shutdown`        | `session.ended`                  | `sessionId`, `reason`                                                                                                                                                                  |
| `input`                   | `prompt.submitted`               | `sessionId`, `chars`, `source`                                                                                                                                                         |
| `turn_start`              | `turn.started`                   | `sessionId`, `turnNumber`, `model`                                                                                                                                                     |
| `turn_end`                | `turn.completed`                 | `sessionId`, `turnNumber`, `model`, `stopReason`, `durationMs`, `inputTokens`, `outputTokens`, `cachedInputTokens`, `cacheWriteTokens`, `reasoningOutputTokens`, `totalTokens`, `cost` |
| `tool_execution_start`    | `tool.started`                   | `tool`, `toolUseId`, `sessionId`, `path` (workspace-relative, from `args.path`)                                                                                                        |
| `tool_execution_end`      | `tool.completed` / `tool.failed` | `tool`, `toolUseId`, `sessionId`, `durationMs` (`wallTimeMs`), `path` (from `resolvedPath`, else `meta.source.value`), `pathOutsideWorkspace`                                          |
| `tool_approval_requested` | `approval.requested`             | `tool`, `toolUseId`, `sessionId`, `approvalMode`                                                                                                                                       |
| `tool_approval_resolved`  | `approval.resolved`              | `tool`, `toolUseId`, `sessionId`, `decision` (`allow` / `deny`)                                                                                                                        |
| `model_select`            | `agent.model_switched`           | `sessionId`, `model`, `previousModel`, `source`                                                                                                                                        |

Anything else the extension does not subscribe to, and anything it does that the Rust side does
not know is refused without a file.

## 4 · Coverage, honestly

| Harness             | Lifecycle | Native                                                                                                                                                                                                                                                                                                                           |
| ------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280 | yes       | Hooks, opt-in: [11](11-agent-events-stage-2-claude.md).                                                                                                                                                                                                                                                                          |
| Codex 0.156.1       | yes       | `notify` + session file, opt-in: [12](12-agent-events-stage-2-codex.md).                                                                                                                                                                                                                                                         |
| OpenCode 1.18.31    | yes       | Plugin, opt-in: [13](13-agent-events-stage-2-opencode.md).                                                                                                                                                                                                                                                                       |
| Grok 1.0.41         | yes       | Session directory, opt-in: [14](14-agent-events-stage-2-grok.md).                                                                                                                                                                                                                                                                |
| OMP 18.2.11         | yes       | **Extension, opt-in**: sessions, prompt sizes, turns with tokens and cost, tools with paths and durations, approvals. **Not available**: subagents (the events are the parent's), compaction, the length of the opening message given on the command line (OMP raises no `input` for it; typed messages it does). Recorded live. |
| Pi 0.87.1           | yes       | **The same extension, opt-in**, plus model switches, minus approvals (Pi has no such events). **Recorded only to the first prompt** — no provider on the recording machine — so the turn and tool mapping is from OMP's identical shapes and Pi's API. `scripts/record-pi.sh pi` completes it.                                   |
| Cursor              | yes       | Hooks through a per-launch plugin directory, opt-in: [16](16-agent-events-stage-2-cursor.md).                                                                                                                                                                                                                                    |

## 5 · What shipped

- `crates/core/src/activity/pi-extension.ts` (the template; `reduce` exported for the test) and
  `crates/core/src/activity/pi.rs`: `serves`, `extension_path`, `extension_source`, `arm`,
  `normalize`.
- The hook mode accepts `omp` and `pi`; the launcher arms either when its switch is on, with
  `capture: "extension"` on the run's start.
- Settings → General **Capture what OMP reports** and **Capture what pi reports**;
  `[activity] capture_omp`, `capture_pi`.
- `scripts/record-pi.sh`, `crates/core/fixtures/omp/18.2.11/`, `crates/core/fixtures/pi/0.87.1/`.
- `src/features/activity/piExtension.test.ts`: the extension's own `reduce` over the OMP
  fixtures, in vitest, so the TypeScript half is tested and not only read.

## 6 · Verification

- Rust: `activity::pi` tests read every OMP fixture through `normalize` and check the sixteen
  kinds come out in order with nothing leaked (no command, no content, no prompt text), Pi's
  partial recording carries the assigned id and the prompt's length, approvals and model
  switches map from the documented fields, arming writes the file with the template filled and
  names it before `--` for either harness, and refuses `--trusted-extension` and a harness the
  adapter does not serve. `launch::tests` arms OMP through a plan-catching host and leaves Pi
  alone when only OMP's switch is on. `crates/cli/tests/cli.rs` runs the real `ys` as the hook
  with `15-turn_end.json` on stdin and lists `turn.completed`, `omp/extension`, `19963 tokens`.
- Frontend: the two switches save, and are disabled without recording; `reduce` over the OMP
  fixtures.
- By hand: see [08 §16](08-manual-checklist.md#16--omp-and-pi-reporting).

## 7 · Risks and the next slice

- The extension API is a moving surface in both forks; a renamed event is a silent gap (the
  registration fails, is caught, and nothing is reported for it), not a broken launch. The
  fixtures name the versions.
- OMP's session id is its own; a delivery whose environment was lost (there is no known way for
  that to happen — the extension runs in-process) would fall to the workspace alone.
- Pi's recording wants a provider. The next slice is [16](16-agent-events-stage-2-cursor.md),
  Cursor, and with it every built-in harness has a native source.
