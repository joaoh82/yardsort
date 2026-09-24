# 05 — Roadmap

Ordered by risk first, then by the shortest path to something usable daily. Every milestone must
pass on **Linux, macOS and Windows** before it is done — CI enforces the build, and
[08-manual-checklist](08-manual-checklist.md) covers what CI can't see.

## M0 — Scaffold ✅

- Tauri 2 + React + TS + Vite + bun; lint/format (clippy, rustfmt, eslint, prettier).
- GitHub Actions matrix: ubuntu / macos / windows — build, `cargo test`, frontend tests.
- Typed IPC generation wired up. Empty three-panel shell with resizable panels.

_Exit:_ a signed-or-not installer artifact is produced for all three OSes on every push to `main`
(pull requests run the check matrix only — macOS minutes are billed at 10x on private repos).

## M1 — Terminal spike (highest risk, do first) ✅

- `pty-host` crate on `portable-pty`, with the message-shaped API (spawn / attach / write / resize /
  kill + events) and **no Tauri dependencies** — in-process for now, daemon-ready.
- xterm.js view; raw-byte channel output with batching; input; resize.
- Headless VT state in the host → snapshot on attach. Detach/re-attach across workspace switches.
- Login-shell environment resolution.
- Run a plain shell, then `claude`, in the center panel.
- WebGL renderer with automatic DOM-renderer fallback; evaluate the WebKitGTK/NVIDIA mitigations.

_Exit:_ a full-screen TUI (Claude Code, plus `vim`/`htop` as torture tests) is usable — colours,
resize, mouse, paste, unicode — on all three OSes, including Hyprland/Wayland. Throughput test:
`cat` a large file without freezing the UI. Record a go/no-go on Linux webview rendering, with
numbers (frame times while a harness streams, WebGL vs DOM), before starting M2.

_Result:_ **go** on Linux — see [07-terminal-benchmarks](07-terminal-benchmarks.md). macOS and Windows
are covered by CI (build + PTY integration tests) but still need a hands-on pass and benchmark rows.

## M2 — Projects & sidebar ✅

- SQLite store + migrations. Open project / create project (with `git init` + initial commit).
- Sidebar tree with `local`. Selecting `local` opens a shell tab at the repo root.
- Persist selection, panel sizes, expansion state.

_Exit:_ add, reorder and remove projects; restart the app and everything is where you left it.

_Notes:_ reordering is "Move up / Move down" in the project menu for now — HTML5 drag-and-drop in
Tauri webviews needs the window's native drop handling disabled, which is better decided together
with dropping files onto terminals. Sessions carry a `workspace` label in the PTY host, which is how
tabs find their workspace again after a webview reload (and, later, after attaching to the daemon).

## M3 — Workspaces (the core loop) ✅

- `git` module: root / default-branch detection, worktree add / list / remove.
- Composer UI: harness, model, effort, base branch, message.
- Start → worktree + branch + harness launch with `argv` prompt transport.
- Built-in harness definitions (hard-coded, no settings UI yet). Naming + slugging.
- Failure handling: nothing half-created is left behind.

_Exit:_ from a cold start, create three workspaces in one project on different harnesses and watch
them work in parallel. **This is the first version worth dogfooding.**

_Notes:_ a basic "Delete workspace" came forward from M6, because a loop that can only create
litters: the folder goes, the branch always stays, and uncommitted work needs a second explicit
confirmation. Rename and archive went to M6 and arrived; "delete the branch too" went to M6 and
did not, and is open question 5. Harness sessions are
labelled in the PTY host with their harness and (where we assign one) the harness's own session id,
ready for M6's resume and fork.

Two follow-ups landed right after M3, from dogfooding:

- **Open an existing branch.** The composer's branch picker has two groups: _New branch from…_ and
  _Open existing branch_ (only branches nobody has checked out — git allows a branch in one worktree
  at a time). This is how a branch kept by a delete comes back as a workspace. Yardsort never
  deletes a branch it did not create, not even when undoing a failed start.
- **Adoption.** Whenever projects are listed, worktrees git knows about but Yardsort does not
  become workspaces — ones made by hand, and ones orphaned when their project was removed and added
  again. Stale ("prunable") entries are skipped. This brought forward part of M6's reconciliation.
  _Narrowed 2026-09-23_ (open question 18): only worktrees under Yardsort's own worktree root are
  adopted by themselves; the rest are listed by **Import worktrees…** and recorded only when
  chosen, and **Forget…** takes a workspace out again without touching the disk.

