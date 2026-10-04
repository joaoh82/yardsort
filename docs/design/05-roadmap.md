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
  manager carries it. _Since superseded:_ every bundle now carries `ys` as a Tauri sidecar; the
  packages and the Homebrew cask put it on `PATH`, the app offers to elsewhere (a link on macOS,
  a copy refreshed at startup for an AppImage and on Windows), and it updates with the app — see
  `src-tauri/src/ys.rs`. The loose archives stay for anyone who wants `ys` without the app.

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
- Found when someone downloaded that unsigned copy and reported the app as unsigned (#93): none
  of that was needed. The `ys` inside the released app is signed on its own, and Apple holds a
  notarization ticket for it by itself — the notary service issues one for every executable in a
  submission. The macOS archive now carries that copy, taken back out of the built app. Checked
  against 0.17.0 by computing each architecture's code-directory hash and asking Apple's ticket
  service (`scripts/notarization-ticket.py`); what Gatekeeper then does with a quarantined copy
  has not been watched on a Mac yet, and is the first row of the checklist's section 6.
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
- Still open: no package manager ships `ys` by itself. None of its terminal
  handling — raw mode, `Ctrl-]`, resize forwarding — nor its idea of where the data directory is
  has been exercised by a human on macOS or Windows; a test can check neither. Section 6 of
  [08-manual-checklist](08-manual-checklist.md) is the pass, and it is owed alongside M7's.

## M11 — Agent events, stage 0–1 ✅

The first slice of [09-agent-events-and-memory](09-agent-events-and-memory.md), built after the
fit pass recorded in [10-agent-events-stage-1](10-agent-events-stage-1.md).

- Migration 0009: `agent_runs`, `agent_events`, `agent_event_diagnostics`.
- `crates/core/src/activity.rs`: the event contract (schema 1), the `Recorder` that never lets a
  failure out, exit recording with idempotent source keys, spool import, reconciliation, retention.
- `Launcher` writes a run before every workspace spawn, labels the PTY `run`, and gives the child
  `YARDSORT_RUN_ID` / `YARDSORT_WORKSPACE_ID` / `YARDSORT_SESSION_RECORD_ID`. Resume and fork go
  through the same path (`start_recorded`).
- The daemon spools every `Exited` to `<data-dir>/activity/spool` when given the directory;
  protocol unchanged. The app and `ys` drain it on connect, on every live exit, and before
  reconciling "interrupted" rows — so a clean exit while the window was closed keeps its code.
- Settings → General: `record_lifecycle` (on) and `show_timeline` (off, experimental); an
  **Activity** panel in the workspace footer; `ys activity list|export`; `ys doctor` rows.
- Stage 0: seven harnesses' versions and native surfaces recorded from their own `--help`; no
  hook, OTLP or session-file adapter, and no fixture, exists yet.

_Exit:_ app/CLI launch, resume, fork, two workspaces, immediate exit, closed-window exit,
duplicate delivery and a broken table all covered by tests that fail without the change; `just
check` and `just bindings-check` green; no PTY byte or input path touched.

## M12 — Agent events, stage 2: Claude Code ✅

The first native adapter, recorded in [11-agent-events-stage-2-claude](11-agent-events-stage-2-claude.md).

- `scripts/record-claude-hooks.sh` and twenty real payloads from Claude Code 2.1.280 under
  `crates/core/fixtures/claude-hooks/`: fresh, resumed and forked sessions, a failed tool; the
  launcher's environment shown to reach the hook.
- `activity/claude.rs`: a per-launch `--settings` file whose hooks run the Yardsort executable as
  an argument list; `normalize()` keeps metadata only. `activity/hook.rs`: the `--yardsort-hook`
  mode in both binaries, always exit 0. `activity/inbox.rs`: the spool's twin for reports.
- `Launcher` arms a recorded `claude` launch when `capture_claude` is on; the run's start says
  `capture: "hook"`. The app drains on start, exit and inbox change; `ys` with the spool.
- Settings → General switch (off by default), inbox diagnostics, the timeline's words for
  reported events and a live reload, `ys doctor` and `ys activity list` rows.
- Coverage: sessions, prompts, tools, permissions, turns, subagents, compaction and model
  switches for Claude Code; token usage is not available through hooks. Everything else stays
  lifecycle-only.

_Exit:_ every fixture maps and leaks nothing; arming rules, the hook binary end to end, inbox
placement by run / native session / workspace, double drains and bad files covered by tests that
fail without the change; `just check`, `just bindings-check`, `just lint-windows` green; a real
Claude Code launched through `ys` on Linux seen reporting live — which is how the `--settings`
placement bug was found before it shipped. The app-window rows, macOS and Windows are in
[08 §12](08-manual-checklist.md#12--claude-code-reporting).

## M13 — Agent events, stage 2: Codex ✅

The second native adapter, recorded in [12-agent-events-stage-2-codex](12-agent-events-stage-2-codex.md).

- `scripts/record-codex.sh` and fixtures from Codex 0.156.1: the session file of one `exec`
  turn, the twelve hook payloads (Claude Code's shape, delivered only under the trust bypass),
  and the `notify` argument.
- The decision the fixtures forced: not hooks (trust-gated, no per-launch way through that is
  safe to pass for a user), but `notify` as the per-launch trigger — an argument list, no shell,
  no review, the user's own `notify` chained — and the session file, read at drain time, for
  commands with exit codes and durations, file changes with paths, MCP calls, token usage and
  turn timing.
- `activity/codex.rs`; the hook mode takes its payload as the last argument; the drain expands a
  trigger with per-fact source keys, waits for a turn the file has not finished, and falls back
  to the trigger alone after five minutes or when the file cannot be read.
- Settings → General **Capture what Codex reports** (off); timeline words for files and tokens.

_Exit:_ fixtures map and leak nothing; arming, the hook binary as `notify`, expansion linked to
the run and idempotent, the wait and both fallbacks covered by tests that fail without the
change; `just check`, `just bindings-check`, `just lint-windows` green; a real Codex launched
through `ys` on Linux seen reporting a turn from its own session file. Hands-on rows in
[08 §13](08-manual-checklist.md#13--codex-reporting).

## M14 — Agent events, stage 2: OpenCode ✅

The third native adapter, recorded in [13-agent-events-stage-2-opencode](13-agent-events-stage-2-opencode.md).

- `scripts/record-opencode.sh` and 80 fixtures from OpenCode 1.18.31: every plugin hook call and
  bus event of one `run` turn, recorded through the same per-launch channel the adapter uses.
- `activity/opencode.rs` and `opencode-plugin.js`: a plugin written before each launch and given
  through `OPENCODE_CONFIG_CONTENT`, merged by OpenCode with the user's own; `reduce()` keeps a
  whitelist of fields inside OpenCode's process and hands each call to the executable in hook
  mode on stdin. `--pure` is respected. `ResolvedLaunch` carries per-launch environment.
- Settings → General **Capture what OpenCode reports** (off); the plugin's `reduce` tested under
  vitest over the same fixtures the Rust side reads.
- Coverage: sessions, prompts, tools with exit codes and failures, file edits, permissions, turns,
  token usage and cost per reply. Not available: a successful tool's duration.

_Exit:_ fixtures map and leak nothing on both sides; arming, merging and the `--pure` refusal;
the hook binary end to end; covered by tests that fail without the change; `just check`,
`just bindings-check`, `just lint-windows` green; a real OpenCode launched through `ys` on Linux
seen reporting live. Hands-on rows in [08 §14](08-manual-checklist.md#14--opencode-reporting).

## M15 — Agent events, stage 2: Grok ✅

A fourth native adapter, recorded in [14-agent-events-stage-2-grok](14-agent-events-stage-2-grok.md).

- `scripts/record-grok.sh` and fixtures from Grok 1.0.41: the session directory's `events.jsonl`,
  `usage.json` and `summary.json`, plus the 13 hook payloads kept as the record of the
  alternative.
- The decision: not hooks — no per-launch way to give them to the TUI, so they would mean a
  file of ours in the user's home — but Grok's own session directory, metadata-only by design,
  found by the id Yardsort chose and read as it grows: `activity/grok.rs`, `Store::live_runs`,
  a cursor per run in the app's watcher, which now ticks while a Grok run is going.
- Settings → General **Read what Grok records** (off); `turn.started` as a new kind; waits and
  outcomes on the timeline and in `ys activity list`.

_Exit:_ the log, usage and summary map and leak nothing; cursors, duplicates, the ten-minute
grace; the real `ys` binary reading a fixture directory; covered by tests that fail without the
change; `just check`, `just bindings-check`, `just lint-windows` green; a real Grok launched
through `ys` on Linux read live. Hands-on rows in [08 §15](08-manual-checklist.md#15--grok-reporting).

_Result:_ 2026-09-24, Linux. Rust: 184 core tests (11 new in `activity.rs`, 7 in `launch.rs`
against a real `PtyHost` and a plan-catching host), 3 new spool unit tests, a real-daemon test in
`pty-ipc/tests/daemon.rs` (exit spooled after the only client left, live exit spooled too), 2
`ys` end-to-end tests (`activity list/export`, spool drained before `session list`). Frontend: 5
new Testing Library tests (timeline paging and clearing, settings switches, footer toggle). By
hand, against a throwaway profile with a shell standing in for an agent: `ys workspace new`
started the daemon and the run, the process exited with 5 while nothing was connected, the daemon
wrote the spool entry and then idled out ("nothing left to look after"), and the next `ys session
list` drained it — record `ended`, `ys activity list` showing `exit 5, via spool`, `ys doctor`
showing 0 waiting. The same through the window, later that evening: Claude started from the
composer, the window closed with **Leave them running**, the agent ended through `ys attach`, the
app reopened and its timeline read `via spool`, the session listed as ended. macOS and Windows: CI
runs every Rust test including the daemon spool test on both; the hands-on pass is owed, rows in
[08 §11](08-manual-checklist.md#11--activity).

_Notes:_

- The daemon stays storage-free. Giving it SQLite would have made it a second writer on a file
  the app and `ys` already share; a directory of tiny files with atomic renames needs no lock
  and no protocol bump, and a client that reads a file the daemon is still writing cannot happen
  because the daemon renames it into place.
- `sessions` rows benefit too: the spool import calls `end_session_by_pty`, so Resume no longer
  labels a clean unattended exit _interrupted_.
- Found by the launcher tests: `Launcher` had no idea who was launching. It now carries
  `launched_by`, which the timeline shows as "from ys".
- Not done, on purpose: no Claude/Codex/OpenCode hook is configured, no OTLP receiver exists, no
  Jev call is made, no memory. Stage 2 starts with recorded Claude Code hook fixtures.

## M16 — Agent events, stage 2: OMP and Pi ✅

One adapter for the two forks, recorded in [15-agent-events-stage-2-pi-omp](15-agent-events-stage-2-pi-omp.md).

- `scripts/record-pi.sh pi|omp` and fixtures: OMP 18.2.11 in full (44 extension events and the
  session file), Pi 0.87.1 in full the next day (48 events; it had no provider on the first).
- The decision: the plugin pattern again — a TypeScript extension written from a template in
  the binary, given with `-e <file>` for one launch, never in the user's extension directories;
  `reduce` in the extension keeps the whitelist before anything leaves the agent's process, and
  is tested in vitest against the same fixtures the Rust side reads.
- Settings → General **Capture what OMP reports** and **Capture what pi reports** (off);
  `capture: "extension"` on the run's start; producers `omp` and `pi`.

_Exit:_ every OMP fixture maps and leaks nothing; Pi's recording (partial at first, in full on
2026-09-26) carries the assigned id; arming for either, refusing `--trusted-extension`; the real
`ys` as the hook; `just check`, `just bindings-check`, `just lint-windows` green; a real OMP launched through `ys` on Linux
reporting live. Hands-on rows in [08 §16](08-manual-checklist.md#16--omp-and-pi-reporting).

_Result:_ 2026-09-25, Linux. Rust: `activity::pi` (6 tests), `launch::tests` (1), a `ys`
end-to-end test; vitest over the OMP fixtures through the extension's own `reduce`. By hand,
against a throwaway profile: `ys workspace new … --harness omp` started OMP with `-e` and the
run's start said _reporting via extension_; within seconds `ys activity list` showed
`session.started` with the model, `turn.started`, `write` started and done with the file's
name, `turn.completed` with duration and tokens, all `omp/extension`; the payloads held no
command, message or content; ending the process recorded `session.ended` from the extension and
`process.exited` from the spool. Found: OMP raises no `input` for the opening message given on
the command line, so a launch's first prompt has no `prompt.submitted` row (typed ones do); and
the timeline's turn row now shows the turn's tokens, which the pi family and Cursor report on
the turn itself. Pi: not run live — no provider on the machine. macOS and Windows: CI runs the
tests; the hands-on pass is owed.

## M17 — Agent events, stage 2: Cursor ✅

The last built-in harness, recorded in [16-agent-events-stage-2-cursor](16-agent-events-stage-2-cursor.md).

- The decision: Cursor's hooks, Claude Code's in shape, through `--plugin-dir <dir>` for one
  launch — a manifest and a `hooks.json` written under the data directory before each launch,
  the command a single-quoted shell string because that is the only form Cursor takes. Passive
  events only; nothing that decides.
- **Built from the documentation, recorded the next day.** The agent was not logged in on the
  machine at hand when it was written; `scripts/record-cursor.sh` ran on 2026-09-26 (19 hooks),
  the tests now read the recording, and it found fractional durations the adapter had been
  dropping, the account's email on every hook, and three hooks this build never delivers.
- Settings → General **Capture what Cursor reports** (off); `capture: "hook"`; producer `cursor`.

_Exit:_ the documented shapes map and leak nothing; arming writes the plugin and names it before
`--`, only when asked; the real `ys` as the hook; the gates green. The recording was made on
2026-09-26 (#52), and the Linux hands-on rows of
[08 §17](08-manual-checklist.md#17--cursor-reporting) were run the same day. Owed: macOS and
Windows.

## M18 — Agent events, stage 3: review and provenance (first slice) ✅

The join of what the agents reported to what git shows, recorded in
[17-agent-events-stage-3-review](17-agent-events-stage-3-review.md).

- The decision: joined at read time, per path, from the three event kinds that can carry a
  write, with the harness's writing tools named per producer; nothing stored, no
  `workspace.changed` event. The join also lists the workspace's agent runs and how each was
  asked to report, which is what lets the panel say why a file has no report — and stay silent
  when no run was reporting.
- Changes list: a badge per reporting agent on a changed file, a counting line above the list,
  a word in the expanded viewer's header. Activity timeline: a **Show diff** button on a row that
  names a listed file. Every word stops short of a line.
- Second slice (2026-09-26): **observed writes**. A file the agent made with a shell command
  has no report, but its own modification time falls inside the command's window on the
  timeline; that is shown as an observation — a dashed badge, worded as what Yardsort read and
  what it did not — and ranked below a report. Only a tool call is a window, never a run.
- Third slice (2026-09-26): **the Jev half**, after a four-question design pass. A switch
  under Assist, off by default, sends who wrote each file in the Changes list's own sentence
  with each diff — never a tool, time or command — and a file no agent accounted for gets one
  more question and an **unaccounted** badge. Every badge's tooltip ends with what was told.

_Exit:_ reported writes are told apart from git-observed changes on the panel, with the
alternatives named; no claim of line-level causality anywhere; the gates green. The Linux
hands-on pass of [08 §18](08-manual-checklist.md#18--reported-writes-beside-the-diff) was run on
2026-09-26 and 27, observed writes included. Owed: macOS and Windows.

## M19 — Agent events, stage 4: handoffs (first slice) ✅

The user story the design opened with, recorded in
[18-agent-events-stage-4-handoff](18-agent-events-stage-4-handoff.md).

- The decision: a deterministic packet — the task whole, the branch and its commits, each
  changed file with the Changes list's word on who wrote it, one entry per run with what it
  did, and what is not there — as the next agent's first message, previewed and edited in the
  composer, sent through the ordinary prompt transport. No model in the loop; Jev's ranking
  waits for use.
- A handoff is not the task: its session record keeps no prompt, so `session_prompts` — what
  Assist and the next packet read — stays the user's own words.
- Second slice (2026-09-27): the change list moved to the core (`yardsort_core::changes`),
  and with it the packet's git half; **`ys workspace handoff <workspace>`** prints the packet;
  and, from the review Assist already runs, the changed files come in the order Assist would
  look at them with its word on each — a judgment, the packet says, not a fact. Nothing new is
  sent, and no new switch.

_Exit:_ the packet renders every part from a recorded run and leaks nothing; **Hand off…**
opens the composer here with it; an edited packet spawns with `handoff: true` and creates no
workspace; the gates green. On Linux, **Hand off…** and the packet in the composer were seen in the
window (they are the 0.11.0 screenshot); sending it on to a second agent has not been run by hand.
Owed: that, the rest of [08 §19](08-manual-checklist.md#19--handing-off), and macOS and Windows.

## M20 — Agent events, stage 5: reviewed memory (first slice) ✅

Recorded in [19-agent-events-stage-5-memory](19-agent-events-stage-5-memory.md), after a
four-question design pass.

- The decisions: the user writes entries (approved as written) and agents propose them with
  `ys memory propose` (always candidates); approved entries reach agents after a first message
  at launch — never in the record — and in handoff packets, when the project shares them, and
  through `ys memory list | search`; review in a Memory view per project; Jev's repeat and
  contradiction checks behind their own Assist switch.
- No approve command anywhere an agent can reach: that is how "no agent approves its own
  candidate" is enforced. No MCP server: every agent runs `ys`, not every agent takes MCP.
- Not built: model extraction of candidates, an MCP server, directory and global scopes.

_Exit:_ unapproved candidates never reach an agent (launch, handoff and `ys` tests); revocation
and edits work with history; injection is opt-in per project and per launch (since 4 October
2026 on until turned off, per project: [19 §2](19-agent-events-stage-5-memory.md#2--decisions));
the gates green.
On Linux, writing an entry, an agent's proposal through `ys` and approving it were tried by hand.
Owed: the rest of [08 §20](08-manual-checklist.md#20--project-memory) — sharing into a launch,
the opt-out, revoke and restore, the handoff section, Jev's tags — and macOS and Windows.

## M21 — Agent events, stage 6: outcomes (first slice) ✅

Recorded in [20-agent-events-stage-6-outcomes](20-agent-events-stage-6-outcomes.md), after a
four-question design pass.

- The decisions: a workspace is an attempt at its first message; the user labels it kept, partly
  or discarded — prompted once after an archive or delete, and any time in an Outcomes view per
  project; a merged pull request or a git merge counts as kept until labelled; nothing weaker is
  ever an outcome. Per-agent history beside the composer's picker, across projects, and "too few
  to say yet" below five outcomes. Jev later.
- An outcome carries its own copy of what it describes, snapshotted in the core before a delete
  removes the workspace and its sessions. Git's "merged" needs the branch to have been seen ahead
  first; a squash merge is caught by the pull request's state.
- Not built: Jev's judgment, suggestions from history, grouped attempts, test evidence.

_Exit:_ labels, merges and weak evidence are kept apart in the data and the words; histories
never speak below the sample size; the gates green. Owed: the hands-on pass in
[08 §21](08-manual-checklist.md#21--outcomes), on all three platforms.

## M22 — Workflows

Proposed in [21-workflows](21-workflows.md), after an eight-question design pass on 2026-09-28.

- Named sequences of agent work in YAML under the user's data directory, never a repository's:
  inputs, a DAG of steps (`start_session`, `wait_session`, `send_to_session`, `wait_pr_activity`,
  `notify`), `{{ variables }}` checked at load time, one trigger — manual. A built-in
  **Request code review**: a second agent reviews the workspace's pull request in the same
  worktree, posts with `gh`, and when the forge shows the review the user is told and so is the
  agent that wrote the PR, once it is quiet.
- The engine is a pure `advance` in the core over rows in SQLite; the app drives it and executes
  the effects. `ys workflow list | show | validate | run | runs | cancel`; a run from `ys` needs
  the app open and reaches it through the activity inbox.
- WORKFLOWS above PROJECTS in the sidebar; a view with a read-only flow chart, a YAML editor with
  the validator's errors, and run history. **Describe it** writes a draft through Drafting, never
  Jev, and never saves without the user.
- Six slices, each its own pull request: core and file; engine and CLI; forge step and the
  built-in end to end; UI; Describe it; docs and pictures.
- [x] Slice 1: the file format, the validator with a line and column on every problem, the
      built-in, the catalog of built-ins and user files, `ys workflow list | show | copy | validate`, and
      the guide to the format. Recorded in [21 § slice 1](21-workflows.md#slice-1-what-shipped).
- [x] Slice 2: runs. Migration 0012; the engine, a pure function over the run's rows; the driver,
      a background thread in the app that starts agents, waits for them to settle, types to them and
      notifies; the app lock that tells `ys` the app is running; `ys workflow run | runs | cancel`.
      Recorded in [21 § slice 2](21-workflows.md#slice-2-what-shipped).
- [x] Slice 3: the pull request and the workspace's own agent. Migration 0013 keeps what a run
      found out; `wait_pr_activity` asks `gh` every 30 s; `session: origin` is resolved when the
      run starts. The built-in code review runs end to end. Recorded in
      [21 § slice 3](21-workflows.md#slice-3-what-shipped).
- [x] Slice 4: the Workflows section above Projects, the view (chart, editor with the
      validator's marks, runs) and the Run dialog, also from a workspace's menu. Recorded in
      [21 § slice 4](21-workflows.md#slice-4-what-shipped).
- [x] Slice 5: Describe it. A description written into a workflow by the agent the user has or
      their Anthropic key, through Drafting, checked and sent back once with its problems, into
      the editor unsaved. Recorded in [21 § slice 5](21-workflows.md#slice-5-what-shipped).
- [x] Slice 6: docs and pictures. The README's highlight and section, the website's section, the
      quick start, and screenshots of the view and the Run dialog. Recorded in
      [21 § slice 6](21-workflows.md#slice-6-what-shipped).

_Exit:_ the code-review workflow runs against a real pull request on all three platforms from the
window and from `ys`; a busy agent is never written to; every validation error has a failing
fixture; the docs list every action and variable that exists and none that does not.

## M23 — Pull requests

Proposed in [22-pull-requests](22-pull-requests.md), after a ten-question design pass on
2026-10-02. All four slices are built; the screenshot and the hands-on pass remain.

- A **Pull requests** row at the top of the sidebar, above Workflows, opens a view in the center
  panel: every pull request of every project's repository, with state, project, author and
  review filters and a search, all but the search remembered. GitHub through `gh` only; no
  credential of Yardsort's own.
- A detail pane with **Summary** (description, checks with links, reviewers, conversation) and
  **Code** (the diff, from the pull request's commits fetched into refs of Yardsort's own, shown
  with the changes panel's viewer). Start a workspace from a pull request, merge, close, reopen —
  each confirmed, a _no_ always respected.
- Found before any code was written: `gh pr list` cannot return 200 pull requests with their
  checks from a busy repository — GitHub answers 502 or 504 after ten seconds. So the open pull
  requests come from a query of our own through `gh api graphql`, fifty to the page, asking for
  check _counts_ and not every check. The measurements are in
  [22 § what was measured](22-pull-requests.md#what-was-measured).
- Four slices, each its own pull request:
- [x] Slice 1: the list, the filters and the actions. The open tier through `gh api graphql`,
      paged, beside the existing list; the sidebar row, the view, its filters remembered in
      `ui_state`; merge, close, reopen and Start workspace, the last through the composer. A
      fork's pull request becomes a branch whose upstream is the pull request, which settled
      [open question 25](06-open-questions.md). Found: taking the list's panel out of the
      tree while the details stayed tripped an assertion in the panel library, so hiding the
      list collapses its panel, as the shell's side panels do; and jsdom lays nothing out, so the
      view was also driven in headless Chromium with demo data. Not done: the screenshot, which
      wants a real window. Recorded in
      [22 § slice 1](22-pull-requests.md#slice-1-what-shipped).
- [x] Slice 2: Summary. `gh pr view` for one pull request in full, kept for 30 s; the description
      and the conversation as Markdown with no HTML, no image loaded and every link opened in the
      browser; every check with its workflow and a link to its run; a summary that is read again
      when the list says the pull request changed. Recorded in
      [22 § slice 2](22-pull-requests.md#slice-2-what-shipped).
- [x] Slice 3: Code. A pull request's commits fetched into `refs/yardsort/pull/<n>/`, never
      checked out; its diff measured from the base commit the forge recorded, so one merged long
      ago still shows; the changes panel's viewer shared rather than copied; refs dropped when a
      pull request leaves a list that is whole. Recorded in
      [22 § slice 3](22-pull-requests.md#slice-3-what-shipped).
- [x] Slice 4: writing. A reply box; every comment on lines beside the lines it is about, in
      the viewer; selected lines sent with a note to the pull request's agent by the conflict
      helper's rules, or as a new workspace's first message, and optionally to GitHub as a
      comment on those lines. Recorded in
      [22 § slice 4](22-pull-requests.md#slice-4-what-shipped).

_Exit:_ on all three platforms, a repository with more open pull requests than one page lists
them, filters them and says when there are more than it shows; a pull request from a fork can be
read, diffed and started as a workspace; a _no_ to Merge, Close and Reopen sends nothing; and
without `gh`, logged out, or on another forge, the view says which and nothing else in the app
changes.

## M24 — Tasks

Proposed in [23-tasks](23-tasks.md), after a four-question design pass on 2026-10-03. All three
slices are built.

- A **Tasks** row in the sidebar, under Pull requests, opens a view in the center panel: every
  project's GitHub issues, with state, project, label, assignee, author and _needs an answer_
  filters and a search. GitHub through `gh` only; other sources later, behind one trait in the
  core.
- **Delegate** opens the composer with the issue as the first message, framed as text other
  people wrote; the workspace started from it is recorded against the task (migration 0014) and
  never guessed.
- Managing from the view — new task, reply, close, reopen, labels, assignees — and all of it in
  `ys task`, with `--json`, so an agent can list what is open, say which need an answer, and
  file an issue. `ys` writes straight to GitHub; `close` and `reopen` want `--yes`.
- Measured before any code: issues are cheap beside pull requests — 50 rows in under 2 s on a
  repository with 11,238 open. The numbers are in
  [23 § what was measured](23-tasks.md#what-was-measured).
- Three slices, each its own pull request:
- [x] Slice 1: read — the source, the list, the filters, the detail; `ys task list` and `show`.
      The sidebar's number is the tasks that need an answer, which settled
      [open question 28](06-open-questions.md). Found: bots comment on most issues of a busy
      repository and some carry a contributor's association, so what is a bot is the author's
      type, and the last five comments are asked for rather than the last one; and
      `gh issue view` cannot say whether an answer is owed, so one issue in full is a query of
      our own too. Not done: the screenshot and the README, which wait for the slice that
      delegates. Recorded in [23 § slice 1](23-tasks.md#slice-1-what-shipped).
- [x] Slice 2: delegate — the message, the composer, the link, `ys task start`. Migration 0014
      records which task a workspace was started from. The message puts everything a stranger
      wrote between two lines carrying a mark made for that message, and leaves bots out. Found:
      a test that makes workspaces must say where worktrees go, or it makes them in the real
      place. Recorded in [23 § slice 2](23-tasks.md#slice-2-what-shipped).
- [x] Slice 3: manage — the writes, in the view and in `ys task`. New task, reply, close and
      reopen (each confirmed, a _no_ sending nothing), labels, assignees; `ys task create`,
      `comment`, `close`, `reopen`, `edit`, the last two wanting `--yes`. Measured first, as
      asked: a page of assignable people is 2 s on a repository with 178 of them. Not done: the
      screenshot, and a write to a real repository — every write is tested against a stand-in
      `gh`. Recorded in [23 § slice 3](23-tasks.md#slice-3-what-shipped).

_Exit:_ on all three platforms, a project's issues are listed, filtered and read; one is
delegated through the composer and its workspace is found again from the task; a _no_ to Close
and Reopen sends nothing; an agent asked which issues need an answer answers from
`ys task list --needs-answer --json`, and one asked to file an issue does; and without `gh`,
logged out, or on another forge, the view and `ys` say which and nothing else changes.

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
- Merge / rebase helpers; "apply this workspace onto local". A first piece is done: a pull
  request GitHub says conflicts can be handed back to the agent that opened it, which merges the
  base in and pushes (see [commits & pull requests](../guide/commits-and-pull-requests.md#resolve-merge-conflicts)).
  Found while building it: a workspace can own several pull requests, and the branch name alone
  cannot say which — the worktree's own `HEAD` reflog can.
- ~~Diff comments sent back to the agent as a prompt.~~ **Done** for a pull request's diff, as
  slice 4 of [M23](#m23--pull-requests): selected lines and a note go to the agent in the pull
  request's workspace by the conflict helper's rules.
- Multi-repo projects; remote/SSH workspaces.
- ~~**Usage / cost view per workspace.**~~ **Done**, as the Usage view: token usage from the
  agents' own logs, and machine resources per terminal. What was found: the figures were already
  on disk — Claude Code writes the API's `usage` on every reply, Codex a `token_usage_record` per
  response and its plan's rate limits, Grok a `usage.json` per session — so no hook, plugin or
  network call was needed, and the count covers conversations started outside Yardsort too.
  Claude Code writes a reply once per content block and copies earlier replies into a resumed
  conversation's file, so counting needs its message and request ids; adding up lines naively
  overcounts. Prices are a table in the core, dated, and a model not in it is counted but never
  priced by a near match. Machine figures are a terminal's whole process tree, taken from the
  daemon's session pids before the app's and host's own trees, so nothing is counted twice with
  or without a daemon. Not done: OpenCode, OMP, pi and Cursor usage; Claude Code's and Grok's plan
  limits, which they do not write down.
- MCP config management per harness.
- Command palette; themes; Omarchy theme integration.

### Deferred from the agent-events stages

Each stage of [09](09-agent-events-and-memory.md) shipped a first slice and named what it left
out. Gathered here so they are found in one place; each note says why it waited.

- **Jev's judgment of an attempt.** Whether an attempt met its request, from its task and final
  diff, shown beside the user's label and never counted as an outcome. Waits for labels to measure
  it against. [20 §5](20-agent-events-stage-6-outcomes.md#5--not-in-this-slice)
- **Suggestions from local history.** Ranking an agent and effort for a task from the user's own
  outcomes, beside Assist's suggestion. Same wait. [20 §5](20-agent-events-stage-6-outcomes.md#5--not-in-this-slice)
- **Try with several agents.** Start one task in two or three workspaces with different agents,
  grouped as attempts at one task, for a head-to-head comparison.
  [20 §2, §5](20-agent-events-stage-6-outcomes.md#5--not-in-this-slice)
- **Test evidence.** No adapter records output, so a passing test run is not evidence of anything
  yet — for outcomes or for handoffs. A pull request's check results are available and could be
  shown beside an attempt. [18 §5](18-agent-events-stage-4-handoff.md#5--what-this-slice-does-not-do-and-the-next),
  [20 §5](20-agent-events-stage-6-outcomes.md#5--not-in-this-slice)
- **A read-only MCP server for memory**, for the agents that take one per launch. `ys memory`
  covers every agent today. [19 §5](19-agent-events-stage-5-memory.md#5--not-in-this-slice)
- **Model extraction of memory candidates** from a finished workspace — generative, with its own
  opt-in. [19 §5](19-agent-events-stage-5-memory.md#5--not-in-this-slice)
- **Directory and global memory scopes**, after project scope has proven itself.
  [19 §5](19-agent-events-stage-5-memory.md#5--not-in-this-slice)
- **Relevance ranking of memory for a task**, rather than newest first, 40 at most.
  [19 §5](19-agent-events-stage-5-memory.md#5--not-in-this-slice)
- **Reported writes attributed to commits.** A file an agent wrote and then committed keeps its
  badge, but reports are not tied to a commit, so one written by one run and committed by another
  says both. [17 §5](17-agent-events-stage-3-review.md#5--what-this-slice-does-not-do-and-the-next)
- **Handoff excerpts cited by event id**, rather than a count and the `ys activity list` line.
  [18 §5](18-agent-events-stage-4-handoff.md#5--what-this-slice-does-not-do-and-the-next)
- **Stage 7, optional sync** — encrypted transport, identity, conflicts — only if users ask for
  it; local operation stays complete offline. [09](09-agent-events-and-memory.md#product-stages-and-exit-gates)

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
request's number and its checks go. Workspace and toolbar hover previews now include branches,
review status, line counts and individual checks; the toolbar offers confirmed squash, merge-commit
and rebase merges. Open harness tabs have their own count and preview. Reused branches prefer an
open PR, and refresh generations prevent older responses from replacing newer data. What Superset
still has and this does not is the review itself: comments and files reviewed.
