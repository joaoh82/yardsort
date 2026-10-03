# 08 — Manual checklist

CI builds on Linux, macOS and Windows and runs the PTY host's integration tests there — real
processes in real PTYs, ConPTY included. What it cannot see is a window: rendering, keyboard
routing, a harness's first-run dialogs, or whether an agent is still working after you close the
app. This is the pass that covers that, and the record of having done it.

The roadmap owes it on **macOS and Windows** (M7; M1 wants the benchmark rows too, M4 wants the
harnesses exercised, M9 the daemon, M10 the `ys` command line, M11 the activity record). Linux is recorded in
[07-terminal-benchmarks](07-terminal-benchmarks.md).

Work on a throwaway profile so nothing here touches real projects:

```sh
YARDSORT_DATA_DIR=/tmp/ys YARDSORT_WORKTREE_ROOT=/tmp/ys-wt just dev
```

On Windows use a short path — `C:\ys` — rather than one under `%LOCALAPPDATA%`: worktrees of a
repository with a deep `node_modules` run into the 260-character limit.

Copy the table below into the results section, mark each row, and note anything surprising. A row
that fails is worth more than a row that passes: say what happened.

## 1 · It starts and says what it found

| ✓   | Check                                                                                                                                              |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | The welcome screen lists git and at least one agent as found. **Check again** re-reads without a restart.                                          |
|     | Status bar, bottom right: `env: login shell` (macOS) or `env: process` (Windows), a plausible PATH count, and **`daemon <pid>`**.                  |
|     | `daemon <pid>` is present, not `no daemon`. If it says `no daemon`, hover it — that tooltip is the finding.                                        |
|     | **Windows:** with Yardsort open, install an agent it did not find, then press **Check again**. It is found, without a restart.                     |
|     | On a fresh profile, `sqlite3 <profile>/yardsort.db 'PRAGMA user_version = 999'`, then start. **Yardsort cannot start** explains; closing it quits. |

The Windows row is a bug that shipped: a process's environment there is fixed when it starts, so
until 0.8.2 **Check again** re-read a stale copy and could never find something installed a
moment ago. `PATH` now comes back from the registry as well, which is where an installer writes.

`env: process` is correct on Windows and `env: login shell` on macOS; the reverse on either is a
bug. If macOS says `process`, the login-shell probe failed and harnesses installed through
mise/asdf/nvm/homebrew will not be found — which is the whole reason that probe exists.

## 2 · The terminal is a real terminal

| ✓   | Check                                                                                                   |
| --- | ------------------------------------------------------------------------------------------------------- |
|     | A shell tab (Mod+T) shows your prompt as it looks elsewhere — a themed prompt, truecolor, `ls` colours. |
|     | A full-screen TUI draws and redraws correctly: `vim`, `htop`, or a harness's own interface.             |
|     | Resizing the window reflows the TUI; nothing is left truncated or doubled.                              |
|     | Mouse reporting works where the program uses it (`htop` rows respond).                                  |
|     | Unicode and emoji line up: `printf 'aé中😀X\n'` puts the `X` in column 7 — 1 + 1 + 2 + 2.               |
|     | Paste of a multi-line block arrives as one paste, not as a submitted line per newline.                  |

## 3 · Keys go to the right place

The rule is in [shortcuts](../guide/shortcuts.md): Mod is `⌘` on macOS, `Ctrl+Shift` elsewhere,
and plain `Ctrl`+letter belongs to the program.

| ✓   | Check                                                                                       |
| --- | ------------------------------------------------------------------------------------------- |
|     | In a shell: plain `Ctrl+B`, `Ctrl+W`, `Ctrl+T`, `Ctrl+C` reach the program (on macOS too).  |
|     | Mod+T, Mod+W, Mod+B, Mod+Alt+B, Mod+, do their app action and do **not** reach the program. |
|     | Mod+C copies a selection and Mod+V pastes, inside a terminal.                               |
|     | Composer: `Enter` starts, `Shift+Enter` adds a line, `Esc` cancels.                         |

## 4 · Every built-in harness, with a real message

M4 verified the flags against `--help` and Claude Code by hand on Linux. This is the row that is
missing: each harness actually started, on this OS, with a prompt that reaches it.

| ✓   | Harness     | Check                                                                         |
| --- | ----------- | ----------------------------------------------------------------------------- |
|     | Claude Code | Starts from the composer with a message, and the message is what it works on. |
|     | Codex       | Same.                                                                         |
|     | Grok        | Same.                                                                         |
|     | OpenCode    | Same.                                                                         |

On Windows these are often npm `.cmd` shims, which only `cmd.exe` can spawn — a harness that
"is not found" while it works in your shell is that, and worth recording.

**The first-run dialog.** A harness that has never seen this folder may ask something before it is
ready — Claude Code asks "Quick safety check: is this a project you trust?". With the prompt on
argv that is harmless. With `prompt_transport = "stdin"` the message would be typed into that
dialog, which is [open question 11](06-open-questions.md). Try one harness switched to `stdin` in
a folder it has not seen, and record what happens.

## 5 · Sessions and the daemon

This is what 0.5.0 added, and the part CI can least reach: named pipes and `DETACHED_PROCESS` on
Windows, a socket file on macOS.

| ✓   | Check                                                                                                     |
| --- | --------------------------------------------------------------------------------------------------------- |
|     | Switch workspaces and back: the terminal is repainted as it was, scrollback included.                     |
|     | Give an agent a long task, **close the window**. It asks, naming the agents. **Cancel** closes nothing.   |
|     | Close again, **Leave them running**. Reopen: the agent is still working, the screen is where it got to.   |
|     | That conversation is **not** listed as _interrupted_ — nothing interrupted it.                            |
|     | An agent that finished while you were away wears the ringed dot.                                          |
|     | Close with only a shell running: no dialog. A shell at a prompt is not work.                              |
|     | Kill the app outright (Task Manager / `kill -9`). Reopen: same as above, and the daemon pid is unchanged. |
|     | Close and **Stop them**: agents and daemon both gone, and the conversation ends with an exit code.        |
|     | Quit with nothing running, wait ~15 s: the daemon has exited by itself.                                   |
|     | Resume an ended conversation, and Fork a running one.                                                     |