## M4 — Harness settings ✅

- Settings file + override model. Settings → Harnesses form, argv preview, PATH detection,
  Test launch, Restore defaults, custom harnesses.
- `stdin` prompt transport with readiness detection.
- Re-verify every default in [04-harnesses](04-harnesses.md) end-to-end on each OS.

_Exit:_ a harness Yardsort has never heard of can be added and used without touching code.

_Notes:_ the settings file stores only differences from the built-ins, so corrected defaults in a
later version still reach everything the user left alone. Settings also cover the worktree folder
and the branch prefix. Still open from this milestone: exercising every built-in end-to-end with a
real prompt on macOS and Windows (flags are verified against `--help`; Claude Code is verified by
hand on Linux), and the `stdin` transport against a harness that shows a trust or login dialog
first — the paste would land in that dialog. WSL launch prefixes remain an open question.

## M5 — Right panel ✅

- File watcher (debounced, `.gitignore`-aware). Changes tab (uncommitted + committed vs merge-base).
- Files tab. Diff viewer + read-only file viewer. Open in editor.

_Exit:_ while an agent works, the changes list and open diff update live, and stay responsive in a
large repo (test against one with a big `node_modules`).

_Notes:_ the diff viewer is CodeMirror 6's merge view (settling open question 9) — inline at
first, with the two-pane view added later from the same package — loaded
lazily with its grammars. Diffs cross IPC as the two versions of the file, not as a patch, so the
viewer decides how much context to show. The watcher never interprets events: after a burst goes
quiet it says "something changed" and the UI asks git again. On Linux it watches exactly the
directories git does not ignore (inotify is per-directory, and a recursive watch would descend into
`node_modules`), adding folders as they appear; macOS and Windows use one recursive watch and
filter events through the ignore rules. Staging, committing and discarding from the panel are not
part of this milestone.

## M6 — Session lifecycle ✅

- Session records; Resume / Fork / New session; restore-on-launch (lazy: a workspace shows its past
  conversations when it is opened — nothing is started until asked).
- Status dots from PTY activity; desktop notification when a busy agent goes quiet.
- Shell tabs alongside harness tabs. Rename / archive / restore / delete workspace with safety
  prompts.
- Worktree reconciliation: unknown worktrees under our root are adopted, the rest imported on
  request (M3, narrowed after 0.9.2); a workspace whose folder vanished is
  flagged and can be restored from its branch or deleted. If the branch went with the folder there
  is nothing to restore from, and Yardsort offers — once — to forget the workspace.

_Exit:_ quit mid-task, relaunch, and be back in the same conversations within a couple of clicks.

_Result:_ verified by hand on Linux with Claude Code — app killed mid-session, relaunched, workspace
restored with the conversation listed as "interrupted", one click on Resume and the conversation
was back. Fork checked against the same conversation (the copy keeps the context and is saved
under the id we chose, so it is resumable too).

_Notes:_

- Harnesses that choose their own session ids (Codex, OpenCode) can only continue their most recent
  conversation in a folder; older records say so instead of offering a Resume that would open the
  wrong one. Reading their session stores would lift this (open question 12).
- Found while testing: when Yardsort is started from a terminal inside an agent, that agent's
  session markers (`CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION`, …) leaked into the harnesses, and
  Claude Code then refuses to save its transcript. The launch environment now drops them.

## M7 — Ship

Done:

- [x] Licence: **GPL-3.0**. Dependency licences checked for compatibility (all MIT / Apache-2.0 /
      BSD-style / MPL-2.0).
- [x] Open-source groundwork: README with screenshots, quick start, a user guide for every part of
      the app, CONTRIBUTING, Code of Conduct, SECURITY, issue and PR templates, `AGENTS.md`.
- [x] Pre-publication audit of the repository and its whole history for secrets and personal data.
- [x] Release workflow: a `v*` tag builds AppImage + deb + rpm, a universal dmg and NSIS + MSI
      into a GitHub release, published once every platform has built; `just release <version>` cuts one. macOS signing and
      notarization switch on when the Apple secrets are set. See [releasing](../releasing.md).

To do:

