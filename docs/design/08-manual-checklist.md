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

## Results

Nothing recorded yet for macOS or Windows. Add a section per pass:

```
### macOS 15.6, M2 Pro, WKWebView, scale 2, 0.5.0 — 2026-09-21
Sections 1–8 pass except …
Benchmarks: see 07-terminal-benchmarks.
Found: …
```