If the status bar says `no daemon`, the tooltip and `daemon.log` next to the database say why.
A unix socket path is limited to about a hundred characters, so a deep `YARDSORT_DATA_DIR` on
macOS is one plausible cause.

## 6 · The `ys` command line

Everything here has only ever been run by a human on Linux. It compiles and its tests pass on all
three platforms, but raw mode, the detach key and resize forwarding are not things a test can
check — and `ys` works out where your data lives by its own copy of Tauri's rules, which has
never met a database that macOS or Windows created.

`ys` is a separate download from the [releases](https://github.com/joaoh82/yardsort/releases), not
part of the app bundle. Put it on your `PATH` first. On macOS it is signed and notarized, but no
ticket can be stapled to it, so what a browser-downloaded copy does on first run is itself the
first row.

Run it against the same throwaway profile the app is using:

```sh
ys --data-dir /tmp/ys doctor        # or C:\ys on Windows
```

### It runs, and it is looking at the right place

| ✓   | Check                                                                                                                                        |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------- |
|     | macOS, downloaded with a browser, online: it runs — no _"ys" Not Opened_ and no `xattr` needed. Downloaded with `curl`: it just runs.        |
|     | Windows: `ys --help` **prints something**. Silence means the console is missing, which is the bug a separate binary exists to avoid.         |
|     | `ys doctor` with no `--data-dir`, while the app is closed: the data directory it names is the one the app really uses.                       |
|     | That same `doctor` says `database … (found)` and a plausible project count — not `NOT FOUND`, and not zero when you have projects.           |
|     | The app prints no `warning: the data directory is …` line at startup. If it does, `ys` and the app disagree and that warning is the finding. |

The third and fourth rows are the important ones on each platform. `ys` resolves the directory
itself rather than asking Tauri, so a difference here is silent: it would report a Yardsort you
have never used rather than an error.

### It sees the same world the window does

| ✓   | Check                                                                                                                                    |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------- |
|     | `ys project list` and `ys workspace list` match what the sidebar shows, paths included.                                                  |
|     | `ys workspace new <project> "a small task"` creates a branch and worktree, and starts an agent.                                          |
|     | That workspace appears in the app — immediately if it is open, on next launch if it is not.                                              |
|     | The agent it started is still working after `ys` has returned: `ys session list` says `running`.                                         |
|     | `ys workspace new --harness <something misspelt>` creates **nothing** — no branch, no folder, no row.                                    |
|     | `ys workspace delete <name>` removes the folder and the row, and the branch is still there. A dirty worktree is refused until `--force`. |

### Attaching

| ✓   | Check                                                                                                                   |
| --- | ----------------------------------------------------------------------------------------------------------------------- |
|     | `ys attach` repaints the agent's screen where it got to, colours and all.                                               |
|     | What you type reaches the agent, and its replies come back.                                                             |
|     | A full-screen TUI draws correctly, and **resizing your terminal window** reflows it within a second.                    |
|     | Plain `Ctrl+C` reaches the agent rather than killing `ys`.                                                              |
|     | `Ctrl-]` detaches. The agent is **still running** afterwards (`ys session list`).                                       |
|     | Your shell is normal again after detaching: echo works, arrow keys work, no stray escape sequences.                     |
|     | Re-attach: the screen is repainted including what happened while you were away.                                         |
|     | With the app open on the same session: both show the same output, and either can type.                                  |
|     | Windows: all of the above in **Windows Terminal** and again in the old **conhost** window, which differ in VT handling. |

Known and not a bug: attaching matches the session to your terminal, and the app treats that same
size as its own setting, so the two can tug at each other while both are attached. The app sets it
again when you next focus the tab.

Known limitation worth confirming rather than discovering: if `ys` is killed from elsewhere
(`kill` from another terminal, closing the window it runs in), the terminal is left in raw mode —
`reset` restores it. Only a clean detach or exit puts it back.

### Reading a screen

| ✓   | Check                                                                                                                                   |
| --- | --------------------------------------------------------------------------------------------------------------------------------------- |
|     | `ys logs` prints the session's output as plain text — readable, no escape sequences, scrollback included.                               |
|     | `ys logs --lines 10` gives the last ten lines; `ys logs --raw \| cat` shows the escapes are really there.                               |
|     | Let an agent finish, then `ys logs` it **while the app is open**: its last screen is still readable.                                    |
|     | Close the app, wait ~15 s with nothing running, then `ys logs`: it says the daemon is gone and screens with it — not "printed nothing". |

That last row is the design, not a failure: screens live in the background process and are never
written to disk. It is on the list because the wrong message here would read as data loss.

## 7 · Changes, files and the editor

| ✓   | Check                                                                               |
| --- | ----------------------------------------------------------------------------------- |
|     | While an agent edits, the changes list updates by itself, and an open diff with it. |
|     | The diff and the read-only file viewer render correctly, including a large file.    |
|     | The file tree opens folders; ignored files appear only when asked for.              |
|     | **Open in editor** opens the right file in your editor.                             |
|     | A repository with a big `node_modules` stays responsive.                            |

## 8 · Committing, pushing, opening a pull request

Needs a project with a real remote you may push to. Do the first half **without `gh` on `PATH`**
(or logged out of it) and the second half with it: the two paths are different code.

| ✓   | Check                                                                                                                                   |
| --- | --------------------------------------------------------------------------------------------------------------------------------------- |
|     | With uncommitted changes, the message box appears; **Commit N files** is dead until a message is typed.                                 |
|     | Pressing it asks first, naming the files, the message and the branch. Answering no commits nothing and keeps the message.               |
|     | Commit: the list empties, the message box goes, and `git log` in a shell tab shows the commit with every file, untracked ones too.      |
|     | A message starting with `--` commits as that message rather than being read as an option.                                               |
|     | **Push N commits** appears; pushing sets the upstream and the button goes. `git status` agrees.                                         |
|     | Without `gh`: **Open pull request** pushes and opens the forge's form in a browser, both branches filled in.                            |
|     | With `gh`: the dialog's title is the oldest unpushed commit; opening lands the browser on the real pull request.                        |
|     | Its number then appears on the workspace's row in the sidebar, and at the right of the panel foot.                                      |
|     | Pressing the badge opens the pull request in a browser; pressing the row still opens the workspace.                                     |
|     | Let CI run: the row goes amber while checks run, then green or red. Coming back to the window catches up within a minute.               |
|     | Merge the pull request on the forge: the row says **merged**.                                                                           |
|     | A project whose remote is a path on disk offers **Commit** and **Push** but no pull request button at all.                              |
|     | On a machine with no `user.name` / `user.email`: the commit button is disabled and says which two commands to run, rather than failing. |

### Writing with a model

| ✓   | Check                                                                                                           |
| --- | --------------------------------------------------------------------------------------------------------------- |
|     | With an agent installed, **✦** beside the commit box writes a message into the box. Nothing is committed by it. |
|     | The message box takes a body: Enter makes a newline, `Ctrl+Enter` / `⌘Enter` commits.                           |
|     | **✦** in the pull request dialog fills the title _and_ the description, and opens nothing.                      |
|     | Typing your own message, then pressing ✦ and having it fail (log the agent out): what you typed is still there. |
|     | With no agent able to write and no Anthropic key, no ✦ appears anywhere and nothing complains.                  |
|     | Settings → Assist: switching "Offer to write them for me" off removes both buttons.                             |
|     | A custom harness with **Write args** filled in is used; one without falls through to the key, or to nothing.    |

## 9 · Deleting nothing by accident

| ✓   | Check                                                                                                                                   |
| --- | --------------------------------------------------------------------------------------------------------------------------------------- |
|     | Delete a workspace with uncommitted changes: it names what would be lost and takes a second yes.                                        |
|     | Answer no: the worktree and the changes are still there.                                                                                |
|     | Delete one that is clean: the folder goes, **the branch stays**, and it can be opened again from the composer's _Open existing branch_. |
|     | Archive and restore: the conversations come back with the workspace.                                                                    |

## 10 · The numbers

```sh
scripts/bench/run.sh
```

Add the rows to [07-terminal-benchmarks](07-terminal-benchmarks.md) in the shape the Linux section
uses — both workloads, both renderers — with the machine, OS, webview version and display scaling.
This is the half of M7's box that is not a judgement call.

## 11 · Activity

What CI cannot see here is a window closing and reopening, and the daemon on a platform's own
process model. Switch **Show the activity timeline** on in Settings → General first.

| ✓   | Check                                                                                                                                                                                                                                                      |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start an agent from the composer. **Activity** in the footer shows `<agent> started` with the model, source `yardsort/lifecycle`.                                                                                                                          |
|     | Quit the agent (`/exit` or `Ctrl+D`). The timeline gains `exited` with _seen as it happened_; the Previous sessions entry shows a clean end, not interrupted.                                                                                              |
|     | Start an agent, close the window with **Leave them running**, end the agent from another terminal (`ys attach`, then quit it), reopen. It is listed ended with its exit code; the timeline says `via spool` or _found already ended_, never _interrupted_. |
|     | The same, but wait more than ten seconds after the agent ends before reopening, so the daemon has exited. Same result.                                                                                                                                     |
|     | `ys workspace new … --harness <one that exits>` then `ys activity list`: `process.started` and `process.exited`, workspace named, exit code right.                                                                                                         |
|     | Resume a conversation: `resumed` on the timeline; Fork one: `forked`, with the source's id.                                                                                                                                                                |
|     | Settings → General: the counts match `ys doctor`; **Clear all recorded activity** empties both; a run still going survives the clear.                                                                                                                      |
|     | Switch **Record when agents start and exit** off, start a shell: nothing new on the timeline, and `YARDSORT_RUN_ID` is unset in it while `YARDSORT_WORKSPACE_ID` is set.                                                                                   |
|     | **Windows:** the spool directory is under `%APPDATA%\dev.yardsort.app\activity\spool`, entries appear there while the window is closed, and are gone after reopening.                                                                                      |

## 12 · Claude Code reporting

Needs a real Claude Code (2.1.x) logged in. Switch on **Record when agents start and exit**,
**Show the activity timeline** and **Capture what Claude Code reports** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                             |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start Claude Code from the composer with a message. The timeline's `claude started` row says _reporting through hooks_; within a second, `agent session started` and `prompt submitted · N characters` appear, source `claude/hook`, without pressing Refresh.                    |
|     | Ask it to read and edit a file. `Read started` / `Read done · <relative path> · N ms`, then `Edit …`. The path is relative to the workspace. Ask it to run a command: `Bash started` / `Bash done`, and no command text anywhere on the timeline or in `ys activity list --json`. |
|     | Let it ask permission for something. `permission asked for Bash` and/or `agent raised a notification · permission_prompt` appear while it waits.                                                                                                                                  |
|     | Close the window with **Leave them running**, send it another message from `ys attach`, reopen. The rows it reported meanwhile are there; Settings → General shows nothing waiting in the inbox.                                                                                  |
|     | Resume the conversation: `agent resumed its session · N tokens of context`. Fork it: `agent forked its session` under the new tab.                                                                                                                                                |
|     | Your own `~/.claude/settings.json` is byte-for-byte unchanged, and a Claude Code started from a plain terminal reports nothing.                                                                                                                                                   |
|     | Switch **Capture** off; start Claude Code again: no `claude/hook` rows, and its command line has no `--settings`.                                                                                                                                                                 |
|     | Add `--settings ~/mine.json` to the Claude harness's arguments in Settings → Harnesses with Capture on: the launch works, no hook rows, and `hooks_not_armed` appears among the counters.                                                                                         |
|     | **macOS / Windows:** all of the above; on Windows the inbox is under `%APPDATA%\dev.yardsort.app\activity\inbox` and the hook is `yardsort.exe --yardsort-hook claude …`.                                                                                                         |

## 13 · Codex reporting

Needs a real Codex (0.156.x) logged in. Switch on **Record when agents start and exit**, **Show
the activity timeline** and **Capture what Codex reports** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                        |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start Codex from the composer with a message asking it to create a file and run a command. The `codex started` row says _reporting through notify_. When the turn ends: `agent session started`, `prompt submitted`, `shell done · exit 0`, `file added · <path>`, `tokens used`, `agent finished its turn · N s`, all `codex/session_file`. |
|     | `ys activity list --json`: no command text, no file content, no message anywhere in the payloads.                                                                                                                                                                                                                                            |
|     | Send a second message. Only that turn's rows are added; the first turn's are not repeated.                                                                                                                                                                                                                                                   |
|     | Resume the conversation and send a message: a new `agent session started` under the new run, then the turn.                                                                                                                                                                                                                                  |
|     | `~/.codex/config.toml` and `~/.codex/hooks.json` are byte-for-byte unchanged. If `config.toml` has a `notify` of your own, it still fires.                                                                                                                                                                                                   |
|     | Switch **Capture** off; start Codex again: no `codex/` rows, and its command line has no `notify=`.                                                                                                                                                                                                                                          |
|     | **macOS / Windows:** all of the above; on Windows the `notify` program is `yardsort.exe --yardsort-hook codex …` and the session file is under `%USERPROFILE%\.codex\sessions`.                                                                                                                                                              |

## 14 · OpenCode reporting

Needs a real OpenCode (1.18.x) logged in. Switch on **Record when agents start and exit**, **Show
the activity timeline** and **Capture what OpenCode reports** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start OpenCode from the composer with a message asking it to create a file and run a command. The `opencode started` row says _reporting through plugin_. Rows arrive as it works: `agent session started`, `prompt submitted`, `write started` / `write done · <path>`, `file changed`, `bash done · exit 0`, `tokens used`, `agent finished its turn`, all `opencode/plugin`. |
|     | `ys activity list --json`: no message text, no command, no output, no file content anywhere in the payloads.                                                                                                                                                                                                                                                                    |
|     | Ask it to read a file that does not exist: `read failed · <path> · N ms`.                                                                                                                                                                                                                                                                                                       |
|     | Ask it for something that needs permission: `permission asked for bash` while it waits.                                                                                                                                                                                                                                                                                         |
|     | `~/.config/opencode/opencode.json` is byte-for-byte unchanged, and a plugin of your own under `~/.config/opencode/plugins/` still runs.                                                                                                                                                                                                                                         |
|     | Switch **Capture** off; start OpenCode again: no `opencode/` rows, and `OPENCODE_CONFIG_CONTENT` is unset in a shell it opens.                                                                                                                                                                                                                                                  |
|     | Add `--pure` to the OpenCode harness's arguments with Capture on: the launch works, no plugin rows, and `hooks_not_armed` appears among the counters.                                                                                                                                                                                                                           |
|     | **macOS / Windows:** all of the above; on Windows the plugin is `%APPDATA%\dev.yardsort.app\activity\hooks\opencode.js` and spawns `yardsort.exe --yardsort-hook opencode …`.                                                                                                                                                                                                   |

## 15 · Grok reporting

Needs a real Grok (1.0.x) logged in. Switch on **Record when agents start and exit**, **Show the
activity timeline** and **Read what Grok records** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                      |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|     | Start Grok from the composer with a message asking it to create a file and run a command. The `grok started` row says _reporting through session_file_. Within a few seconds of each step: `agent session started`, `agent started a turn`, `write started` / `write done · N ms`, `run_terminal_command done`, then `agent finished its turn` and `tokens used`, all `grok/session_file`. |
|     | `ys activity list --json`: no command, no path, no message anywhere in the payloads.                                                                                                                                                                                                                                                                                                       |
|     | Ask it for something that needs your permission and wait before answering: `run_terminal_command allow · you took N s`.                                                                                                                                                                                                                                                                    |
|     | Interrupt a turn with Escape: `agent's turn was interrupted`.                                                                                                                                                                                                                                                                                                                              |
|     | `~/.grok/config.toml` and `~/.grok/hooks/` are byte-for-byte unchanged; `ls ~/.grok/hooks` shows nothing of Yardsort's.                                                                                                                                                                                                                                                                    |
|     | Switch **Read** off; start Grok again: no `grok/` rows.                                                                                                                                                                                                                                                                                                                                    |
|     | **macOS / Windows:** all of the above; on Windows the directory is under `%USERPROFILE%\.grok\sessions`.                                                                                                                                                                                                                                                                                   |

## 16 · OMP and pi reporting

Needs a real OMP (18.x) or pi (0.87+) with a provider configured. Switch on **Record when agents
start and exit**, **Show the activity timeline** and **Capture what OMP reports** / **Capture
what pi reports** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start OMP from the composer with a message asking it to create a file and run a command. The `omp started` row says _reporting through extension_. As it works: `agent session started`, `agent started a turn`, `write started` / `write done · hello.txt · N ms`, `bash done`, `agent finished its turn · N tokens`, all `omp/extension`. Type a second message: `prompt submitted · N characters` (the opening one, given on the command line, raises no input event). |
|     | `ys activity list --json`: no command, no message, no file contents anywhere in the payloads; `bash`'s row has no command line.                                                                                                                                                                                                                                                                                                                                           |
|     | OMP: ask for something that needs your permission: `bash asked`, then `bash allow` or `bash deny`.                                                                                                                                                                                                                                                                                                                                                                        |
|     | pi: switch models mid-session (`/model`): `model switched to …`.                                                                                                                                                                                                                                                                                                                                                                                                          |
|     | `~/.pi/agent/extensions/`, `~/.omp/` and the project's `.pi/` are unchanged; your own extensions still load (their startup line still prints).                                                                                                                                                                                                                                                                                                                            |
|     | Add `--trusted-extension x` to the harness's arguments in Settings → Harnesses; start it: no extension is given, `ys doctor` counts one `hooks_not_armed`.                                                                                                                                                                                                                                                                                                                |
|     | Switch **Capture** off; start it again: no `omp/` or `pi/` rows.                                                                                                                                                                                                                                                                                                                                                                                                          |
|     | **macOS / Windows:** all of the above; on Windows the extension is written under `%APPDATA%\yardsort\activity\hooks\`.                                                                                                                                                                                                                                                                                                                                                    |

## 17 · Cursor reporting

Needs a real Cursor agent CLI, logged in (`cursor-agent login`). Recorded 2026-09-26 on Linux
(`scripts/record-cursor.sh`; the launch environment reached the hooks). Switch on **Record when
agents start and exit**, **Show the activity timeline** and **Capture what Cursor reports** in
Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                                                            |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start Cursor from the composer with a message asking it to create a file and run a command. The `cursor started` row says _reporting through hook_. As it works: `agent session started`, `Write done · hello.txt`, `file changed · hello.txt`, `shell done · N ms`, then `agent session ended`, all `cursor/hook`; `file changed` lands before `Write done`. No `prompt submitted` row: that hook can block, so it is not used. |
|     | `ys activity list --json`: no command, no message, no file contents, no edit text anywhere in the payloads.                                                                                                                                                                                                                                                                                                                      |
|     | `agent finished its turn` does not appear: this build does not deliver `stop` to a plugin's hooks (nor `afterAgentResponse`, nor `subagentStop`). Note if a newer build does.                                                                                                                                                                                                                                                    |
|     | Ask it for something that spawns a subagent (a broad exploration): the subagent runs — nothing of Yardsort's answers `subagentStart` — and `subagent stopped` follows.                                                                                                                                                                                                                                                           |
|     | `~/.cursor/hooks.json` and the project's `.cursor/` are unchanged; your own hooks still run.                                                                                                                                                                                                                                                                                                                                     |
|     | Switch **Capture** off; start it again: no `cursor/` rows.                                                                                                                                                                                                                                                                                                                                                                       |
|     | **macOS / Windows:** all of the above; on Windows the hook command runs through PowerShell, so a data directory with spaces in its path is the case to try.                                                                                                                                                                                                                                                                      |

## 18 · Reported writes beside the diff

Needs one reporting agent; Claude Code is the most direct (`Write` and `Edit` are reported as
tool calls with the file). Switch on **Record when agents start and exit**, **Show the activity
timeline** and **Capture what Claude Code reports** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start Claude Code from the composer and ask it to _use its Write tool_ to create `hello.txt` — asked plainly, it tends to run `echo > hello.txt`, and a shell command reports no file: `Bash done`, and the file gets a _dashed_ badge, since its time falls inside that command's window; hover it for _Yardsort read the time on the file, not who wrote it_. As the `Write done · hello.txt` row lands, the Changes list's `hello.txt` gains a **claude** badge without a refresh; hovering it says _claude reported writing this file once, last at …_ and ends with the line about git. |
|     | A line above the list reads _Every changed file was reported written by an agent._ Now edit a second file yourself: the line becomes _Agents reported writing 1 of 2 changed files_ and names you, a script, a command the agent ran, or a non-reporting run as the alternatives. Your file has no badge.                                                                                                                                                                                                                                                                                    |
|     | Open `hello.txt` and **Expand**: the header says _reported by claude · 1 write · HH:MM_. Open your own file and expand: _no agent reported writing this_.                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|     | In the Activity panel, the `Write done · hello.txt` row has a **Show diff** button; pressing it opens that diff below the list. A `Bash done` row has none.                                                                                                                                                                                                                                                                                                                                                                                                                                  |
|     | Switch **Capture** off and start a second Claude Code; ask it to edit `hello.txt`. The badge stays (the first run reported) and the line now counts _one of the 1 agent run here that was not reporting_.                                                                                                                                                                                                                                                                                                                                                                                    |
|     | Clear the activity: every badge and the line go; the list reads as it did before this section.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
|     | **macOS / Windows:** the first three rows; the badge's path is what the Changes list shows (forward slashes on Windows too).                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

## 19 · Handing off

The story stage 4 was written for: Claude, then Codex from the packet. Needs both installed;
capture on for Claude Code makes the packet worth reading, and is what the second half checks.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                      |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|     | Start Claude Code from the composer with a real task, let it write a file with its Write tool and run a command, then press **Hand off…** in the tab bar. The composer opens here, headed _Hand off in <project>_, with a line saying what the text is and is not, and the message box filled.                                             |
|     | Read the message: your task quoted whole; the branch and its base; each changed file with the same words as the Changes list's badges; Claude's run with its model, duration, exit, turns, tools by count, failures with file, files reported written; _What is not here_ with the event count and the `ys activity list` line. UTC times. |
|     | Edit a line, pick **codex**, press Enter. Codex starts in the same workspace, gets the edited packet as its first message, and its tab reads _Handoff_. No new workspace or branch.                                                                                                                                                        |
|     | **Assist**, if on: the review still judges against your original task, not the packet (the line above the change list names the task). `ys session list`: the handoff session has the title _Handoff_ and the first session keeps its title.                                                                                               |
|     | Press **Hand off…** again: the packet now has two runs, the Codex one titled by its harness, and the task section still quotes only your first message.                                                                                                                                                                                    |
|     | Capture off for Claude, fresh workspace, one Claude run, then Hand off: the run's entry says only that it ran, and _What is not here_ says no run was reporting.                                                                                                                                                                           |
|     | **Assist on** (review switch, key in force): Hand off… again. The changed files are in Assist's order — on task first, unrelated last — each with its word, and a line says it is a judgment. `ys workspace handoff <name>` prints the same packet without that order and without that line.                                               |
|     | **macOS / Windows:** the first three rows; on Windows a long packet goes over stdin (the paste), so watch that it arrives whole in Codex.                                                                                                                                                                                                  |

## 20 · Project memory

Needs one agent that can run a shell command (any built-in one) and `ys` on its `PATH` — Settings
→ General → Command line installs it.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                         |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Open **Memory…** from a project's menu. Write "The tests need TZ=UTC." and press **Add**: it appears under **Approved**, from the user.                                                                                                                                                                                                                       |
|     | Start an agent in the project and ask it to run `ys memory propose "Run the linter before committing."`. It answers that the proposal waits for the user. Within a moment of refocusing the window, the project's row shows **1**; the menu reads **Memory… (1 waiting)**; the proposal is under **Waiting for you**, from that agent in that workspace.      |
|     | Ask the agent to run `ys memory list`: only the approved entry. Ask it to run `ys memory approve x`: an error, no such command.                                                                                                                                                                                                                               |
|     | **Approve** the proposal. Tick **Give this project's agents its memory**. Open the composer in the project: a line under the pickers says 2 approved entries go after the message; **Show** reads them, cited. Start an agent with "Say what you were told about this project": it quotes both. `ys session list`: its title is your message, not the memory. |
|     | Untick the composer's box and start another: it has no memory section. Start one from the tab bar with no message: nothing added.                                                                                                                                                                                                                             |
|     | **Revoke** one entry and start again: only the other is given. **Restore** brings it back. **Edit** one: the new text is given.                                                                                                                                                                                                                               |
|     | **Hand off…** in a workspace of the project: the packet has a _Project memory_ section before _What is not here_.                                                                                                                                                                                                                                             |
|     | **Assist**, with a key: switch on **Check memory proposals…**, have the agent propose "The tests need TZ=UTC set." — it is marked **repeats an entry**.                                                                                                                                                                                                       |
|     | **macOS / Windows:** the first two rows; on Windows the agent's `ys` is `ys.exe` from Settings → General.                                                                                                                                                                                                                                                     |

## 21 · Outcomes

Needs a project with a few worktree workspaces; the GitHub CLI for the pull-request rows.

| ✓   | Check                                                                                                                                                                                                                                                                      |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Delete a workspace: the sidebar asks _How did fix-login go?_. **Partly**: the question goes. Archive another and press **×**: it goes, unanswered.                                                                                                                         |
|     | **Outcomes…** from the project menu: both attempts are listed, the deleted one with its task and agents though the workspace is gone, one _partly_, one _no outcome yet_ and _archived_. Click **Kept** on the archived one, then **Kept** again: the label is taken back. |
|     | In a workspace, commit something and look again: _ahead of main_. Fast-forward `main` to the branch by hand and look again: _merged into main_ and _kept — merged, not labelled_. A branch with no commits never reads as merged.                                          |
|     | With `gh`: open and merge a pull request (squash) for a workspace's branch, then look again: _PR #N merged_, and kept from the merge. A closed, unmerged pull request shows and counts for nothing.                                                                        |
|     | The composer, with Claude picked: _too few to say yet_ until Claude has five attempts with an outcome, then _kept N of M_. Pick an agent with no attempts: no line.                                                                                                        |
|     | **macOS / Windows:** the first two rows.                                                                                                                                                                                                                                   |

## 22 · Workflow runs

Needs `ys` on your `PATH`, a project with a worktree workspace, and Claude or another agent. Save
this as `ping-agent.yaml` in the profile's `workflows` folder (`ys workflow list` prints where):

```yaml
id: ping-agent
name: Ping an agent
version: 1
trigger:
  kind: manual
inputs:
  - id: who
    kind: harness
    required: true
steps:
  - id: start
    action: start_session
    harness: "{{ inputs.who }}"
    prompt: List the files in this folder, then wait.
  - id: settle
    action: wait_session
    needs: [start]
    session: "{{ steps.start.session }}"
    timeout: 10m
  - id: nudge
    action: send_to_session
    needs: [settle]
    session: "{{ steps.start.session }}"
    prompt: Now count them.
  - id: tell
    action: notify
    needs: [nudge]
    title: "{{ inputs.who }} counted the files in {{ workspace.name }}"
```

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | With Yardsort closed, `ys workflow run ping-agent --workspace <name> --input who=claude`: _Yardsort is not running_, and `ys workflow runs` lists nothing.                                                                                                                                                                                                                                      |
|     | Open Yardsort, with another tab in front in that workspace, and run it again. Within a second a new Claude tab appears there; the tab in front stays in front.                                                                                                                                                                                                                                  |
|     | When Claude has listed the files and gone quiet, _Now count them._ is typed into it and submitted, never while it is still printing; then a system notification says it counted the files.                                                                                                                                                                                                      |
|     | `ys workflow runs --run <id>`: every step _succeeded_. Run it again at once: refused, naming the run.                                                                                                                                                                                                                                                                                           |
|     | Run it again and `ys workflow cancel <id>` while it waits: the run is _cancelled_, nothing is typed into Claude afterwards, and Claude keeps running.                                                                                                                                                                                                                                           |
|     | Run it again and quit Yardsort while `settle` waits (**Keep running**). Open it again: the run carries on, and finishes when Claude reports its turn.                                                                                                                                                                                                                                           |
|     | Code review, on a throwaway GitHub repository with `gh` logged in: an agent in a workspace opens a pull request and stays running. `ys workflow run code-review --input reviewer=codex`: a Codex tab opens, reads and runs the checks, edits nothing, and posts a review; within 30 s of it appearing on GitHub a notification says so, and the first agent is told, with the link, once quiet. |
|     | In a workspace with no pull request, or with `gh` logged out: `ys workflow run code-review` is refused, saying which, and nothing is queued.                                                                                                                                                                                                                                                    |
|     | The sidebar's **Workflows** section lists **Request code review**; open it: the chart shows five steps, the last two side by side, and the file is read-only with **Customize** and **Duplicate**. **Customize**: the row now says it is your copy, the file is editable, and **Reset to built-in** asks before deleting it.                                                                    |
|     | Type `oops: 1` into your copy: within a moment the line is marked in the gutter and listed under the editor, **Run…** is disabled, and the row shows **!**; **Revert** clears it. Type a real change and **Save**: the dot on the row goes.                                                                                                                                                     |
|     | **+**: a new workflow from the example; **Save** writes `<id>.yaml` in the workflows folder and the row appears. **Delete** asks, and a **No** leaves the file.                                                                                                                                                                                                                                 |
|     | From a workspace's **⋯** menu, **Run workflow…** → `ping-agent`, an agent picked, **Run**: the run appears at the top of the workflow's runs within a second, its steps moving on as in the rows above, and the chart's boxes colour with it. **Cancel run** while it waits: _cancelled_.                                                                                                       |
|     | **Request code review…** on a workspace with no pull request: the dialog says so and **Run** stays disabled.                                                                                                                                                                                                                                                                                    |
|     | **macOS / Windows:** the first three rows, and that the notification shows.                                                                                                                                                                                                                                                                                                                     |

## 23 · Pull requests

Needs the GitHub CLI logged in, and a project on GitHub with a few pull requests: one of your
own with a workspace, one somebody else opened, one from a fork, one merged, one closed. A
throwaway repository is the place for the rows that merge and close.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | **Pull requests** at the top of the sidebar shows how many are open. Press it: the list replaces the center panel, most recently updated first, each row with project, number, state, author, checks as passed/total, age and lines. Press it again, or a workspace: the panel goes back.                                                                                                                                                                       |
|     | The tabs: **Open** to start with, **Merged** and **Closed** each show theirs, **All** everything. Pick an author, then a review status: the list narrows by both. Quit and reopen: the tab and both filters are as they were; the search box is empty. **Clear filters** puts back everything but the tab.                                                                                                                                                      |
|     | **Awaiting review from you** shows the pull requests where a review was asked of you by name, and not the ones asked of a team you are in.                                                                                                                                                                                                                                                                                                                      |
|     | Press a row: details open beside the list. Its description appears within a second, as Markdown; its checks are listed with failures first, and pressing one opens its run in the browser; its reviewers and the conversation follow, oldest first. **↑**/**↓** still move in the list; **Esc** closes the details and the row keeps the focus. **⇤** hides the list, **⇥** brings it back, and dragging the divider resizes. Narrow the window: the two stack. |
|     | A description with a screenshot and a link: the image is not loaded — it is a link, _[image: …] ↗_ — and the link opens in the browser, with the Yardsort window staying where it is. A description from a template: its `<!-- comments -->` are not shown.                                                                                                                                                                                                     |
|     | With the details open, comment on the pull request from the browser: within about a minute the comment is in the conversation, without pressing anything. Push a commit that starts CI: the checks line changes as they run and finish.                                                                                                                                                                                                                         |
|     | **Code**: within a few seconds the changed files are listed with their kinds and line counts, and `git branch -a` and `git status` in the project are exactly as before. Press a file: its diff, highlighted, unchanged stretches folded. **Side by side** makes two panes, and the changes panel's own viewer is now side by side too. **‹ ›** and the **Files** box move between files; **← All files** goes back.                                            |
|     | Code on a pull request merged weeks ago, and on one from a fork: both show their files. Push a commit to an open one with Code showing: within about a minute the list of files follows. Go to another pull request: it opens on Code too.                                                                                                                                                                                                                      |
|     | With no network, Code on a pull request not opened before: what git said, **Retry**, and **See the diff on GitHub**. On one opened before: its files, at once.                                                                                                                                                                                                                                                                                                  |
|     | Under the conversation, write a comment and press **Comment**: it is on GitHub within a second, and in the conversation without pressing anything else. With `gh` logged out, the words stay in the box and the reason is shown.                                                                                                                                                                                                                                |
|     | On a pull request with comments on lines: each file in Code says how many threads it has; open one and each thread sits under its lines, with its replies; one on the old text sits in the left pane side by side; one the diff moved on from is listed under the viewer. Comment on a line from the browser: within a minute it is there.                                                                                                                      |
|     | Select lines in the diff and press **Note on lines…**. With the pull request's workspace running an agent that is quiet: the dialog names it, **Send to agent** types the note into it and lands you on that tab, and nothing is on GitHub. With the box ticked: the same note is a comment on those lines on GitHub too. With the agent busy: the dialog says so and only **Post on GitHub only** works.                                                       |
|     | Expand a collapsed stretch of the diff, select lines there, tick the box and **Send to agent**: GitHub refuses (the line is not in the diff), the dialog says so, and the agent has not been given the note. Untick, Send again: the agent gets it once. Press **Comment** under a conversation twice quickly: one comment on GitHub, not two.                                                                                                                  |
|     | On a pull request with no workspace: **Start a workspace with it** opens the composer on its branch with the note as the message; Start makes the workspace and the agent's first message is the note.                                                                                                                                                                                                                                                          |
|     | On somebody else's pull request, **Merge ▾** → **Squash and merge**: the confirmation names them — _… opened it, not you_ — and the commit. **Cancel**: nothing happens on GitHub. Again, and confirm: it merges, the row turns **Merged**, and its branch is still on GitHub.                                                                                                                                                                                  |
|     | Push a commit to a pull request from elsewhere, then merge it from a list that has not refreshed: refused, saying it changed; nothing merges.                                                                                                                                                                                                                                                                                                                   |
|     | A draft, and one that conflicts: **Merge ▾** is greyed out and its tooltip says which.                                                                                                                                                                                                                                                                                                                                                                          |
|     | **Close**, cancel: still open. **Close**, confirm: closed, branch kept, no comment posted. On the **Closed** tab, **Reopen**, confirm: open again.                                                                                                                                                                                                                                                                                                              |
|     | **Start workspace** on a pull request whose branch is in the repository: the composer opens with that branch under _Open existing branch_. Start: a workspace on that branch, the pull request on its row, and **Push** after a commit updates the pull request.                                                                                                                                                                                                |
|     | Make a local branch with a pull request's branch name, behind it. **Start workspace**: the composer says how many commits behind, and `git rev-parse` on the branch is unchanged afterwards.                                                                                                                                                                                                                                                                    |
|     | **Start workspace** on a pull request from a fork: the branch is `pr/<n>`, the composer says Yardsort will not push it. In the workspace: its row shows the pull request, the changes panel offers no **Push** after a commit and says why, `git pull` brings in a new commit from the author, and `git push` in a shell tab is refused by git.                                                                                                                 |
|     | A row whose pull request has a workspace names it; pressing the name, or **Go to workspace**, selects it.                                                                                                                                                                                                                                                                                                                                                       |
|     | With `gh` logged out (`gh auth logout`), **Refresh**: one line saying to run `gh auth login`, no alert, and workspace rows simply lose their badges. Log in again and **Refresh**: everything is back.                                                                                                                                                                                                                                                          |
|     | A project on GitLab and one with no remote: each is named on a line above the rows, and the other projects' rows are still there.                                                                                                                                                                                                                                                                                                                               |
|     | A repository with more than 200 open pull requests: the list holds 200, the line above says _Showing the 200 most recently updated of N open_, the sidebar count ends in **+**, and **See all on GitHub** opens its pull requests page.                                                                                                                                                                                                                         |
|     | **Mod+K**, _Pull requests_: the view opens. In **Settings → Keyboard** it has no key until you give it one; give it one and it opens with that.                                                                                                                                                                                                                                                                                                                 |
|     | **macOS / Windows:** the first, fourth, fifth, ninth and eleventh rows. On Windows, a pull request branch with a `/` in its name becomes a workspace like any other.                                                                                                                                                                                                                                                                                            |

## 24 · Tasks

Needs the GitHub CLI logged in, and a project on GitHub with a few issues: one somebody else
opened that nobody answered, one you answered last, one a bot commented on last, one with a
label and an assignee, one closed as completed and one as not planned.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                                                |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | **Tasks**, under Pull requests in the sidebar, shows how many need an answer — the unanswered one and the one a bot spoke on last, not the one you answered. Press it: the list replaces the center panel, most recently updated first, each row with project, key, state, author, age, comments, assignee and labels in their colours. Press it again, or a workspace: the panel goes back.                         |
|     | Answer the unanswered issue on GitHub, come back to the window with the view open: within a minute its **Needs an answer** is gone and the sidebar's number is one less.                                                                                                                                                                                                                                             |
|     | The tabs: **Open** to start with. **Closed**: after a moment the closed ones appear, each saying **Closed** or **Not planned**, with a line saying they are the 50 most recently updated. **All**: both.                                                                                                                                                                                                             |
|     | Pick a label, then an assignee, then tick **Needs an answer**: the list narrows by all of them. Quit and reopen: the tab and the filters are as they were; the search box is empty. **Clear filters** puts back everything but the tab.                                                                                                                                                                              |
|     | Press a row: details open beside the list. The description appears within a second, as Markdown; the conversation follows, oldest first, with **maintainer** and **bot** marked. **↑**/**↓** still move in the list; **Esc** closes the details and the row keeps the focus. **⇤** hides the list, **⇥** brings it back.                                                                                             |
|     | A description with a screenshot and a link: the image is not loaded — it is a link — and the link opens in the browser, with the Yardsort window staying where it is.                                                                                                                                                                                                                                                |
|     | **Open on GitHub** opens the issue in the browser; **Copy link** puts its address on the clipboard.                                                                                                                                                                                                                                                                                                                  |
|     | With `gh` logged out (`gh auth logout`), **Refresh**: one line saying to run `gh auth login`, no alert, and the rows that were there stay. Log in again and **Refresh**.                                                                                                                                                                                                                                             |
|     | A project on GitLab, one with no remote, and one whose repository has issues switched off: each is named on a line above the rows, and the other projects' rows are still there.                                                                                                                                                                                                                                     |
|     | **Mod+K**, _Tasks_: the view opens. In **Settings → Keyboard** it has no key until you give it one.                                                                                                                                                                                                                                                                                                                  |
|     | In a workspace's shell tab: `ys task list` lists that project's open tasks; `ys task list --needs-answer --json` is JSON with only the waiting ones; `ys task show <n>` prints one with its conversation. Outside any workspace, `ys task list` asks for `--project`.                                                                                                                                                |
|     | On an open task, **Delegate**: the composer opens for that project with the message filled in — the issue between two marked lines, then what to do if it asks for more than the work — and a line above the box naming the task. Nothing has started. Change a word and **Start**: the workspace is named `<number>-…`, its agent's first message is what you saw, and its row in the sidebar shows the task's key. |
|     | Press that key: the Tasks view opens on that task, which now names the workspace under its row and offers **Go to workspace** and **Delegate again**. **Delegate again**, start: a second workspace, `…-2`, and **Go to workspace ▾** lists both. Nothing changed on GitHub: no assignee, no comment, no label.                                                                                                      |
|     | Open a pull request from that workspace: its row shows the pull request's number, and its preview still says _Started from_ the task. A closed task has no **Delegate**. Delete the workspace: the issue is untouched and the task no longer names it.                                                                                                                                                               |
|     | In a shell tab: `ys task start <n> --no-agent` makes a workspace named after the task and says `task #<n>`; `ys workspace list --json` has it under `tasks`. `ys task start` on a closed task, and on a link to another repository's issue, each refuse and make nothing.                                                                                                                                            |
|     | **macOS / Windows:** the first, fifth, sixth and eleventh rows. On Windows, `ys task list` in PowerShell and in `cmd`.                                                                                                                                                                                                                                                                                               |

## Results

Nothing recorded yet for macOS or Windows. Add a section per pass:

```
### macOS 15.6, M2 Pro, WKWebView, scale 2, 0.5.0 — 2026-09-21
Sections 1–8 pass except …
Benchmarks: see 07-terminal-benchmarks.
Found: …
```