- [x] **v0.1.0 released** (2026-09-19, under the project's first name, _Switchyard_): first public release, built by the release workflow on its
      first run. The macOS build is signed with a Developer ID certificate and notarized by Apple.
- [ ] Hands-on pass and benchmark rows on macOS and Windows hardware — the pass is
      [08-manual-checklist](08-manual-checklist.md), the rows are `scripts/bench/run.sh`.
      Reported as looking good on both (2026-09-21), but nothing is recorded yet. The checklist
      has grown a section for the `ys` command line since, which no one has run anywhere but
      Linux.
- [x] Auto-update: signed updates via the Tauri updater. The app checks `latest.json` on the GitHub
      release after start and daily, shows an "Update to x.y.z" button, and installs on request,
      warning about running terminals. AppImage, macOS and Windows copies update themselves;
      package-manager installs are only told. The release workflow signs bundles, publishes
      `latest.json`, and refuses to publish a release whose manifest is incomplete.
      Proven end to end on 2026-09-19: a released 0.3.0 AppImage found 0.3.1, installed it on
      request, replaced its own file and came back as 0.3.1.
- [x] **Renamed to Yardsort** (v0.2.0): the Switchyard name was taken everywhere that matters — see
      open question 14. Existing users' data is carried over on first launch.
- [x] AUR package `yardsort-bin`: rendered, test-built and published by the release workflow.
- [ ] **First AUR publish — blocked upstream**, and its release job is switched off
      (`PUBLISH_AUR`, see [releasing](../releasing.md)) so it does not fail every release. The AUR
      paused new account registration on
      2026-09-19, and a maintainer account is needed to register the deploy key. Everything else
      is in place (the `AUR_SSH_PRIVATE_KEY` secret is set); once an account exists, register the
      public key and re-run the _Publish to the AUR_ job of the latest release.
- [x] Homebrew: cask in the `joaoh82/homebrew-yardsort` tap, install-tested on macOS (Gatekeeper:
      _Notarized Developer ID_) by the release workflow before each push.
- [ ] winget: `joaoh82.Yardsort` submitted (microsoft/winget-pkgs#437567); validation passed and
      the CLA is signed, so it waits on a community moderator. Its release job is switched off
      (`PUBLISH_WINGET`) until the package is merged and `WINGET_TOKEN` is set, after which the
      workflow submits each new version.
- [ ] Flatpak, Scoop, Chocolatey — on request.
- [ ] Windows code signing, if funding appears.
- [x] First-run check: the welcome screen reports whether git and at least one agent were found,
      with install commands and a "Check again" that re-reads the environment without a restart.
- [x] Project website: [yardsort.sh](https://yardsort.sh), deployed on Vercel from `website/` —
      a static Next.js app built from a Claude Design landing page. It renders `docs/` (Markdown
      or MDX) and `CHANGELOG.md` directly, is checked in CI, and redeploys on every push to `main`.

_Exit:_ v0.1.0 public release.

_Found after release (dogfooding the AppImage on Hyprland):_ the AppImage's GTK launch hook forces
`GDK_BACKEND=x11`, so the released app ran under XWayland. Typing lagged, and text typed by
`wtype` (Omarchy's dictation) arrived garbled. The M1 benchmarks never saw this because they ran a
dev build on native Wayland. Yardsort now switches the AppImage back to Wayland, with an automatic
X11 fallback if a Wayland start never shows its window. The same hook and the AppImage runtime
also leaked their variables (`LD_LIBRARY_PATH`, `PYTHONHOME`, …) into every session; the launch
environment now drops them. A second pass (0.4.0) found the same leak surviving an in-app update:
the updater starts the new AppImage from the old process, so the replaced version's mount — still
mounted — came through in the appended path variables. Any AppImage mount now counts as ours to
drop.

## M8 — Assist (optional AI judgments)

Done, off by default, behind the user's own TypeSafe API key:

- [x] `assist` module: a Jev client (retries, pinned model), the key in the OS credential store
      with `TYPESAFE_API_KEY` as the fallback, and a per-feature switch in `settings.toml`.
- [x] Changed files checked against the workspace's task, and for secrets, weakened tests and
      disabled checks; badges on the change list, cached per diff.
- [x] Composer suggestions: a harness (from the user's own "Good at" descriptions) and an effort
      level, offered and never applied by themselves.
- [x] Thresholds are settings (whole percentages, with "Restore defaults"); raw answers are
      cached, so moving one re-reads instead of re-asking.
- [x] Threshold _defaults_ and question wording: **left as shipped** (open question 17, settled
      2026-09-23). They were never measured against a labelled corpus and will not be: they are
      settings, moving one costs nothing because the raw answers are cached, and anyone who finds
      a badge noisy turns it down where they meet it.
- [ ] **Why an agent went quiet.** Decided 2026-09-21 (open question 16): Assist may send the
      last screen from the headless VT when a harness session falls quiet, so Jev can say whether
      it is waiting for permission, asking something, finished or failed — and the notification
      can say that instead of "is waiting". Needs its own design pass first: what is sent, how a
      secret on screen is kept out of it, what the opt-in looks like, and what the notification
      may repeat. Never for a shell.

_Exit:_ with no key, Yardsort behaves exactly as it did before; with one, a workspace that wrote
to `ci.yml` while asked to fix a login bug says so before you read the diff.

## M9 — The daemon ✅

Agents no longer die with the window.

- `crates/pty-ipc`: the wire (length-prefixed frames, JSON control traffic, **raw** output), a
  blocking server loop around one `PtyHost`, and a client that is itself a `TerminalHost`.
- `yardsortd` is the app's own binary re-run with `--yardsort-daemon <socket>` — no second
  artifact to bundle, sign or notarize, and no way for the two to be different builds.
- One daemon per data directory. The app starts one when nobody answers (under a lock file, so
  two copies starting at once produce one daemon) and it stops itself once no client is
  connected and no session is running.
- Quitting with agents running asks: leave them, stop them, or cancel. Closing the window on its
  own leaves them be.
- Session records stop lying: a conversation whose process is still alive stays `running` instead
  of being settled as _interrupted_ at startup.
- Prompt delivery over `stdin` moved into the host, so a first message still lands if the window
  closes while the harness is starting.
- `YARDSORT_NO_DAEMON=1` keeps the old in-process behaviour, for debugging.

_Exit:_ start an agent on a long task, close Yardsort, reopen it — the agent is still working and
its terminal is repainted where it got to. Killing the app with `SIGKILL` is the same.

_Result:_ verified by hand on Linux — the app binary runs as a daemon with no window, survives
`kill -9` of the app, and the reopened app reattaches to the _same_ daemon rather than starting a
second one. The `sessions_outlive_the_client_that_started_them` test in `crates/pty-ipc` covers
the same ground against a real daemon process, on all three OSes in CI.

Re-verified against the **published 0.5.0 AppImage**, because the daemon and the AppImage have a
lifetime problem waiting to happen: the app spawns it with `current_exe()`, which inside an
AppImage is a path in that run's own mount (`/tmp/.mount_<name><random>/usr/bin/yardsort`) — and
`env.rs` already treats such a mount as lasting only as long as the run that made it. It holds
up. Quitting the app did not take the mount away: the AppImage runtime stayed alive alongside the
daemon it had started, and the daemon went on working — a fresh client connected, listed the
surviving session and spawned another. When the daemon later exited on its idle grace, the
runtime exited with it and the mount went too, leaving nothing behind. So an AppImage user has
one lingering runtime process for as long as their agents run, which is the daemon doing its job
rather than a leak. (Why the runtime waits rather than unmounting was not investigated.)

_Notes:_

- `attach` hands the snapshot to the sink _before returning_, under the lock that delivers
  output, so nothing is lost or doubled. Over a socket the response would race those bytes, so
  the **client** picks the stream id and registers its sink before asking.
- Linux abstract-namespace sockets were deliberately passed over for socket _files_: anything
  that can connect can type into an agent's terminal, and a `0600` socket is what keeps that
  to its owner.
- Found while writing the protocol: serde's internal tagging cannot encode a newtype variant
  wrapping a sequence, so `list` silently returned nothing. Every result is a struct variant now.
- Found by Windows CI: `Shutdown { stop_sessions }` killed the sessions and answered without
  waiting for them, so on Windows — where tearing a pseudo-console down is not quick — the
  daemon could exit before the exits were announced, and the app would quit with its records
  still claiming to be running. It now waits for them to go. Because everything shares one
  ordered stream, the exits are delivered, and the records settled, before the reply is.
- Also from Windows CI, in the tests rather than the product: a viewer that records output but
  never answers `ESC [ 6 n` is not a terminal, and ConPTY will not start the program until one
  has answered. `pty-host`'s tests knew this; `pty-ipc`'s now do too. Its reply has to come from
  a thread of its own — a client's output sink runs on the reader thread, and a request made
  from there would be waiting on the thread that has to deliver its answer.
- Found while taking the screenshots for 0.5.0: the daemon tightened its socket's _parent_
  directory to `0700` and gave up if it could not — so a socket in `/tmp` (the last fallback for
  an over-long data directory, and where you would put one starting a daemon by hand) stopped the
  daemon dead. The socket file's own `0600` is the lock; the directory is now best effort.
- CI is green on all three platforms; the hands-on pass M7 owes on macOS and Windows is still
  outstanding.

## M10 — The `ys` command line ✅

Yardsort without the window, for scripting and for asking an agent to start another one.

- `crates/core`: the store, git, projects, workspaces, settings, harnesses, the launch
  environment and the daemon client, with **no Tauri dependency**. The app re-exports it under
  the module names it always used, so the change inside `src-tauri` is small and the generated
  bindings came out byte-identical.
- `crates/cli`: `ys`, a second client of that core. `project list`, `workspace list`,
  `workspace new <project> "<prompt>"`, `session list`, `doctor`; `--json` on all of them.
- `ys workspace new` creates the branch and worktree and starts the agent with the prompt as its
  first message — the composer's job, from a terminal. The agent belongs to the daemon, so it
  outlives the command.
- Shipped as its own archive in each release, per platform. Not self-updating, and no package
  manager carries it.

_Exit:_ `ys workspace new` from a terminal leaves a working agent behind after the command
returns, and the app shows it when it next looks.

_Result:_ verified by hand on Linux against a throwaway profile with a stand-in agent: `ys`
started the daemon from its own binary, the agent survived `ys` exiting, `ys session list`
reported it running, and git had the branch and worktree. `ys doctor` against the real profile
resolved the same directory Tauri does — the best evidence available that `paths.rs` matches,
short of running it on the other two platforms.

_Notes:_

- **Why a separate binary and not a flag.** The app's binary is built with
  `windows_subsystem = "windows"`, so in release it has no console and would print nothing on
  Windows. `ys` is its own executable in its own crate for that reason. It is also why the
  command is `ys` and not `yardsort`: two binaries of one name in a cargo workspace collide in
  `target/`.
- **Why the core had to move.** Linking the app's lib would have pulled in Tauri, and on Linux
  that means a command-line tool that will not start without WebKitGTK. Almost all of it was
  already Tauri-free — seven of the moved files had not one reference.
- Found by the move: every write to the store was a `BEGIN DEFERRED` that reads before it
  writes, which SQLite refuses outright when another connection holds the write lock rather than
  waiting for it. Harmless while the app was the only writer. Write transactions are immediate
  now.
- Found while testing: the daemon keeps exited sessions in its list so their last screen can
  still be read, so "is this session alive?" is a question about its _state_, not about whether
  the daemon has heard of it. `ys session list` distinguishes `running`, `gone` (the record says
  running but the process is not — nothing was connected to hear it exit) and `running?` (no
  daemon to ask).
- `ys` never creates a database. Working the data directory out without Tauri to ask is the one
  thing that could quietly differ on a platform this has not been tried on, and a CLI that
  created one would answer every question with a convincing, empty "no projects". The app warns
  at startup if the two resolutions disagree.
- Found by the first release that carried it: `ys` cannot be signed in a later step of the macOS
  job, because `tauri-action` imports the certificate into a keychain of its own and takes it
  away again. The signing step was dropped rather than reimplementing the import, since signing
  without notarization spares nobody the quarantine prompt — a bare executable cannot have a
  ticket stapled to it. Doing it properly means importing the certificate ourselves _and_
  notarizing, and even then Gatekeeper checks over the network.
- `ys attach` followed: raw mode, size matching and a `Ctrl-]` that detaches without stopping
  anything. Input is forwarded as **raw bytes**, never as parsed key events, so mouse reporting,
  bracketed paste and anything crossterm does not model survive the trip. Reading happens on a
  thread of its own: a blocking read in the loop would hold on until the next keystroke, and an
  agent finishing while nobody types is exactly what one attaches to see.
- Found by attaching through a pty that had never been given a size (`script` with piped input
  gives one, reporting 0×0): the size was passed straight through, and resizing a session to
  zero columns throws away the screen the daemon holds — the screen that repaints the app's
  window and the next attach. A degenerate size is now ignored rather than forwarded. The bug
  destroyed something a _different_ client was relying on, which is the kind a single-client
  design never shows you.
- `ys logs` reads a session's screen, finished ones included, by replaying the snapshot through
  a headless terminal rather than stripping escapes out of it — a snapshot is _state_, not the
  text in order, so a pattern would have been guesswork. `pty_host::snapshot::to_text` does it
  where the VT knowledge already lives.
- The limit that came with it, which is the daemon's design rather than a gap: screens are held
  in the daemon and never written to disk, and the daemon stops once nothing is connected and
  nothing is running. So a finished agent's last screen outlives the agent but not the daemon.
  Keeping screens would mean writing whatever an agent printed to disk, which is a decision
  about secrets rather than about storage — not taken here.
- Still open: `ys` is unsigned on macOS and no package manager ships it. None of its terminal
  handling — raw mode, `Ctrl-]`, resize forwarding — nor its idea of where the data directory is
  has been exercised by a human on macOS or Windows; a test can check neither. Section 6 of
  [08-manual-checklist](08-manual-checklist.md) is the pass, and it is owed alongside M7's.

## Later (unordered)

- Commit / push / open PR from the UI; show PR + CI status on the workspace row.
- ~~**Have a model write the commit message and the pull request.**~~ **Done**, though it was
  never on this list — it came out of dogfooding the item above. The finding worth keeping: Assist
  could not do it. Jev answers typed questions and never writes text, so this needed a generative
  model, which Yardsort had never called. What made it cheap anyway is that every agent already
  has a non-interactive mode, so the writer is the agent the user already installed, logged into
  and pays for — one new harness field, no new credential. An Anthropic API key is the fallback.
  See [open question 19](06-open-questions.md).
- [x] Per-project setup script and "files to copy into new worktrees" (`.env` etc.); run/dev-server button. Local SQLite configuration; preparation shared with the CLI, failed preparation retained for inspection, run output in daemon terminal tabs.
- Merge / rebase helpers; "apply this workspace onto local".
- Diff comments sent back to the agent as a prompt.
- Multi-repo projects; remote/SSH workspaces.
- Usage / cost view per workspace. MCP config management per harness.
- Command palette; themes; Omarchy theme integration.

### Measured against Superset

[Superset](https://superset.sh) is the app Yardsort is modelled on and cannot run on Linux or
Windows — which is [why this exists](01-vision.md). Going through what it offers, most of the
gaps were already in the list above. These four were not, and are worth naming rather than
rediscovering later. None of them is committed to; the point is to know what is missing.

- ~~**Side-by-side diffs.**~~ **Done.** `MergeView` came from the `@codemirror/merge` already
  depended on, so it was a branch in `CodeView` and a toggle in the viewer's header, remembered
  in `ui_state`. Inline stays the default: it is the better reading of a small change, and the
  narrow panel is where most diffs are opened.
- **Automations — agents on a schedule.** Superset turns chores into recurring agents: issue
  triage, changelog drafts, dependency bumps, with last-run and current status. Yardsort has no
  notion of time at all. M9 is what makes it plausible: something has to be running when nobody
  is looking, and now something is — the daemon already outlives the window and already owns
  session lifetime. The hard parts are not the timer but the answers: what a run does when the
  previous one is still going, what happens to a worktree per run, and how a scheduled agent
  asks for permission when there is no one to ask.
- ~~**A `yardsort` CLI.**~~ **Done, as `ys` — see M10.** The estimate above was wrong in an
  instructive way: the daemon protocol carries _sessions_, not workspaces, and projects live in
  the SQLite store the app owns. "A second client of the socket with no new core" needed the
  core to be extracted first.
- **Mobile.** Superset has an iOS app for steering agents from a phone. This one is not a small
  feature but a different product shape: it needs remote workspaces first (already in the list
  above), and then something to connect to from outside the machine — which runs into "team
  features, accounts" that [01-vision](01-vision.md) puts out of scope. Listed for honesty, not
  as a plan.

Two further Superset ideas are already covered elsewhere: richer per-workspace status ("running",
"blocked", "ready for review") is what [open question 16](06-open-questions.md) decided to build
with Assist, and its PR view is the first item in the list above — now done, as far as a pull
request's number and its checks go. What Superset still has and this does not is the review
itself: comments, files reviewed, merging from the app.
