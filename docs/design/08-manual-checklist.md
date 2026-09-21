# 08 — Manual checklist

CI builds on Linux, macOS and Windows and runs the PTY host's integration tests there — real
processes in real PTYs, ConPTY included. What it cannot see is a window: rendering, keyboard
routing, a harness's first-run dialogs, or whether an agent is still working after you close the
app. This is the pass that covers that, and the record of having done it.

The roadmap owes it on **macOS and Windows** (M7; M1 wants the benchmark rows too, M4 wants the
harnesses exercised, M9 the daemon). Linux is recorded in
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

## 6 · Changes, files and the editor

| ✓   | Check                                                                               |
| --- | ----------------------------------------------------------------------------------- |
|     | While an agent edits, the changes list updates by itself, and an open diff with it. |
|     | The diff and the read-only file viewer render correctly, including a large file.    |
|     | The file tree opens folders; ignored files appear only when asked for.              |
|     | **Open in editor** opens the right file in your editor.                             |
|     | A repository with a big `node_modules` stays responsive.                            |

## 7 · Deleting nothing by accident

| ✓   | Check                                                                                                                                   |
| --- | --------------------------------------------------------------------------------------------------------------------------------------- |
|     | Delete a workspace with uncommitted changes: it names what would be lost and takes a second yes.                                        |
|     | Answer no: the worktree and the changes are still there.                                                                                |
|     | Delete one that is clean: the folder goes, **the branch stays**, and it can be opened again from the composer's _Open existing branch_. |
|     | Archive and restore: the conversations come back with the workspace.                                                                    |

## 8 · The numbers

```sh
scripts/bench/run.sh
```

Add the rows to [07-terminal-benchmarks](07-terminal-benchmarks.md) in the shape the Linux section
uses — both workloads, both renderers — with the machine, OS, webview version and display scaling.
This is the half of M7's box that is not a judgement call.

## Results

Nothing recorded yet for macOS or Windows. Add a section per pass:

```
### macOS 15.6, M2 Pro, WKWebView, scale 2, 0.5.0 — 2026-09-21
Sections 1–7 pass except …
Benchmarks: see 07-terminal-benchmarks.
Found: …
```
