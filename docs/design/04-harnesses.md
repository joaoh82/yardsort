# 04 — Harnesses

A **harness** is a terminal coding agent. To Yardsort it is pure configuration: a command plus
argument templates for the handful of things we need to do with it. Every supported agent has the
same shape because they all run in a terminal and all take roughly the same startup options.

## What we need from a harness

| Action                                                 | When                                                    |
| ------------------------------------------------------ | ------------------------------------------------------- |
| **Start** with an initial prompt (plus model / effort) | New workspace, new session                              |
| **Start** with no prompt                               | Empty composer, `local`                                 |
| **Resume** a previous session                          | App restarted, or harness exited and user clicks Resume |
| **Fork** a previous session                            | User wants to branch the conversation                   |

## Definition

Stored in the settings TOML; this is what the Settings → Harnesses form edits.

```toml
[[harness]]
id          = "claude"                 # stable key, referenced by sessions
label       = "Claude"                 # shown in the UI
command     = "claude"                 # resolved against the login-shell PATH
enabled     = true

base_args   = []                                   # always passed
model_args  = ["--model", "{model}"]               # omitted when model = default
effort_args = ["--effort", "{effort}"]             # omitted when effort = default
efforts     = ["low", "medium", "high", "xhigh", "max"]
models      = ["fable", "opus", "sonnet"]          # suggestions only; free text allowed

session_args = ["--session-id", "{session_id}"]    # omitted unless session_id_mode = "assigned"
prompt_args = ["{prompt}"]                         # omitted when there is no opening message
resume_args = ["--resume", "{session_id}"]
fork_args   = ["--resume", "{session_id}", "--fork-session", "--session-id", "{new_session_id}"]

prompt_transport = "argv"              # "argv" | "stdin"
session_id_mode  = "assigned"          # "assigned" | "latest-in-cwd"

[harness.env]                          # optional extra environment
```

The user's description of the reference app's settings maps directly: _label_, _command_,
_prompt-only args_ → `prompt_args`, _resume args_, _fork args_, _prompt transport_, _restore
defaults_. `model_args` / `effort_args` are our addition so the composer's pickers work without
every harness needing hand-written templates.

### Templating rules

- Args are an **array**; each element is one argv entry. Placeholders are substituted _inside_ an
  element and never re-split, so a prompt with spaces, quotes or newlines is always exactly one
  argument. No shell is involved.
- The settings form may show/edit args as a single shell-like line for convenience, parsed with
  shlex rules into the array — but the array is what is stored and executed.
- Placeholders: `{prompt}` `{model}` `{effort}` `{session_id}` `{new_session_id}` `{workspace}`
  `{worktree}` `{branch}`.
- An arg group whose placeholder has no value is dropped whole (no model chosen → no `--model`).
- Final argv to start = `command` + `base_args` + `model_args` + `effort_args` + `session_args` +
  `prompt_args`; to resume or fork, `resume_args` / `fork_args` take the place of the last two.
  The session id has a group of its own so that an empty prompt drops only the prompt.
- Braces that are not one of our placeholders are literal, so JSON can be passed in an argument.
- Built-in definitions are compiled in. User edits are stored as overrides, so **Restore defaults**
  is just "delete the override", and new app versions can ship corrected defaults.

### Prompt transport

- **`argv`** — the prompt is substituted into `prompt_args`. Simple and reliable. Default.
- **`stdin`** — the harness is started without the prompt, which is then pasted into the terminal
  and submitted. For harnesses with no prompt argument.

  "Ready" means: the program has printed something, then stayed quiet for `stdin_ready_ms`
  (default 1500) — a TUI that has drawn itself and is waiting. We never parse what it printed. If
  it never settles, the prompt is pasted after 20 s anyway rather than lost; if it exits first,
  nothing is sent. The paste is bracketed when the program enabled bracketed paste (so newlines
  stay part of the message), otherwise sent as typed input; Enter follows 150 ms later, because
  some TUIs drop an Enter that arrives glued to a paste.

  Limitation: if the harness opens with a dialog (folder trust, login), the paste lands there.

Automatic fallback: a prompt longer than 24 000 characters on Windows (100 000 elsewhere) is
delivered over `stdin` even for an `argv` harness, because the OS would otherwise refuse to start
the process at all.

### Session ids

Resume and fork need the harness's own session id. Two strategies:

- **`assigned`** — we generate a UUID and pass it at start. Deterministic; preferred where supported.
- **`latest-in-cwd`** — the harness picks its own id, so we resume "the most recent session in this
  directory". This is safe _because every workspace has a unique worktree path_. Its limit: with
  several sessions in one workspace only the newest is addressable. Later we can recover the real
  id from the harness's session store.

## Verified defaults

Checked against the CLIs installed on the planning machine on 2026-09-17 (`--help` output, not yet
exercised end-to-end). Re-verify during M4; these flags move.

