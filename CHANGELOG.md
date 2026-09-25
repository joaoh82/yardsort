# Changelog

Notable changes in each release. The [releases page](https://github.com/joaoh82/yardsort/releases)
has the downloads and the full commit lists.

## Unreleased

- **Every built-in agent can now report what it does.** Three more switches in Settings →
  General, all off by default: **Capture what OMP reports** and **Capture what pi reports** give
  those agents a small extension on the command line of each launch — never in your extension
  folders — that reports each turn with its tokens and cost, each tool with its file or duration,
  and, on OMP, each permission you answer. **Capture what Cursor reports** gives the Cursor agent
  a plugin directory for that launch, whose hooks report each tool and file edit beside your own
  hooks. Never a prompt, a command, a file or a reply, as before. The Cursor
  adapter is built from Cursor's documentation and awaits a recording; pi was recorded as far as
  its first prompt. See [What OMP and pi report](docs/guide/activity.md#what-omp-and-pi-report),
  [What Cursor reports](docs/guide/activity.md#what-cursor-reports) and the design notes
  ([15](docs/design/15-agent-events-stage-2-pi-omp.md),
  [16](docs/design/16-agent-events-stage-2-cursor.md)).
- **Grok's own session log on the timeline.** Switch on **Read what Grok records** in Settings →
  General and every Grok that Yardsort starts has its turns read from the log Grok keeps for
  each session: which tool ran and how long it took, tools that failed, the permissions you were
  actually asked for and how long you took to answer, each turn and its tokens. That log never
  holds a command, a path or a message. Nothing is given to Grok and nothing in `~/.grok` is
  written; Yardsort chose the session id, so it knows where to look. Off by default. See
  [What Grok records](docs/guide/activity.md#what-grok-records) and the
  [design note](docs/design/14-agent-events-stage-2-grok.md).
- **OpenCode can report what it does.** Switch on **Capture what OpenCode reports** in Settings
  → General and every OpenCode that Yardsort starts puts its work on the activity timeline as it
  happens: each tool with its file or exit code, tools that failed, files it edited, permission
  prompts, and the tokens each reply cost — never a message, a command, an output or a file's
  contents. It works through a small plugin given to that launch alone through OpenCode's
  environment; your `opencode.json` is not edited and your own plugins keep running. Off by
  default. See [What OpenCode reports](docs/guide/activity.md#what-opencode-reports) and the
  [design note](docs/design/13-agent-events-stage-2-opencode.md).
- **Codex can report what it does.** Switch on **Capture what Codex reports** in Settings →
  General and every Codex that Yardsort starts puts each turn on the activity timeline: the shell
  commands it ran with their exit codes and durations, the files it added or changed, the tokens
  the turn cost, and how long it took — never a command line, a file's contents or a message. It
  works through Codex's `notify` program, given for that launch alone, plus Codex's own session
  file; nothing in `~/.codex` is edited and a `notify` of your own keeps running. Off by default.
  See [What Codex reports](docs/guide/activity.md#what-codex-reports) and the
  [design note](docs/design/12-agent-events-stage-2-codex.md).
- **Claude Code can report what it does.** Switch on **Capture what Claude Code reports** in
  Settings → General and every Claude Code that Yardsort starts tells the activity timeline about
  each prompt, tool call, permission prompt, turn and session end — as metadata: the tool's name
  and the file's relative path, never the prompt, the command or the output. It rides on a
  per-launch settings file, so your own Claude Code settings and hooks are never touched, and it
  keeps working while the window is closed. The timeline moves as the agent works; `ys activity
list` shows the same rows as `claude/hook`. Off by default. Other agents stay at what Yardsort
  itself sees. See [What Claude Code reports](docs/guide/activity.md#what-claude-code-reports)
  and the [design note](docs/design/11-agent-events-stage-2-claude.md).
- **Activity: a local record of what ran.** Yardsort now notes when each agent, shell or run
  command starts in a workspace — fresh, resumed or forked, from the app or from `ys` — and how
  it ended, without reading anything the agent prints. An agent that finishes while the window
  is closed is no longer listed as _interrupted_ the next time Yardsort starts: the background
  process keeps its exit and the record gets the real exit code. **Show the activity timeline**
  in Settings → General (experimental) adds an **Activity** button to each workspace's footer;
  **Record when agents start and exit** switches the recording off. `ys activity list` and
  `ys activity export` (NDJSON) read it from a terminal, and `ys doctor` reports how much there
  is. Programs started in a workspace are given `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID` and
  `YARDSORT_SESSION_RECORD_ID`. Nothing leaves the machine. See [Activity](docs/guide/activity.md)
  and the [design note](docs/design/10-agent-events-stage-1.md).

## 0.10.0

- **OMP, Cursor and Pi are built-in harnesses.** Pick them in the composer and configure them
  in Settings → Harnesses, with install links on the welcome screen. OMP and Pi can write commit
  messages and pull request descriptions; Cursor drafting stays disabled to protect Resume. OMP and Pi expose thinking levels; Pi supports
  resuming and forking by session id, while OMP and Cursor resume the latest conversation in
  the workspace. Existing custom harnesses with these IDs keep their definitions and session
  identity. OMP and Pi accept opening messages starting with `@` as text. See [Harness settings](docs/guide/settings.md#harnesses).

- **Project setup and dev servers.** Project settings can copy files such as `.env` and run a setup script in new or restored worktrees before starting an agent. Failed preparation keeps the workspace and setup log. Logs stay outside the checkout, imported worktrees skip setup on restore, and blank argument lines are ignored. Configure a run command and use **▶ Run** to start or focus a dev-server terminal in any workspace. See [Project automation](docs/guide/projects.md#project-automation).

- **`ys workspace delete`.** Remove a workspace from the command line: the folder and the record
  go, the branch stays. Uncommitted work is refused unless `--force` is given, and a harness can
  delete the workspace it is running in — the command does not stop that process. On Windows it
  steps out of the folder first, and deletes nothing if another program still has that folder as
  its current directory. See [The `ys` command line](docs/guide/cli.md).

- **Worktrees made elsewhere no longer appear by themselves.** Yardsort used to turn every
  worktree of a repository into a workspace the moment it looked; a project with worktrees of
  its own — from a script, another tool, or your own `git worktree add` — opened full of
  workspaces nobody asked for. Now only worktrees under Yardsort's own folder are picked up (so a
  removed and re-added project still gets its workspaces back), and the rest wait for
  **Import worktrees…** in the project menu or the composer, which lists them with branch and
  path before anything is recorded. Workspaces adopted by earlier versions are still there: use
  the new **Forget…** in the workspace menu to take one out of Yardsort without touching its
  folder or branch. Its saved conversations are kept unless you tick the box to delete them. See
  [Worktrees made elsewhere](docs/guide/workspaces.md#worktrees-made-elsewhere).

- **Removing a project keeps its workspaces and conversations for when it comes back.** The
  guide always said a removed project's workspaces return when the folder is added again; in
  fact only the worktrees did, as empty workspaces, and the saved conversations were gone. Now
  the project is hidden rather than deleted, so adding the same folder brings back every
  workspace — imported worktrees included — with its history. The remove dialog has a box to
  delete Yardsort's record instead. Nothing on disk is touched either way. See
  [The project menu](docs/guide/projects.md#the-project-menu).

- **Links open in your browser again.** Opening a URL from the app — the release notes on an
  update, the agent install links on the welcome screen, and `Ctrl`/`⌘`-clicking a URL in a
  terminal — was refused with nothing shown but a line in a console nobody sees. The permission
  granted the command but carried no URL scope, which denies everything. Found by pressing a
  pull request badge that had just been made clickable.

- **Have a model write the commit message or the pull request.** A **✦** beside the commit box
  and in the pull request dialog fills them in from the diff, and from what the workspace was
  asked to do. What comes back goes in the box for you to read and edit — it is never committed
  or opened for you. It uses the coding agent you already have, in its non-interactive mode, so
  there is no new account and no new key; your own Anthropic API key is the fallback when no
  configured agent can write. Off with one switch, and absent entirely when nothing can write.
  See [Have it written for you](docs/guide/commits-and-pull-requests.md#have-it-written-for-you).

- **Commit, push and open a pull request from the panel.** Under the changed-files list: a
  message box that commits everything the workspace has changed, a **Push** button once the
  branch is ahead of its remote, and **Open pull request** — which pushes first if it needs to,
  and fills the title and description in from the commits the remote has not got yet: one
  commit's own message body becomes the description, several become a list. Committing is
  confirmed first, naming the files and the branch it lands on. With the
  [GitHub CLI](https://cli.github.com) it opens the pull request from here; without it the branch
  is pushed and your browser opens the forge's own form, which works for GitHub, GitLab,
  Bitbucket and the Gitea family alike. Yardsort holds no forge credentials of its own. See
  [Commits & pull requests](docs/guide/commits-and-pull-requests.md).

- **A workspace's pull request and its checks, on its row.** The number appears in the sidebar
  once there is one, coloured by CI — green when every check has finished and passed, amber while
  one is running, red if one failed — and says **merged** or **closed** when it is over. One
  request per project, not one per workspace. Press it to open the pull request in your browser.
  Needs `gh`; without it nothing shows and nothing complains. See [On the workspace row](docs/guide/commits-and-pull-requests.md#on-the-workspace-row).

## 0.9.2

- **A waiting update is announced beside Settings.** The **update** pill at the foot of the
  projects panel appears when the daily check finds a newer version, and opens the update dialog;
  it replaces the button that used to sit in the status bar. Coming back to the window also looks
  again when the last check is a day old, so a laptop that slept through it still finds out. See
  [Updates](docs/guide/updates.md).

- **Every agent has a mark of its own.** Claude Code, Codex, Grok and OpenCode each get a small
  icon, shown wherever an agent is named: the tab strip and its agent buttons, the composer's
  picker, the session history and Settings → Harnesses. A harness you add yourself gets its
  initial in the same frame. See [Harnesses](docs/guide/settings.md#harnesses).

- **Workspace rows say what their agents are up to.** A number at the end of the row counts the
  agents waiting for you — filled in solid if they finished while you were elsewhere — a **✓**
  means they have all ended, and a **✗** that one exited with an error. Nothing to click. See
  [The badge on a workspace row](docs/guide/terminals-and-sessions.md#the-badge-on-a-workspace-row).

## 0.9.1

- **Drop a file on the composer to point the first message at it.** The same gesture that pastes
  a path into a running terminal now inserts the path into the message you start a workspace
  with, at the cursor. Several files land as several paths. See
  [Starting one: the composer](docs/guide/workspaces.md#starting-one-the-composer).

## 0.9.0

- **`Shift+Enter` adds a line in an agent's terminal.** It used to send the message, because the
  terminal encoded it as a plain `Enter`. It is now sent the way kitty, Ghostty and foot send it,
  which Claude Code, Codex, Grok and OpenCode all read as a new line. Shells still get `Enter`.
- **Drop a file on a terminal to give the agent its path.** The terminal is outlined while you
  hover, and the path — several, if you drop several — is pasted at the cursor, quoted for the
  shell. See [Typing to an agent](docs/guide/terminals-and-sessions.md#typing-to-an-agent).

## 0.8.2

- **Windows: an agent installed while Yardsort is running is now found by "Check again".** It
  never was — a process's environment on Windows is fixed when it starts, and the installer
  writes the new `PATH` where only new processes look, so re-reading our own environment told us
  nothing and a restart was the only cure. `PATH` is now read back from the registry and merged
  with the one we were launched with, so nothing a terminal had added is lost.

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
