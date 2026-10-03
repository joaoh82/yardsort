# Changelog

Notable changes in each release. The [releases page](https://github.com/joaoh82/yardsort/releases)
has the downloads and the full commit lists.

## 0.18.0

- **Install ys no longer, now and then, says the copy it just made is "not Yardsort's".** Right
  after the copy, the file could still be held open for a moment, and running it to read its
  version failed. The check now waits that moment out.

- **The standalone `ys` for macOS is signed and notarized.** The `ys-…-macos-universal.tar.gz` on
  the releases page used to hold an unsigned binary, which macOS refused with _"ys" Not Opened_
  when it was downloaded with a browser. It is now the same signed, notarized file the app
  carries. See [the `ys` guide](docs/guide/cli.md#without-the-app).

- **`brew install --cask joaoh82/yardsort/yardsort` works again.** Current Homebrew refused the
  cask over a minimum-macOS line it no longer accepts (#90). The cask now says `depends_on :macos`
  with no minimum version — the one form today's `brew style` takes, which also insists on the
  stanza for every app cask and on its exact placement — and releases test the cask against an
  up-to-date Homebrew before publishing it, so a rule change like this is caught there rather
  than by users.

- **Usage is easier to find in the docs and website.** The README and homepage now explain both
  tabs, supported token sources and API cost estimates. The quick start introduces Usage, and
  the guide adds a first-look walkthrough, refresh guidance and the distinction from activity
  capture. New screenshots show Token usage and Machine resources with demo data.
- **Pull requests: every project's, in one place.** **Pull requests**, at the top of the sidebar
  or in the command palette, lists the pull requests of every project on GitHub — every open one
  up to the 200 most recently updated, and the newest fifty of any state — with who opened each,
  its checks as passed out of total, its age, its size and whether it conflicts. Filter by state,
  project, author and review status, including _Awaiting review from you_ and _Reviewed by you_;
  the filters are remembered. Open one for its **Summary** — the description as Markdown, every
  check with a link to its run, the reviewers, and the conversation of comments and reviews —
  and to act on it: **Start workspace**
  fetches its branch and opens the composer on it, **Merge** (squash, merge commit or rebase),
  **Close** and **Reopen** each ask first and say whose pull request it is. No branch is ever
  deleted, and a branch you already have is never moved. A pull request from a fork becomes a
  `pr/<number>` branch that follows it; Yardsort does not push to forks, and the changes panel
  says so instead of offering **Push**. Other people's words are shown with care: no image in
  a description is loaded, links open in your browser, and HTML is not rendered. It needs `gh`,
  and says plainly when `gh` is missing, logged out, or the project is on another forge. See
  [Pull requests](docs/guide/pull-requests.md).
- A workspace whose open pull request has more than fifty newer ones in front of it now shows it
  on its row: every open pull request is read when Yardsort starts, not only the newest fifty.

## 0.17.0

- **Usage: the tokens your agents spent, and what they use of the machine.** **Usage**, at the
  foot of the sidebar or in the command palette, has two tabs. **Token usage** reads the logs
  Claude Code, Codex and Grok keep on this machine and shows the last 7, 30 or 90 days by day,
  agent, model and workspace — with cache reads, cache writes, output, and what the cache saved —
  priced at each vendor's API rates as an estimate (a subscription is not billed that way), and
  Codex's plan limits as it last heard them. **Machine resources** samples every two seconds while
  it is open: Yardsort's CPU and memory over the last five minutes, the machine's memory and load,
  and a breakdown by project, workspace and terminal, each terminal counted with every process it
  started. Nothing is sent anywhere. A switch in its header takes it off the sidebar. See
  [Usage](docs/guide/usage.md).

## 0.16.0

- **Every pull request a workspace opened.** A workspace that opened more than one — a second PR
  on its branch after the first merged, or one from another branch its agent created — shows the
  rest beside its badge (**#42 +2**), lists them when you hover the row, and lets you switch the
  toolbar between them. Which PRs belong to a workspace comes from its worktree's own git history,
  so another workspace's PR on a reused branch name is not counted.

- **Ask the agent to resolve merge conflicts.** When GitHub reports that an open PR conflicts
  with its base, the badge turns red with a **⚠**, and the toolbar menu offers **Ask its agent to
  resolve conflicts…**. It asks the agent that opened the PR — typed into it if it is running,
  resumed with the request if it has ended — to merge the base in, resolve the conflicts, run the
  checks and push, never rebasing or force-pushing. A busy agent is never interrupted.

- **Copy and paste in the terminal work the way more desktops expect.** Right-click for Copy,
  Paste and Select all. Plain `Ctrl+C` copies while text is selected and interrupts otherwise,
  so Omarchy's `Super+C` copies. `Ctrl+Insert` and `Shift+Insert` copy and paste too.

- **Always start an agent in auto mode.** Settings → Harnesses has a switch per agent that starts
  every new, resumed and forked session in that agent's own auto mode — `--permission-mode auto`
  for Claude Code and Grok, `--approve-for-me` for Codex, `--auto` for OpenCode,
  `--approval-mode=write` for OMP and `--auto-review` for Cursor. Off until you turn it on; custom
  harnesses can give their own flag.

- **A nudge to compact a full context.** When a Claude Code or Codex conversation has used 80 %
  of its context window, a bar above its terminal says so and offers **Compact**, which types the
  agent's `/compact` for you; **Not now** hides it until the next step up. It reads the agents' own
  usage records, so it needs their activity capture switched on in Settings → General. The
  Activity timeline shows the same numbers on each turn.

- **Blog and tutorials on the website.** Read the story behind Yardsort, a practical parallel-agent
  tutorial, a worktree explainer, and dated comparisons with Superset and Conductor.

- **Answers and navigation on the website.** The homepage and About Yardsort guide explain
  supported agents, costs, worktrees, background sessions and data handling. Documentation pages
  now have breadcrumb navigation; product and breadcrumb structured data describe the same content.

## 0.15.0

- **Keyboard navigation and configurable shortcuts.** Search commands and workspaces with
  Mod+K, focus the three panels, cycle workspaces and terminal tabs, and navigate trees with
  arrow keys. Settings → Keyboard is the searchable cheat sheet and binding editor, with
  conflict checks, clear/reset controls and saved bindings. Dialogs keep keyboard focus inside.
  Modified arrows retain text-editing behavior in text fields, and disabled commands leave keys
  unconsumed. Saved custom bindings take precedence over new defaults; recovery notices explain
  conflicts or invalid saved data without resetting unrelated shortcuts.
- **An optional welcome tour.** First launch asks whether to take a short guided tour of the
  main panels. Skip or finish it once; replay it any time from Help / Tour in the bottom bar.
- **Clone projects from GitHub.** Add a project from an HTTPS or SSH repository URL, or
  `owner/repository`, choosing its local name and location. Common GitHub web links are accepted,
  the folder name is suggested from the repository, and cloning can continue in the background
  while you use your terminals.
- **Show files in your file explorer.** Right-click entries in Files or Changes to reveal
  their location; deleted files open the nearest existing folder.
- **Clearer website search and link previews.** Every guide now has its own description and
  share title, with a branded preview image, consistent canonical URLs and a sitemap.

## 0.14.0

- **Images and editing in the file panel.** Selecting an image opens a preview; SVG files can
  switch between preview and source. Text and code files can be edited and saved inside
  Yardsort, with syntax highlighting, undo, retained drafts, and protection against saving over
  a file changed by an agent or another editor. Disk reloads reset undo history; symlinks are
  explained before editing, and read-only files are refused without changing their permissions.
  Image changes show before and after previews, retaining text when a file changes type.

## 0.13.1

- **A Yardsort that cannot start now says why.** Opening your data with an older version than
  the one that last used it — after going back to an old download, say — used to close the app
  a moment after launch, every time, without a word. It now shows a dialog saying so and asking
  you to install the latest version; your data is left as it was. Any other failure while
  starting is shown the same way. On Linux such a failure no longer counts as Wayland not
  working, which kept that version of the AppImage under XWayland from then on. See
  [Troubleshooting](docs/guide/troubleshooting.md#yardsort-cannot-start).
- **A way back after an upgrade.** Before a new version upgrades your database, Yardsort keeps a
  copy of it as it was, `yardsort.db.before-upgrade`, which the version before can still open.
  If you go back to that version, its dialog points at the copy. See
  [Going back to the older version](docs/guide/troubleshooting.md#going-back-to-the-older-version-instead).

## 0.13.0

- **Workflows: named agent work, in the sidebar and from `ys`.** A **Workflows** section above
  Projects lists them; open one to see its steps as a chart, edit its file with every mistake
  marked where it is, follow its runs, and **Run…** it. A new one can be written for you: say
  what it should do and press **Write it**, and the agent you already have, or your Anthropic
  key, writes the file, checked like any other and never saved until you say so. A workflow is a
  short YAML file in your profile's `workflows` folder: the inputs to ask for, and steps that
  start an agent, wait for it to settle, type to it once it is quiet, and notify you. Steps run
  as soon as the ones they need succeed, so they can run side by side. `ys workflow validate`
  checks a file and names the line and column of every mistake, including every
  `{{ variable }}`; `list`, `show` and `copy` manage them. With Yardsort open, `ys workflow run`
  queues a run and the app carries it out in the background, with the agents it starts as tabs;
  `ys workflow runs` follows it and `cancel` stops it, never the agents. Steps can wait for a
  review or comment on the workspace's pull request, use its number and link, and talk to the
  workspace's own agent. The built-in **Request code review** puts it together, from a
  workspace's menu or as `ys workflow run code-review --input reviewer=codex`: a second agent
  reviews the pull request and posts on GitHub, then you are told, and the agent that wrote it
  is told to address the review. See [Workflows](docs/guide/workflows.md).
- Workflow generation has a **Workflow writer** choice in Settings → Assist, including Codex and
  custom harnesses with Write args. Automatic selection skips harnesses that are not installed.
  Select a chart node to highlight it and jump to its YAML source line.
- **Project search.** Click the search icon beside Projects to filter by name. Clear the filter
  with **×**, or press **Escape** to close it; **×** also closes an empty field. The selected
  workspace's project stays visible, including projects added while filtering.
- Workspace rows now preview pull request details on hover or keyboard focus, including branches,
  review status, line counts and individual checks. Open harnesses have a count and a preview of
  their names and activity. The workspace toolbar adds the same PR preview with actions to open,
  copy, refresh, or merge by squash, merge commit or rebase after confirmation.
- Workspace previews open on hover or keyboard focus without popping over clicks or menus.
  They only say no PR was found after the forge has answered successfully.
- PR badges and the toolbar dropdown share status colours: green for open PRs, yellow for
  drafts or pending checks/reviews, red for failures, requested changes or closed PRs, and
  purple for merged PRs. Open PRs without CI no longer fall back to grey.
- Reused branches prefer an open pull request over an older merged or closed one. Slower refresh
  responses no longer replace newer PR data in the sidebar or the core cache.

## 0.12.0

- **Project memory.** Short lessons about a project for its agents — "the tests need
  `TZ=UTC`". Write them in **Memory…** from a project's menu, where they are approved as you write
  them; agents propose them with `ys memory propose`, and those wait for you, with a count on the
  project's row. Tick **Give this project's agents its memory** and every agent you start there
  with a first message gets the approved entries after it, cited, as notes rather than
  instructions — never into what the workspace was asked — and handoffs carry them too. Agents
  can run `ys memory search` any time. Nothing an agent proposes reaches another agent until you
  approve it, and there is no command an agent could run to approve it. With Assist, a new switch
  marks proposals that repeat or contradict an approved entry. See
  [Memory](docs/guide/memory.md).
- **Outcomes: what became of each attempt.** After you archive or delete a workspace, the
  sidebar asks how it went — kept, partly or discarded, one optional click. **Outcomes…** in a
  project's menu lists every attempt, deleted ones included, with its task, agents, outcome and
  evidence, and each agent's history. A merged pull request or a branch merged into its base
  counts as kept until you say otherwise; nothing else is ever counted. The composer shows your
  history with the agent you picked — "too few to say yet" until there are five outcomes. See
  [Outcomes](docs/guide/outcomes.md).
- The composer's hint under a handoff said it runs "in the project's own checkout", which is only
  true of `local`; in a worktree it now says it runs in this workspace.

## 0.11.0

- **`ys` comes with the app, and stays up to date with it.** Every installer now carries the
  command-line client next to Yardsort. The `.deb`, `.rpm` and AUR packages put it in
  `/usr/bin`, and the Homebrew cask links it. Elsewhere, **Install ys** — in Settings → General
  and in the first-run checklist — links it into `/usr/local/bin` on macOS, or copies it to
  `~/.local/bin` (AppImage) or to a folder it adds to your `PATH` (Windows). Both screens say
  which `ys` your terminal finds and whether it is this version. A copy is replaced when Yardsort
  starts and finds it older, and a file there that is not `ys` is replaced only after you
  confirm. On macOS the bundled `ys` is signed and notarized with the app. See
  [Installing `ys`](docs/guide/cli.md#installing).
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
- **Claude Code can report what it does.** Switch on **Capture what Claude Code reports** in
  Settings → General and every Claude Code that Yardsort starts tells the activity timeline about
  each prompt, tool call, permission prompt, turn and session end — as metadata: the tool's name
  and the file's relative path, never the prompt, the command or the output. It rides on a
  per-launch settings file, so your own Claude Code settings and hooks are never touched, and it
  keeps working while the window is closed. The timeline moves as the agent works; `ys activity
list` shows the same rows as `claude/hook`. Off by default. Other agents stay at what Yardsort
  itself sees. See [What Claude Code reports](docs/guide/activity.md#what-claude-code-reports)
  and the [design note](docs/design/11-agent-events-stage-2-claude.md).
- **Codex can report what it does.** Switch on **Capture what Codex reports** in Settings →
  General and every Codex that Yardsort starts puts each turn on the activity timeline: the shell
  commands it ran with their exit codes and durations, the files it added or changed, the tokens
  the turn cost, and how long it took — never a command line, a file's contents or a message. It
  works through Codex's `notify` program, given for that launch alone, plus Codex's own session
  file; nothing in `~/.codex` is edited and a `notify` of your own keeps running. Off by default.
  See [What Codex reports](docs/guide/activity.md#what-codex-reports) and the
  [design note](docs/design/12-agent-events-stage-2-codex.md).
- **OpenCode can report what it does.** Switch on **Capture what OpenCode reports** in Settings
  → General and every OpenCode that Yardsort starts puts its work on the activity timeline as it
  happens: each tool with its file or exit code, tools that failed, files it edited, permission
  prompts, and the tokens each reply cost — never a message, a command, an output or a file's
  contents. It works through a small plugin given to that launch alone through OpenCode's
  environment; your `opencode.json` is not edited and your own plugins keep running. Off by
  default. See [What OpenCode reports](docs/guide/activity.md#what-opencode-reports) and the
  [design note](docs/design/13-agent-events-stage-2-opencode.md).
- **Grok's own session log on the timeline.** Switch on **Read what Grok records** in Settings →
  General and every Grok that Yardsort starts has its turns read from the log Grok keeps for
  each session: which tool ran and how long it took, tools that failed, the permissions you were
  actually asked for and how long you took to answer, each turn and its tokens. That log never
  holds a command, a path or a message. Nothing is given to Grok and nothing in `~/.grok` is
  written; Yardsort chose the session id, so it knows where to look. Off by default. See
  [What Grok records](docs/guide/activity.md#what-grok-records) and the
  [design note](docs/design/14-agent-events-stage-2-grok.md).
- **Every built-in agent can now report what it does.** Three more switches in Settings →
  General, all off by default: **Capture what OMP reports** and **Capture what pi reports** give
  those agents a small extension on the command line of each launch — never in your extension
  folders — that reports each turn with its tokens and cost, each tool with its file or duration,
  and, on OMP, each permission you answer. **Capture what Cursor reports** gives the Cursor agent
  a plugin directory for that launch, whose hooks report each tool and file edit beside your own
  hooks. Never a prompt, a command, a file or a reply, as before. Both are
  recorded against the real agents. See [What OMP and pi report](docs/guide/activity.md#what-omp-and-pi-report),
  [What Cursor reports](docs/guide/activity.md#what-cursor-reports) and the design notes
  ([15](docs/design/15-agent-events-stage-2-pi-omp.md),
  [16](docs/design/16-agent-events-stage-2-cursor.md)).
- **Cursor and pi recorded.** The Cursor and pi adapters now rest on recordings of the real
  agents rather than their documentation. For Cursor that found tool durations being dropped
  (they are fractional), which is fixed, and that this Cursor build does not tell a plugin's
  hooks when a turn ends. For pi it found that a `write` is named by its start and not its end,
  which the Changes list's badges now allow for.
- **The Changes list says which files an agent reported writing.** With any _Capture what …
  reports_ switch on, a changed file the agent said it wrote carries a badge with the agent's
  name — hover it for how many times, when, and through what — and a line above the list counts
  the files with a report and the files without one: you, a script, or a run that was not
  reporting. The expanded diff's header says the same for the open file. The other way round, a
  timeline row that names a listed file has a **Show diff** button. It never claims which lines came
  from whom; git's diff is the whole change and no agent reports a line. Nothing new is
  stored. See [Who wrote it](docs/guide/changes-and-files.md#who-wrote-it) and the design note
  ([17](docs/design/17-agent-events-stage-3-review.md)).
- **A file an agent made with a shell command is marked too, as seen rather than reported.**
  A command names no file, so it cannot be reported; but the file's own modification time says
  when it was last written, and if that falls inside the command's run on the timeline the file
  gets a dashed badge with the agent's name. Its tooltip says exactly that: the time on the file
  was read, not who wrote it. A report, when there is one, is shown instead. Only a tool call
  counts as a window, never the agent merely being open, so with no capture switched on nothing
  changes. See [Who wrote it](docs/guide/changes-and-files.md#who-wrote-it).
- **Assist can be told who wrote each file.** A new switch under Settings → Assist, off by
  default and offered once the review is on, sends with each diff one sentence in the Changes
  list's own words: _reported written by claude_, _last written while claude ran a command_,
  or _not reported written by any agent_. Never a tool, a time or a command. A substantive
  change no agent accounted for then earns an **unaccounted** badge, and every Assist badge's
  tooltip ends with the exact sentence Assist was told. See
  [Telling Assist who wrote each file](docs/guide/assist.md#telling-assist-who-wrote-each-file).
- **Hand off a workspace to another agent.** A new **Hand off…** button in the tab bar writes
  the next agent's first message from what Yardsort recorded: what the workspace was asked,
  where the branch stands and who wrote each changed file, what each agent run did, and what
  is not known — never the last agent's words, which Yardsort does not keep. It opens in the
  composer to read and edit before it is sent; the new conversation's record keeps no first
  message, so the task on record stays yours. See
  [Handing work to another agent](docs/guide/terminals-and-sessions.md#handing-work-to-another-agent)
  and the design note ([18](docs/design/18-agent-events-stage-4-handoff.md)).
- **`ys workspace handoff`, and Assist's order in the handoff.** The handoff packet can be
  printed from the command line with `ys workspace handoff <workspace>` (`--json` for the text
  with counts), the same text the app's button starts from. In the app, with Assist reviewing
  changes, the packet lists the changed files in the order Assist would look at them — on task
  first, unrelated last — with its word on each, from the review the Changes list already
  shows; nothing new is sent, and the packet says it is a judgment, not a fact.

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