|                   | **Claude Code** 2.1.273                              | **Codex** 0.154.0                       | **Grok** 1.0.30                                      | **OpenCode** 1.18.31            |
| ----------------- | ---------------------------------------------------- | --------------------------------------- | ---------------------------------------------------- | ------------------------------- |
| command           | `claude`                                             | `codex`                                 | `grok`                                               | `opencode`                      |
| model             | `--model {model}`                                    | `-m {model}`                            | `-m {model}`                                         | `-m {model}` (`provider/model`) |
| effort            | `--effort {effort}`                                  | `-c model_reasoning_effort="{effort}"`  | `--reasoning-effort {effort}`                        | — (no flag)                     |
| effort values     | low, medium, high, xhigh, max                        | _verify_                                | _verify_                                             | n/a                             |
| prompt            | positional `{prompt}`                                | positional `{prompt}`                   | positional `{prompt}`                                | `--prompt {prompt}`             |
| assign session id | `--session-id {uuid}`                                | —                                       | `--session-id {uuid}`                                | —                               |
| resume            | `--resume {session_id}`                              | `resume {session_id}` / `resume --last` | `--resume {session_id}`                              | `--session {id}` / `--continue` |
| fork              | `--resume {id} --fork-session --session-id {new_id}` | `fork --last`                           | `--resume {id} --fork-session --session-id {new_id}` | `--continue --fork`             |
| session_id_mode   | assigned                                             | latest-in-cwd                           | assigned                                             | latest-in-cwd                   |

Notes:

- Codex `resume` / `fork` are **subcommands**, not flags — which is exactly why resume and fork are
  full arg lists rather than "extra flags appended to the start args". `resume --last` filters by
  cwd by default (there is an `--all` flag to disable that), which is what makes `latest-in-cwd` work.
- Claude and Grok both have their own `--worktree` flag. We don't use it: Yardsort owns worktree
  creation so behaviour is identical across harnesses.
- **Gemini CLI** (0.60.0) is also installed and fits the same shape — `-m`, `-i {prompt}` for
  "prompt then stay interactive", `--session-id`, `--resume latest`. A possible future default.
- Permission / approval modes (`--permission-mode`, `-a`, `--always-approve`, `--auto`) are
  deliberately **not** in the defaults. Users who want them add them to `base_args`.

### OMP, Cursor and Pi

Added on 2026-09-24, checked against OMP 18.1.19, Cursor Agent 2026.09.23-86fc751 and Pi
0.87.1 `--help`, plus the official [OMP CLI reference](https://omp.sh/docs/cli),
[Cursor parameters](https://cursor.com/docs/cli/reference/parameters) and
[Pi CLI reference](https://pi.dev/docs/latest/cli). These checks verify the flags, not a
provider-authenticated end-to-end conversation.

|                   | OMP                                               | Cursor                | Pi                                                  |
| ----------------- | ------------------------------------------------- | --------------------- | --------------------------------------------------- |
| command           | `omp`                                             | `cursor-agent`        | `pi`                                                |
| model             | `--model {model}`                                 | `--model {model}`     | `--model {model}`                                   |
| effort            | `--thinking {effort}`                             | —                     | `--thinking {effort}`                               |
| effort values     | off, minimal, low, medium, high, xhigh, max, auto | —                     | off, minimal, low, medium, high, xhigh, max         |
| prompt            | `-- {prompt}`                                     | `-- {prompt}`         | `-- {prompt}`                                       |
| assign session id | —                                                 | —                     | `--session-id {session_id}`                         |
| resume            | `--continue`                                      | `--continue`          | `--session {session_id}`                            |
| fork              | —                                                 | —                     | `--fork {session_id} --session-id {new_session_id}` |
| session_id_mode   | latest-in-cwd                                     | latest-in-cwd         | assigned                                            |
| write             | `--print --no-session -- {prompt}`                | `--print -- {prompt}` | `--print --no-session -- {prompt}`                  |

Cursor's installer also exposes `agent`; the more specific `cursor-agent` avoids confusion
with other tools and the `cursor` editor launcher. Users can override the command in settings.
OMP's documented fork option requires an agent session id, which Yardsort does not have, and
is absent from the checked version's help; its Fork template stays empty. Cursor has no
verified fork flag. Pi can choose the new id on a fork, so the fork stays addressable.
OMP and Pi drafting runs use `--no-session` to avoid replacing the latest interactive session.
Model names are free text for all three; no provider-specific model suggestions are imposed.

## Settings

**Settings → Harnesses** (`Mod+,`). The list shows every harness with a status dot (installed /
disabled / not found) and whether it is `modified` or `custom`. The form edits one definition:

- Argument groups are edited as one shell-like line each (`-c 'key="{effort}"'`). That is notation
  only — what is stored and run is the array, and no shell is involved. An open quote is reported
  on the field and blocks saving.
- **What will run** shows the exact `start`, `resume` and `fork` command lines for sample values,
  computed by the core from the unsaved form — the fastest way to debug a template.
- **Test launch** starts the unsaved definition, with no prompt, in the home directory.
- **Restore defaults** (built-ins) deletes the override. **Delete harness** removes a custom one.
- **Add custom harness** for anything else that runs in a terminal.

### The file

`settings.toml` lives in the OS config directory (next to the database when
`YARDSORT_DATA_DIR` is set). It is meant to be readable and hand-editable:

```toml
[workspaces]
worktree_root = "/data/worktrees"      # default: ~/yardsort
branch_prefix = "ys"                   # "" for none

[[harness]]                            # a built-in: only what differs is stored
id = "claude"
base_args = ["--append-system-prompt", "Be brief."]

[[harness]]                            # an id Yardsort does not ship is a custom harness
id = "aider"
label = "Aider"
command = "aider"
prompt_args = ["--message", "{prompt}"]
```

Unknown keys are ignored, so a file from a newer version still loads. It is written atomically.
A file that cannot be parsed is **never overwritten**: defaults are used, the problem is shown in
Settings, and the first save moves the file aside as `settings.toml.unreadable`. Comments are not
preserved when the app saves.
