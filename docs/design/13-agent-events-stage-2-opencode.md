# 13 — Agent events, stage 2: OpenCode

Status: **shipped** · 25 September 2026 · The third native adapter of
[09-agent-events-and-memory](09-agent-events-and-memory.md), after
[Claude Code](11-agent-events-stage-2-claude.md) and [Codex](12-agent-events-stage-2-codex.md),
and the last of the three the plan named. The proposal's line for OpenCode — "opt-in managed
plugin emitting tool lifecycle, usage and session metadata; install only into a scoped/reversible
config" — held, with a better scope than it hoped for: the plugin is given for one launch,
through the environment, and installed nowhere.

## 1 · What was recorded first

`scripts/record-opencode.sh` runs the _installed_ `opencode run` in a throwaway repository under
a throwaway `XDG_DATA_HOME` (OpenCode's database lands there; its credentials live elsewhere, so
it stays logged in), with a capture plugin given through `OPENCODE_CONFIG_CONTENT` — the same
channel Yardsort's own plugin uses, so the recording proves the channel as well as the shapes.
Every hook call and bus event the plugin saw is under `crates/core/fixtures/opencode/<version>/`,
one file each (`{hook, payload, directory}`), redacted, minus the streaming deltas, the text and
reasoning parts and the catalogue events nobody reads: 80 of 168.

Recorded 2026-09-25 against **OpenCode 1.18.31** (`opencode/big-pickle`), one `run` turn with a
`write`, a `read`, a `read` of a missing file and a `bash`.

| Surface                          | Seen                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Plugin context                   | `{ client, project, directory, worktree }`; `process.env` carries the launch's `YARDSORT_*` — the plugin runs in OpenCode's own process (a worker thread of it, for the TUI), so the environment is inherited.                                                                                                                                                                                                                                                            |
| `chat.message`                   | `input.sessionID`; `output.message` (`id`, `role: user`, `agent`, `model {providerID, modelID}`, `time`) and `output.parts` (the text).                                                                                                                                                                                                                                                                                                                                   |
| `tool.execute.before` / `.after` | `input {tool, sessionID, callID}`; before: `output.args` (`filePath`, `content`, `command`, `workdir`, …); after: `input.args` plus `output {title, output, metadata}` — for `bash`, `metadata.exit`; for `write`, `metadata.exists`. No timing.                                                                                                                                                                                                                          |
| Bus `event`                      | `session.created` (`info`: id, version, directory, project), `session.status`, `session.updated`, `session.idle`, `file.edited {file}`, `message.updated` (`info`: the message; an assistant's, once `time.completed`, carries `tokens {input, output, reasoning, cache {read, write}}`, `cost`, `modelID`, `providerID`, `finish`), `message.part.updated` (every part state — including a tool's `error` state with `time {start, end}`, which the hooks never report). |
| Not seen                         | `permission.ask` (the hook is declared but, in this version, never triggered; the bus event `permission.asked` is the real one, with `{id, sessionID, permission, patterns, tool {messageID, callID}}`), `permission.replied`, `session.error`. Mapped from the SDK types and marked _documented_.                                                                                                                                                                        |

