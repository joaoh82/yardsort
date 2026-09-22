# Changelog

Notable changes in each release. The [releases page](https://github.com/joaoh82/yardsort/releases)
has the downloads and the full commit lists.

## 0.8.1

- **A first message starting with `-` no longer stops the agent.** Pasting a bullet out of a list
  — `- Commit / push / open PR from the UI` — put the agent's own argument parser in charge of it,
  and Claude Code, Codex and Grok each refused to start with `unknown option` or
  `unexpected argument`. The prompt is now passed after `--`, which is how a parser is told the
  options have ended. `ys workspace new` accepted no such message either, and does now.

## 0.8.0

- **`ys attach`.** Put a running agent on your terminal from the command line: the screen is
  repainted where it got to, and what you type reaches it. `Ctrl-]` detaches and leaves it
  running. The window can have the same session open at the same time. See
  [The `ys` command line](docs/guide/cli.md).
- **`ys logs`.** Print what a session has on its screen as plain text, finished ones included —
  how an agent ended up, without opening the window. Screens are held by the background process
  and never written to disk, so they last as long as it does.

## 0.7.0

- **A command line: `ys`.** Start a workspace and put an agent in it without opening the window —
  `ys workspace new <project> "<prompt>"` — plus `project list`, `workspace list`,
  `session list` and `doctor`. It reads the same database as the app, so each sees the other's
  work, and the agent it starts belongs to the background process, so it keeps going after the
  command returns. A separate download in each release; `--json` on every command, for scripts.
  See [The `ys` command line](docs/guide/cli.md).
- **The terminal responds faster.** Output was held for up to 8 ms before being sent to the
  window, including a single keystroke echoing back into a terminal that was otherwise idle —
  where there was nothing to batch it with and the delay bought nothing. It is now held only
  while output is actually streaming, which is what the batching was for. Typing and
  quick-responding prompts feel noticeably more immediate; agents flooding the screen are
  batched exactly as before.
- Workspace and branch names look for the task after introductory context in the first message,
  skipping common request lead-ins and fenced code instead of always taking the opening words.
- Internally, the core is now its own crate with no Tauri in it, which is what lets a
  command-line client exist without carrying a webview around.

## 0.6.0

- **`local` asks what to open.** Clicking a project's own checkout used to open a shell by itself.
  It now offers **Open Terminal** or **Open Composer**, and the composer runs the agent in that
  checkout on the branch you have out — no branch and no worktree are created. It only asks when
  there is nothing running there and nothing to resume. See
  [Projects](docs/guide/projects.md#the-local-workspace).
- **Side-by-side diffs.** A diff opens inline as before; **Side by side** in the viewer's header
  puts the old and new versions in two panes, which is easier to read on a wide change. Your
  choice is remembered. See [Changes & files](docs/guide/changes-and-files.md).
- Hovering a workspace no longer shows its path in a tooltip; the bottom bar already says it for
  the workspace you are in. Archived and missing workspaces keep theirs — they cannot be opened,
  so it is the only place their path is written.

## 0.5.0

- **Agents keep working when you close Yardsort.** Terminals moved out of the app into a small
  background process (`yardsortd`) that owns them, so closing the window — or Yardsort crashing —
  no longer stops anything. Open it again and every terminal is repainted where it got to, and
  conversations that never stopped are no longer listed as _interrupted_.
- Closing Yardsort with agents still running now asks, naming them: leave them running, stop
  them, or cancel. Leaving them running is the default, and shells are never counted as work.
- The status bar shows the background process, and `YARDSORT_NO_DAEMON=1` turns it off.
- A contact address for questions and support, `hello@yardsort.sh`, on the website and in
  the docs.

## 0.4.0

- **Assist (optional).** With a TypeSafe API key of your own, Yardsort can check a workspace's
  changed files against what the agent was asked to do — badging files that look off-task, or that
  add a secret, weaken a test or switch a check off — and suggest a harness and an effort for the
  message you are typing. Off until you enter a key and tick a feature; the key is kept in your
  system credential store, never in `settings.toml`. Agent output is still never read. See
  [Assist](docs/guide/assist.md).
- Settings → Harnesses gains **Good at**: your own description of what a harness suits, used only
  by Assist's composer suggestion.
- Assist's thresholds — how sure Jev must be before a badge or a suggestion appears — are settings,
  with **Restore defaults**. Changing one re-reads answers already given instead of asking again.
- **Worktrees removed with git are noticed.** Delete a workspace's worktree _and_ its branch
  yourself, and Yardsort marks the workspace **gone** and asks whether to delete it from the app
  too — a "keep" is remembered. See [Workspaces](docs/guide/workspaces.md#when-the-branch-goes-too).
- **Linux AppImage: nothing from the replaced version follows an update.** After updating in place,
  terminals still carried the previous AppImage's `LD_LIBRARY_PATH` and friends — the new app is
  started by the old one, so it inherits them — and git kept loading libraries out of the version
  that had just been replaced. Paths into any AppImage's mount are now dropped, not only our own.

## 0.3.2

- Install with Homebrew on macOS: `brew install --cask joaoh82/yardsort/yardsort`.
- Linux: the quick start shows how to give the AppImage a launcher entry and icon.
- Submitted to winget (`joaoh82.Yardsort`), pending Microsoft's review.
- **Linux AppImage: native Wayland.** The AppImage used to run under XWayland, where typing lagged
  and dictation (Omarchy's voice input, anything built on `wtype`) dropped or garbled characters.
  It now opens a Wayland window, and falls back to X11 by itself if that ever fails.
  `YARDSORT_GDK_BACKEND` picks one explicitly.
- **Linux AppImage: terminals get your own environment.** Shells and agents no longer inherit the
  AppImage's private variables (`LD_LIBRARY_PATH`, `PYTHONHOME`, `GDK_BACKEND`, …), which broke
  `python3` and made git print library warnings.

## 0.3.1

- A changelog (this file).
- First release delivered through the in-app updater: a 0.3.0 install offers it by itself.

## 0.3.0

- **In-app updates.** Yardsort looks for new versions shortly after starting and once a day, shows
  an **Update to x.y.z** button, and installs on request — signed and verified. The macOS app,
  Windows installers and the Linux AppImage update themselves; `.deb`, `.rpm` and AUR installs are
  told a new version exists. See [Updates](docs/guide/updates.md).
- **First-run check.** The welcome screen says whether git and at least one agent were found and,
  if not, how to install them — with **Check again**, no restart needed. The status bar's
  `env: …` indicator re-reads your environment from anywhere.
- Settings → General gains **Check for updates automatically** and **Check now**.

## 0.2.0

- **Renamed from Switchyard to Yardsort.** Projects, workspaces, session history and settings are
  carried over automatically on first launch. New branches are prefixed `ys/`; `sy/` branches
  keep working, as do the `SWITCHYARD_*` environment variables.
- An AUR package, `yardsort-bin`, is prepared and will be published by the release workflow.

## 0.1.0

First public release, as _Switchyard_.

- Projects and workspaces: every task gets its own git worktree and branch.
- Real terminals running the agent of your choice — Claude Code, Codex, Grok, OpenCode, or any
  terminal agent you configure.
- Live list of changed files with diffs, a file tree, and one click into your editor.
- Resume and fork agent conversations; status dots and desktop notifications.
- Rename, archive, restore and delete workspaces, never losing uncommitted work silently.
- Linux, macOS (signed and notarized) and Windows builds.
