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

| ✓   | Check                                                                                                                             |
| --- | --------------------------------------------------------------------------------------------------------------------------------- |
|     | The welcome screen lists git and at least one agent as found. **Check again** re-reads without a restart.                         |
|     | Status bar, bottom right: `env: login shell` (macOS) or `env: process` (Windows), a plausible PATH count, and **`daemon <pid>`**. |
|     | `daemon <pid>` is present, not `no daemon`. If it says `no daemon`, hover it — that tooltip is the finding.                       |
|     | **Windows:** with Yardsort open, install an agent it did not find, then press **Check again**. It is found, without a restart.    |

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
part of the app bundle. Put it on your `PATH` first. On macOS it is **unsigned**, so a copy
downloaded with a browser is quarantined — that is itself the first row.

Run it against the same throwaway profile the app is using:

```sh
ys --data-dir /tmp/ys doctor        # or C:\ys on Windows
```

### It runs, and it is looking at the right place

| ✓   | Check                                                                                                                                            |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
|     | macOS, downloaded with a browser: it is refused on first run. `xattr -d com.apple.quarantine ys` fixes it. Downloaded with `curl`: it just runs. |
|     | Windows: `ys --help` **prints something**. Silence means the console is missing, which is the bug a separate binary exists to avoid.             |
|     | `ys doctor` with no `--data-dir`, while the app is closed: the data directory it names is the one the app really uses.                           |
|     | That same `doctor` says `database … (found)` and a plausible project count — not `NOT FOUND`, and not zero when you have projects.               |
|     | The app prints no `warning: the data directory is …` line at startup. If it does, `ys` and the app disagree and that warning is the finding.     |

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

Needs a real Cursor agent CLI, logged in (`cursor-agent login`). **First**, run
`scripts/record-cursor.sh` and commit the fixtures it writes: the adapter was built from the
documentation, and this is the recording it owes. Then switch on **Record when agents start
and exit**, **Show the activity timeline** and **Capture what Cursor reports** in Settings →
General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                  |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | `scripts/record-cursor.sh` reports that the launch environment reached the hooks (`YARDSORT_RUN_ID` and the rest). If it did not, every Cursor row below will be unlinked (`inbox_unlinked` in `ys doctor`), and the adapter needs a launch-side session id before the rest of this section means anything.                                                                            |
|     | Start Cursor from the composer with a message asking it to create a file and run a command. The `cursor started` row says _reporting through hook_. As it works: `agent session started`, `Write done · hello.txt`, `file changed · hello.txt`, `shell done · N ms`, then `agent session ended`, all `cursor/hook`. No `prompt submitted` row: that hook can block, so it is not used. |
|     | `ys activity list --json`: no command, no message, no file contents, no edit text anywhere in the payloads.                                                                                                                                                                                                                                                                            |
|     | Note whether `agent finished its turn` appears: the documentation says `stop` fires for plugin hooks; the version this was built against was reported not to.                                                                                                                                                                                                                          |
|     | Ask it for something that spawns a subagent (a broad exploration): the subagent runs — nothing of Yardsort's answers `subagentStart` — and `subagent stopped` follows.                                                                                                                                                                                                                 |
|     | `~/.cursor/hooks.json` and the project's `.cursor/` are unchanged; your own hooks still run.                                                                                                                                                                                                                                                                                           |
|     | Switch **Capture** off; start it again: no `cursor/` rows.                                                                                                                                                                                                                                                                                                                             |
|     | **macOS / Windows:** all of the above; on Windows the hook command runs through PowerShell, so a data directory with spaces in its path is the case to try.                                                                                                                                                                                                                            |

## 18 · Reported writes beside the diff

Needs one reporting agent; Claude Code is the most direct (`Write` and `Edit` are reported as
tool calls with the file). Switch on **Record when agents start and exit**, **Show the activity
timeline** and **Capture what Claude Code reports** in Settings → General.

| ✓   | Check                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     | Start Claude Code from the composer and ask it to _use its Write tool_ to create `hello.txt` — asked plainly, it tends to run `echo > hello.txt`, and a shell command reports no file: `Bash done`, no badge, by design (found on the first pass). As the `Write done · hello.txt` row lands, the Changes list's `hello.txt` gains a **claude** badge without a refresh; hovering it says _claude reported writing this file once, last at …_ and ends with the line about git. |
|     | A line above the list reads _Every changed file was reported written by an agent._ Now edit a second file yourself: the line becomes _Agents reported writing 1 of 2 changed files_ and names you, a script, a command the agent ran, or a non-reporting run as the alternatives. Your file has no badge.                                                                                                                                                                       |
|     | Open `hello.txt` and **Expand**: the header says _reported by claude · 1 write · HH:MM_. Open your own file and expand: _no agent reported writing this_.                                                                                                                                                                                                                                                                                                                       |
|     | In the Activity panel, the `Write done · hello.txt` row has a **diff** button; pressing it opens that diff below the list. A `Bash done` row has none.                                                                                                                                                                                                                                                                                                                          |
|     | Switch **Capture** off and start a second Claude Code; ask it to edit `hello.txt`. The badge stays (the first run reported) and the line now counts _one of the 1 agent run here that was not reporting_.                                                                                                                                                                                                                                                                       |
|     | Clear the activity: every badge and the line go; the list reads as it did before this section.                                                                                                                                                                                                                                                                                                                                                                                  |
|     | **macOS / Windows:** the first three rows; the badge's path is what the Changes list shows (forward slashes on Windows too).                                                                                                                                                                                                                                                                                                                                                    |

## Results

Nothing recorded yet for macOS or Windows. Add a section per pass:

```
### macOS 15.6, M2 Pro, WKWebView, scale 2, 0.5.0 — 2026-09-21
Sections 1–8 pass except …
Benchmarks: see 07-terminal-benchmarks.
Found: …
```