And by experiment: `OPENCODE_CONFIG_CONTENT` **merges** with the user's configuration — `opencode
debug config` showed their plugin and ours side by side — and OpenCode's plugin list is appended
to and deduplicated by URL, never replaced. There is no trust prompt, hash or sandbox for a
plugin: OpenCode imports the module. `--pure` skips every configured plugin, ours included.

## 2 · Decisions

| Question                    | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **How the plugin is given** | `OPENCODE_CONFIG_CONTENT={"plugin":["file:///<data-dir>/activity/hooks/opencode.js"]}` in the launch's environment (since [§ 8](#8--opencode-2), the file `opencode/index.js` there, and its folder under `plugins` for OpenCode 2). Merged with the user's configuration by OpenCode itself; if the launch environment already carries the variable, our plugin is added to its `plugin` list rather than replacing it, and configuration that is not JSON is left alone (`hooks_not_armed`). No file of the user's is written; `~/.config/opencode` is not visited.                                                                                    |
| **Where the plugin lives**  | `<data-dir>/activity/hooks/opencode.js` (since [§ 8](#8--opencode-2), `opencode/index.js`), written from a template in the binary before each launch, so it names the current executable. `ResolvedLaunch` gained `env` for this: a launch may carry variables of its own, appended to the user's environment by `Launcher::start`.                                                                                                                                                                                                                                                                                                                      |
| **What the plugin does**    | Subscribes to `event`, `chat.message`, `tool.execute.before`, `tool.execute.after`, `permission.ask`. `reduce()` — pure, exported, tested under vitest against the fixtures — keeps a whitelist of fields of the raw payload's own shape and drops everything else: no prompt text, command, output, file content, title or error message leaves OpenCode's process. Each kept call is handed to this executable in hook mode on **stdin** (a tool's output can exceed an argument's length; the reduced payload never does, but stdin has no limit to think about). Spawned as an argument list; fire and forget; every hook swallows its own failures. |
| **`--pure`**                | Respected by not arming: the user asked for no external plugins.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| **Both shapes, one reader** | The Rust normalizer reads the raw hook shapes; the plugin's reduced shape is a subset with the same paths, plus `directory` at the top. Rust tests run over the raw fixtures, so they prove reduction even of what the plugin does not send.                                                                                                                                                                                                                                                                                                                                                                                                             |
| **Paths**                   | A tool's `workdir` when it has one, else the launch directory the plugin sends with every delivery; relative to the workspace or `pathOutsideWorkspace`, as for the other adapters.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| **Usage**                   | From `message.updated` once an assistant message has `time.completed`: tokens, cache reads and writes, cost, the model. One `usage.reported` per assistant message, which is per step of a turn — OpenCode's turn is several messages.                                                                                                                                                                                                                                                                                                                                                                                                                   |
| **Correlation**             | `YARDSORT_RUN_ID` from the plugin's `process.env`, then the workspace id. The session id OpenCode chose is on every event and recorded, but no run has it (OpenCode chooses its own, and Yardsort resumes it as "latest in cwd"); recording it from `session.created` so Resume can name it is a follow-up shared with Codex.                                                                                                                                                                                                                                                                                                                            |
| **Version**                 | No probe. The fixture version is named in the code and the guide; unknown events are dropped by the plugin, unknown hooks refused by the reader.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

## 3 · Event mapping

Producer `opencode`, method `plugin`, fidelity `reported`; `occurred_at` is the hook process's
clock (OpenCode's events carry no time of their own except a tool's `state.time`).

| Hook / event                                             | Kind                  | Payload                                                                                                                                                                      | Source     |
| -------------------------------------------------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `event session.created`                                  | `session.started`     | `sessionId`, `version`, `parentId`                                                                                                                                           | fixture    |
| `chat.message`                                           | `prompt.submitted`    | `sessionId`, `messageId`, `chars`, `agent`, `model`                                                                                                                          | fixture    |
| `tool.execute.before`                                    | `tool.started`        | `tool`, `toolUseId`, `sessionId`, `path`? / `pathOutsideWorkspace`?                                                                                                          | fixture    |
| `tool.execute.after`                                     | `tool.completed`      | as above, plus `exitCode` (`bash`), `truncated`, `created` (`write` of a new file)                                                                                           | fixture    |
| `event message.part.updated`, tool part in `error`       | `tool.failed`         | `tool`, `toolUseId`, `sessionId`, `messageId`, `path`?, `durationMs`                                                                                                         | fixture    |
| `event file.edited`                                      | `file.reported_write` | `sessionId`, `path`? / `pathOutsideWorkspace`?, `kind: "changed"`                                                                                                            | fixture    |
| `event message.updated`, assistant, completed            | `usage.reported`      | `sessionId`, `messageId`, `model`, `agent`, `finish`, `inputTokens`, `outputTokens`, `reasoningOutputTokens`, `cachedInputTokens`, `cacheWriteTokens`, `totalTokens`, `cost` | fixture    |
| `event session.idle`                                     | `turn.completed`      | `sessionId`                                                                                                                                                                  | fixture    |
| `event session.error`                                    | `turn.failed`         | `sessionId`, `errorType`                                                                                                                                                     | documented |
| `event permission.asked` (and the `permission.ask` hook) | `approval.requested`  | `sessionId`, `tool` (the permission is the tool asked for), `permission`, `permissionId`, `toolUseId`                                                                        | documented |
| `event permission.replied`                               | `approval.resolved`   | `sessionId`, `decision`, `permissionId`                                                                                                                                      | documented |

## 4 · Coverage, honestly

| Harness             | Lifecycle | Native                                                                                                                                                                                                                                                               |
| ------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code 2.1.280 | yes       | Hooks, opt-in: [11](11-agent-events-stage-2-claude.md).                                                                                                                                                                                                              |
| Codex 0.156.1       | yes       | `notify` + session file, opt-in: [12](12-agent-events-stage-2-codex.md).                                                                                                                                                                                             |
| OpenCode 1.18.31    | yes       | **Plugin, opt-in**: sessions, prompts, tools with exit codes and failures, file edits, permissions, turns, token usage and cost per assistant message. **Not available**: a tool's duration when it succeeds (the hooks carry no timing; only the error state does). |
| Grok                | yes       | **Shipped**: [14](14-agent-events-stage-2-grok.md).                                                                                                                                                                                                                  |
| OMP, Pi             | yes       | **Shipped**: [15](15-agent-events-stage-2-pi-omp.md), this pattern again.                                                                                                                                                                                            |
| Cursor              | yes       | **Shipped**, from the documentation: [16](16-agent-events-stage-2-cursor.md).                                                                                                                                                                                        |

**Pull requests opened** ([38 § slice 3](38-pull-requests-elsewhere.md#slice-3-what-shipped)).
`tool.execute.after` carries the call's arguments and output. For a call that may have opened a
pull request — `bash` whose command names `gh`, `pr` and `create`, or a tool whose name says
GitHub and ends `create_pull_request` (OpenCode names an MCP tool `<server>_<tool>`) — the
plugin hands on the command and the output too, and the Rust side keeps the link of what
`gh pr create` printed, or of the tool's answer, as `pullRequestUrl`, and nothing else. Every
other call's output stays in OpenCode's process. Recorded against **1.18.34**
(`scripts/record-pr-creation.sh opencode`, `fixtures/opencode-pull-requests/`).

## 5 · What shipped

- `crates/core/src/activity/opencode.rs` and `opencode-plugin.js`: `arm()` (the file, the
  merged variable, the `--pure` refusal), `normalize()`; tests over every fixture, the
  documented-only events, paths, arming and merging, `file://` URLs.
