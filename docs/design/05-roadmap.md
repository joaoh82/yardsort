# 05 — Roadmap

Ordered by risk first, then by the shortest path to something usable daily. Every milestone must
pass on **Linux, macOS and Windows** before it is done — CI enforces the build, a short manual
checklist covers what CI can't see.

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
confirmation. Rename, archive and "delete the branch too" remain in M6. Harness sessions are
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

_Notes:_ the diff viewer is CodeMirror 6's unified merge view (settling open question 9), loaded
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
- Worktree reconciliation: unknown worktrees are adopted (M3); a workspace whose folder vanished is
  flagged and can be restored from its branch or deleted.

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
- [ ] Hands-on pass and benchmark rows on macOS and Windows hardware.
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
- [ ] **First AUR publish — blocked upstream.** The AUR paused new account registration on
      2026-09-19, and a maintainer account is needed to register the deploy key. Everything else
      is in place (the `AUR_SSH_PRIVATE_KEY` secret is set); once an account exists, register the
      public key and re-run the _Publish to the AUR_ job of the latest release.
- [x] Homebrew: cask in the `joaoh82/homebrew-yardsort` tap, install-tested on macOS (Gatekeeper:
      _Notarized Developer ID_) by the release workflow before each push.
- [ ] winget: `joaoh82.Yardsort` submitted (microsoft/winget-pkgs#437567); awaiting Microsoft's
      review. Updates are submitted by the release workflow once `WINGET_TOKEN` is set.
- [ ] Flatpak, Scoop, Chocolatey — on request.
- [ ] Windows code signing, if funding appears.
- [x] First-run check: the welcome screen reports whether git and at least one agent were found,
      with install commands and a "Check again" that re-reads the environment without a restart.
- [ ] Project website. Built in `website/` from a Claude Design landing page: a static Next.js
      app that renders `docs/` (Markdown or MDX) and `CHANGELOG.md` directly, checked in CI.
      Left: deploy it and point `yardsort.sh` at it, then link it from the README.

_Exit:_ v0.1.0 public release.

## Later (unordered)

- Commit / push / open PR from the UI; show PR + CI status on the workspace row.
- Per-project setup script and "files to copy into new worktrees" (`.env` etc.); run/dev-server button.
- `yardsortd`: move the PTY host out of process so agents survive closing the window
  (boundary already in place from M1; see open questions for lifecycle).
- Merge / rebase helpers; "apply this workspace onto local".
- Diff comments sent back to the agent as a prompt.
- Multi-repo projects; remote/SSH workspaces.
- Usage / cost view per workspace. MCP config management per harness.
- Command palette; themes; Omarchy theme integration.