- `hook.rs`: the OpenCode branch; `ResolvedLaunch.env` and `Launcher::start` appending it;
  `capture: "plugin"` on the run's start.
- Settings → General **Capture what OpenCode reports**; `[activity] capture_opencode`.
- `src/features/activity/opencodePlugin.test.ts`: the plugin's `reduce` under vitest, over the
  same fixtures — the one adapter with code that runs inside the agent, tested where it runs.
- `scripts/record-opencode.sh`, `crates/core/fixtures/opencode/1.18.31/`.

## 6 · Verification

| Case                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Where                                                                                                                           |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Every recorded call maps to the expected sequence; content never survives                                                                                                                                                                                                                                                                                                                                                                             | `opencode::tests::the_recorded_run_maps…`, `…what_is_kept_is_metadata…`                                                         |
| Documented-only events; a failed tool's duration; paths against `workdir` / launch directory                                                                                                                                                                                                                                                                                                                                                          | `opencode::tests::calls_this_build_does_not_record…`, `…paths_are_relative…`                                                    |
| The plugin file names this executable; the variable merges with the user's; `--pure`; bad JSON                                                                                                                                                                                                                                                                                                                                                        | `opencode::tests::the_plugin_names_this_executable…`                                                                            |
| The plugin's `reduce`: keeps the same calls, leaks nothing, keeps what Rust reads                                                                                                                                                                                                                                                                                                                                                                     | `opencodePlugin.test.ts`                                                                                                        |
| Only `opencode`, only with its switch on; nothing on the command line                                                                                                                                                                                                                                                                                                                                                                                 | `launch::tests::an_opencode_launch_is_given_its_plugin…`                                                                        |
| Hook mode on stdin; the real `ys` binary end to end                                                                                                                                                                                                                                                                                                                                                                                                   | `hook::tests::an_opencode_plugin_delivery…`, `cli::the_binary_as_the_opencode_plugins_hook…`                                    |
| Settings switch                                                                                                                                                                                                                                                                                                                                                                                                                                       | `ActivitySettings.test.tsx`                                                                                                     |
| By hand: a real OpenCode, launched through `ys` on Linux, reporting live — session, prompt, `write` started and done with the path, the file changed, `bash` with its exit code, usage per reply, the turn's end — and the bug that found: the plugin exported its `reduce` helper beside the plugin function, and OpenCode calls every export as a plugin, so the launch died on start-up (exit 1); the module has one export now, checked by a test | [08 §14](08-manual-checklist.md#14--opencode-reporting) has the app-window rows; Linux via `ys` done, macOS and Windows pending |

## 7 · Risks and the next slice

- **OpenCode 2 has shipped, and the plugin did not load there.** Found on 2026-11-09 against
  2.2.6, while recording [38](38-pull-requests-elsewhere.md)'s slice 3: a plugin given as a
  file is refused (`configured plugin path must be a directory`), and one given as a folder must
  export a default definition with an `id` and an `effect` or `setup` function, where 1.x calls
  every export as a plugin function. Ported on 2026-11-10: [§ 8](#8--opencode-2).
- **The plugin runs inside the agent.** It is the one adapter whose code can, in principle, affect
  the harness: every hook is wrapped, the spawn is fire-and-forget, and nothing is awaited. A
  plugin that could not spawn the executable — after an update moved it — reports nothing and
  says nothing.
- **Side effects of OpenCode's own** on any config directory it scans (a `.gitignore`, a
  background `@opencode-ai/plugin` install) are OpenCode's and happen with or without Yardsort;
  `OPENCODE_CONFIG_CONTENT` with a `file://` plugin adds no directory to scan.
- **Next:** the three harnesses the plan named now report. What follows is using it: a per-run
  token total from `usage.reported` (Codex and OpenCode), `agent.notified` /
  `approval.requested` driving the sidebar badge, and the session-id follow-up so Resume can
  name a Codex or OpenCode conversation instead of taking the latest in the directory.

## 8 · OpenCode 2

Ported on 2026-11-10 against **OpenCode 2.2.6**. Both versions are kept: one module, one adapter
with a mapping for each.

**What was found.**

- **The plugin's form.** A plugin is a folder (its `index.js`), listed under `plugins`, not
  `plugin`, and its default export is a definition `{ id, setup(ctx) }`. `setup` registers hooks
  on the domains — `ctx.tool.hook("execute.before" | "execute.after")`,
  `ctx.session.hook("prompt")`, `ctx.permission.hook("evaluate")` — and reads the bus through
  `ctx.event.subscribe({ types, signal })`; it answers with a cleanup. OpenCode 1 from 1.18.29
  starts an object entry point through its `server` function, so one default export with `id`,
  `server` and `setup` serves both, and each ignores the other's config key. Checked by hand
  against 1.18.34 and 2.2.6, with both keys given: each started its own entry point, once.
  `OPENCODE_CONFIG_CONTENT` still merges with the user's configuration.
- **Where sessions run.** The TUI starts a background service (`opencode serve --service`) and
  every later launch on the machine connects to it, and it outlives the launch that started it.
  It loads plugins from _its own_ environment. Seen with two TUIs in two folders, each given a
  different capture plugin and run id: the second was served by the first's plugin, with the
  first's `YARDSORT_RUN_ID`, and its own plugin never loaded. A plugin there would put every
  later OpenCode session on the machine — one started outside Yardsort, too — on a run that has
  ended. `--standalone` gives a launch a private server (`serve --stdio`) that inherits its
  environment and ends with it, and the sessions are kept in the same database, so
  `opencode session list` and `--continue` find them. OpenCode 1 refuses `--standalone` as an
  unknown flag, so the launch has to know which it is starting.
- **The shapes**, recorded with `scripts/record-opencode.sh` (now version-aware: `--standalone
--auto`, and a rule that every shell command asks, so a permission is asked for and granted)
  into `crates/core/fixtures/opencode/2.2.6/`, and with `scripts/record-pr-creation.sh opencode`
  into `crates/core/fixtures/opencode-pull-requests/2.2.6/`. Every delivery is
  `{api: 2, hook, payload, directory}`. A hook gets one event, not `input` and `output`: a tool's
  is `{tool, sessionID, messageID, id, input}`, and its end adds `status` (`completed` or
  `error`) with `result {output, content, metadata}` or `error`, so a failed call is reported
  directly. The shell tool is `shell`, its exit in `result.output.exit`; a file tool's argument
  is `path`; `write` says `existed`. The bus is new: `session.created` (only when the TUI makes the
  session; `run` makes its own before a plugin listens), `session.step.started` / `.ended` (the
  step's model at its start; `tokens`, `cost`, `finish` and the `files` it changed at its end),
  `session.execution.succeeded` / `.failed` / `.interrupted` (the turn), `permission.asked`
  (`action`, `resources` — the command or path — and `source.id`, the call) and
  `permission.replied` (`reply`). There is no file-edited event. An MCP tool runs inside the
  `execute` tool, whose hooks fire around the inner call's, under the same call id.

**Decisions.**

| Question               | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| One plugin or two      | One module, `opencode-plugin.js`, written to `activity/hooks/opencode/index.js`: given as the file to `plugin` and as the folder to `plugins`. Its one export is the default; the pure functions hang off it under `testing`.                                                                                                                                                                                                                                                                                                                                                               |
| Telling 1 from 2       | `opencode --version`, run with the launch's environment, in its folder (a version manager's shim answers for what that folder pins), before arming: five seconds at most, in a process group ended if it is still running. An answer is kept a minute per executable, folder and modification time; no answer is not kept. Only a program whose name starts `opencode` is asked (a wrapper such as `npx` would answer for itself), and only a line that is a version (`1.18.34`, `opencode v2.2.6`) answers. Known, only that version's key is added to the configuration; not known, both. |
| The shared service     | An OpenCode 2 launch is given `--standalone`, first, unless it has it. One whose arguments name `--server` is not armed (`hooks_not_armed`). A plugin that finds `--service` in its process's arguments registers nothing: an unknown version, or a service started some other way, reports nothing rather than reporting on the wrong run.                                                                                                                                                                                                                                                 |
| Hooks and events       | `tool.execute.before` / `.after`, `session.prompt` (the length only), and the bus events above. Not `permission.evaluate`, which runs for every action that is allowed too; `permission.asked` is the one that waits for you.                                                                                                                                                                                                                                                                                                                                                               |
| What the plugin adds   | From its own memory, and marked as such: a failed call's `durationMs` (from its start, keyed by tool and id) and a step's `model` (from its start).                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Writes, for provenance | No file event exists, so a completed `write` or `edit` is marked `wrote` and counted as the write; OpenCode 1's tool calls are still not counted beside its file event. `patch` writes too, but its call names no path, so it is not counted. The step's end lists the files it changed; that list is not sent, as nothing reads it yet.                                                                                                                                                                                                                                                    |
| Pull requests          | As for 1: the shell's `gh pr create` hands on its command and printed output, a GitHub `…create_pull_request` tool its `content`; the executable keeps the link alone. The inner MCP call carries the link; the `execute` around it hands on nothing.                                                                                                                                                                                                                                                                                                                                       |

| OpenCode 2                                     | Event                                                                  |
| ---------------------------------------------- | ---------------------------------------------------------------------- |
| `session.created`                              | `session.started` (session, version, parent)                           |
| `session.prompt` hook                          | `prompt.submitted` (characters)                                        |
| `tool.execute.before`                          | `tool.started` (tool, call, path)                                      |
| `tool.execute.after`, `completed`              | `tool.completed` (exit code, truncated, created, `wrote`, a PR's link) |
| `tool.execute.after`, `error`                  | `tool.failed` (duration)                                               |
| `permission.asked` / `permission.replied`      | `approval.requested` (the action as `tool`) / `approval.resolved`      |
| `session.step.ended`                           | `usage.reported` (tokens, a total summed from them, cost, model)       |
| `session.execution.succeeded` / `.interrupted` | `turn.completed` (`interrupted: true` for the second)                  |
| `session.execution.failed`                     | `turn.failed` (the error's type)                                       |

**Verification.** `opencode::tests::the_recorded_opencode_2_run_maps…`,
`…a_pull_request_an_opencode_2_call_opened…`, `…opencode_2_sessions_turns_and_failures…`,
`…an_opencode_2_launch_is_given_a_server_of_its_own`, `…the_version_is_read…`,
`…the_program_is_asked_which_opencode_it_is` (stand-ins), and
`launch::tests::an_opencode_2_launch_is_given_its_plugin_and_a_server_of_its_own`;
`opencodePlugin.test.ts` plays the 2.2.6 fixtures through `reduceV2` (nothing of the prompt,
command, output, file or error leaves it) and `setup` against a stand-in context (its hooks, its
subscription and its cleanup; nothing at all in the shared service). By hand on Linux, through
`ys workspace new … --harness opencode` in a throwaway profile: the real 2.2.6 TUI started as
`opencode --standalone --prompt …`, its private server carrying the run's id, and the timeline
showed `session.started`, `prompt.submitted`, `write` started and done with `hello.txt`, `read`
failed with `missing.txt` and 14 ms, `shell` done with exit 0, two `usage.reported` with
`opencode/big-pickle`, and `turn.completed`, nothing of the prompt or a command in any payload.
And the guard: a TUI started without `--standalone`, given Yardsort's plugin and a workspace's
ids, ran its turn in the shared service and left nothing in the inbox. The app window's rows,
and macOS and Windows, are [08 § 14](08-manual-checklist.md#14--opencode-reporting)'s.
