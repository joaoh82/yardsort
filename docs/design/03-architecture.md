# 03 — Architecture

## Overview

```
┌──────────────────────────── Tauri app ─────────────────────────────┐
│  Webview (React + TS)                                              │
│   sidebar · composer · xterm.js terminals · file tree · diff view  │
│        │  invoke(commands)            ▲  events / channels         │
│ ───────┼──────────────────────────────┼─────────────────────────── │
│        ▼                              │                            │
│  Rust core                                                         │
│   ├─ projects     registry, open/create                            │
│   ├─ workspaces   lifecycle, naming, worktree paths                │
│   ├─ git          GitBackend trait → git CLI                       │
│   ├─ harness      definitions, arg templating, launch plans        │
│   ├─ daemon       finds/starts yardsortd; TerminalHost client      │
│   ├─ watch        notify-based fs watcher, debounced               │
│   ├─ env          login-shell environment resolution               │
│   ├─ assist       optional Jev judgments: diffs, composer hints    │
│   └─ store        SQLite (state) + settings file                   │
└──────────────────────────────┬──────────────────┬──────────────────┘
                               │ local socket     │ shells out
                               ▼                  ▼
        ┌─── yardsortd (the same binary, --yardsort-daemon) ───┐   git
        │  PTY host: sessions, I/O pump, headless VT, snapshots│
        └──────────────────────────┬───────────────────────────┘
                                   │ spawns
                                   ▼
                          claude / codex / grok / …
```

The daemon is what makes closing the window harmless: it owns the pseudo-terminals, so the app
is a client that can come and go.

Rule of thumb: **the frontend holds no truth.** All state lives in the Rust core and the store; the
UI renders it and sends intents. This keeps the door open for a headless core later (see
"session persistence" below).

## Stack choices

| Concern      | Choice                                                                                                                          | Why                                                                                                                                             |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Shell        | **Tauri 2**                                                                                                                     | Small, Rust core, all three OSes, good updater/bundler story.                                                                                   |
| Frontend     | **React + TypeScript + Vite**, Tailwind, Zustand                                                                                | Boring and well-trodden; biggest component ecosystem for trees, panels, diff views.                                                             |
| JS tooling   | **bun**                                                                                                                         | Already installed; fast. Plain `package.json`, so npm/pnpm still work for contributors.                                                         |
| Terminal     | **xterm.js** + fit, webgl (with DOM-renderer fallback; the canvas addon was dropped in xterm.js 6), web-links, unicode11 addons | The standard; what VS Code uses.                                                                                                                |
| PTY          | **`portable-pty`** (wezterm)                                                                                                    | One API over Unix PTYs and Windows ConPTY.                                                                                                      |
| App ↔ daemon | **`interprocess`** local sockets                                                                                                | One blocking API over Unix domain sockets and Windows named pipes; no async runtime needed for a handful of connections.                        |
| Git          | **`git` CLI** behind a `GitBackend` trait                                                                                       | Worktree support in libgit2/gitoxide is partial; the CLI is the reference implementation and respects the user's config, hooks and credentials. |
| State        | **SQLite** via `rusqlite` (bundled)                                                                                             | Projects/workspaces/sessions are relational; bundled build avoids system-lib differences.                                                       |
| Settings     | TOML file in the OS config dir                                                                                                  | Human-editable, easy to back up and diff. Harness definitions live here.                                                                        |
| Paths        | `directories` crate                                                                                                             | Correct config/data dirs per OS.                                                                                                                |
| FS watching  | `notify` + debouncer                                                                                                            | Cross-platform; honour `.gitignore` via the `ignore` crate.                                                                                     |
| Diff view    | CodeMirror 6 merge view _(tentative)_                                                                                           | Much lighter than Monaco. See open questions.                                                                                                   |
| Assist       | TypeSafe **Jev** over HTTP (`reqwest`, the updater's TLS stack); key in the OS credential store (`keyring`)                     | A judgment model answers typed questions, so there is no generated text to parse. The key must never reach `settings.toml`.                     |

## PTY & terminal data path

How it works: the Rust core opens a pseudo-terminal and spawns the harness attached to it, so the
harness believes it is in an ordinary terminal. xterm.js in the webview is the terminal _emulator_:
it parses the escape sequences, keeps the screen grid and draws it. This is the same split VS Code
and every Electron terminal use (xterm.js + `node-pty`); we swap `node-pty` for `portable-pty` and
Chromium for the system webview.

- One session per terminal tab, serviced by three threads: a **reader** blocking on the PTY, a
  **waiter** blocking on the child, and a **pump** that batches output, feeds the **headless
  terminal state** (see below) and fans out to viewers.
- **Output**: reader thread → Tauri `Channel` carrying raw bytes → `xterm.write()`. Use channels,
  not global events: they are ordered, per-session and avoid JSON-encoding the stream. Coalesce
  reads into ~16 ms batches so a TUI that repaints constantly doesn't flood IPC.
- **Input**: `xterm.onData` → `write(session_id, bytes)`. IPC calls are dispatched to a thread
  pool, so back-to-back calls can run out of order; the frontend therefore sends one write at a
  time and coalesces whatever is typed meanwhile into the next (`writer.ts`).
- **Resize**: fit addon → `resize(cols, rows)`, debounced.
- **Activity signal**: the pump timestamps the last output; the sidebar status dots derive from
  that plus process liveness. We never parse harness output for meaning.
- **Exit**: capture exit code, keep scrollback, surface Resume/Fork actions. `Exited` is announced
  only after all output has been delivered. Where end-of-file never arrives (ConPTY; a background
  grandchild holding the terminal) the pump stops once the PTY has been quiet for 150 ms.

### Rendering & performance expectations

Agent workloads are low-throughput (KB/s); the cost is full-screen repaints while a harness streams.

| Webview            | Expectation                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------------- |
| WebView2 (Windows) | Chromium — on par with Electron.                                                                  |
| WKWebView (macOS)  | Fast JS and WebGL — no concern.                                                                   |
| WebKitGTK (Linux)  | **The risk.** WebGL is less reliable; NVIDIA + Wayland has known DMABUF blank/slow-window issues. |

Renderer policy: try `@xterm/addon-webgl`; on context loss or init failure fall back to xterm's
built-in DOM renderer (the canvas addon is not available for xterm.js 6). Expose a setting to force
either. Linux mitigations to evaluate in M1: `WEBKIT_DISABLE_DMABUF_RENDERER=1`, preferring the
integrated GPU on hybrid laptops. The planning machine (RTX 3070 Ti + Radeon 680M, WebKitGTK 2.52,
Hyprland) is exactly the hard case, so M1 is a meaningful test.

### The PTY host boundary (daemon-ready)

Reference apps in this space run a **terminal daemon**: a background process owns the PTYs and the
window is just a client, tmux-style. That is not about rendering speed — it is what lets agents keep
working when the window closes, crashes or updates. The `pty` module was written as a **PTY host**
with a message-shaped API from day one, so that the move could be made without anything above it
changing:

```
spawn(LaunchPlan) -> SessionId          list() -> [SessionInfo]
attach(SessionId) -> Snapshot + stream  detach(SessionId)
write(SessionId, bytes)                 resize(SessionId, cols, rows)
kill(SessionId, signal)                 events: output, exit, activity
```

- **v1:** the host ran in-process; "transport" was a function call plus a Tauri channel.
- **Since M9:** the same host runs in a separate process, the transport is a local socket, and the
  app is one client of it. Nothing above the boundary changed — `terminal.rs` and `sessions.rs`
  hold an `Arc<dyn TerminalHost>` and cannot tell which they have.
- Rules that made this cheap, and still hold: the host depends on nothing from Tauri or the UI;
  every request and event is a serialisable type; sessions are addressed by id, never by handle;
  the host — not the frontend — is the owner of scrollback and terminal state.

**Headless terminal state.** To restore a screen on (re)attach, raw byte replay is not enough for
alt-screen TUIs. The host feeds output through a headless VT parser in Rust (`vt100`, 10 000 lines
of history) and can emit a **snapshot** — a byte sequence that repaints the
current screen — followed by the live stream. This pays off immediately in v1 (clean switching
between workspaces without keeping every xterm instance mounted) and is mandatory for the daemon.

**The host is the terminal when nobody is watching.** Programs can query the terminal and wait for
an answer — above all "where is the cursor?" (`ESC [ 6 n`). With a viewer attached, xterm.js
replies. With none, the host replies from its headless terminal; the decision is made under the
same lock that delivers output, so exactly one reply is ever sent. This is load-bearing on Windows:
`portable-pty` creates the pseudo-console with `PSEUDOCONSOLE_INHERIT_CURSOR`, which makes ConPTY
ask that question at startup and run nothing until it is answered.

- **Re-attach**: switching workspaces detaches the view; the session keeps running in the host.
  On re-mount: `attach` → write snapshot → stream live.
- **App quit**: merely a disconnect. The conversation is still running when the app comes back,
  and its record is left `running` rather than settled as _interrupted_. Resume args are still
  what brings back a conversation whose **process** is gone — see [04-harnesses](04-harnesses.md).

### The daemon (`yardsortd`)

The daemon is **the app's own executable, re-run with `--yardsort-daemon <socket>`** — the same
trick `--yardsort-print-env` uses. No second binary means nothing extra to bundle, sign or
notarize on three platforms, and the daemon can never be a different build from the app that
started it. The flag is handled in `main` before Tauri is touched, so a daemon never creates a
window or a webview.

| Concern         | Decision                                                                                                                                                                                                                                                                                                                                                                                                            |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Transport       | `interprocess` local sockets: a **socket file** on Unix, a **named pipe** on Windows.                                                                                                                                                                                                                                                                                                                               |
| Who may connect | The socket file is `0600` — Linux checks write permission on it, so that is the lock; its directory is tightened to `0700` too where we own it, which the last fallback (`/tmp`) is not. A named pipe's default descriptor is already limited to the user's session. Linux abstract sockets are deliberately _not_ used: they have no filesystem permissions, and anything that can connect can type into an agent. |
| How many        | **One per data directory**, addressed by an FNV-1a tag of its path. `YARDSORT_DATA_DIR` therefore isolates a throwaway profile's agents as well as its database.                                                                                                                                                                                                                                                    |
| Who starts it   | Whichever app finds nobody listening, under a lock file so two starting at once produce one daemon. A socket left by a killed daemon is cleared first.                                                                                                                                                                                                                                                              |
| Who stops it    | `Shutdown` from the app (the quit dialog's "Stop them"), or itself once no client is connected **and** no session is running, after a 10 s grace so a restarting app does not take it down.                                                                                                                                                                                                                         |
| Upgrades        | A protocol number, separate from the app version. Same number → just talk. Different, and the daemon is idle → replace it silently. Different, with agents running → leave it alone, run terminals in-process, and tell the user. Killing work nobody agreed to lose is never the answer.                                                                                                                           |
| Wire format     | `[u32 len][u8 kind][payload]`. Requests, responses and events are JSON, reusing the host types' `serde` impls; **output has a frame kind of its own and stays raw bytes**, so a repainting TUI never pays for JSON.                                                                                                                                                                                                 |
| Back-pressure   | Each connection has a writer thread and a queue; a client that stops reading is dropped at 32 MiB rather than allowed to block the session's pump thread. Dropping it is safe — reattaching starts from a snapshot.                                                                                                                                                                                                 |
| Logs            | The app hands the child `daemon.log` in the data directory, replaced on each start. Started by hand, it keeps its terminal.                                                                                                                                                                                                                                                                                         |

`attach` needed one adjustment to cross a socket. In-process, the snapshot is handed to the sink
_before_ `attach` returns, under the lock that delivers output, so no byte is lost or doubled.
Over a socket the response would race those bytes — so the **client** picks the stream id and
registers its sink before sending the request. The daemon's own attach stays atomic and tags that
session's output with the id the client chose.

Prompt delivery over `stdin` moved into the host for the same reason: it waits for the program to
settle, which can take seconds, and it must still land if the window closes meanwhile. It rides
along in the `LaunchPlan`.

## Environment resolution (important)

GUI apps do not inherit the user's interactive shell environment. On macOS, and on Linux when
launched from a desktop launcher, `PATH` will be missing mise/asdf/nvm/cargo/homebrew entries — so
`claude` or `codex` simply won't be found even though they work in the user's terminal. (On the
machine this was planned on, every harness is installed via mise.)

- **Unix**: at startup run the user's login shell once — `$SHELL -i -l -c '<yardsort>
--yardsort-print-env'` — with a timeout. Asking our own binary to dump the environment (between
  markers, NUL-separated) avoids depending on `env -0` or any shell's syntax, and ignores whatever
  the startup files print. The result is cached and is the _whole_ environment of every spawn;
  `env_info(reload: true)` re-runs it.
- **Windows**: use the process environment; re-read user/system `PATH` from the registry on reload.
- Resolve the harness `command` against that `PATH` ourselves so "not found" becomes a clear,
  actionable error in the UI (with the PATH we searched), not a silent dead terminal.
- Set `TERM=xterm-256color`, `COLORTERM=truecolor`, and `YARDSORT_WORKSPACE`, `YARDSORT_PROJECT`
  for scripts and hooks.

Harnesses are spawned **directly with an argv array — never through a shell string.** This removes
a whole class of quoting bugs, especially for prompts and especially on Windows.

## Git & worktrees

Operations needed for v1, all via the CLI with `--porcelain` / `-z` output where available:

| Need                | Command                                                                                             |
| ------------------- | --------------------------------------------------------------------------------------------------- |
| Is repo / find root | `git rev-parse --show-toplevel`                                                                     |
| Default branch      | `git symbolic-ref --short refs/remotes/origin/HEAD`, else current branch, else `init.defaultBranch` |
| Create workspace    | `git worktree add -b <branch> <path> <base>`                                                        |
| List / reconcile    | `git worktree list --porcelain`                                                                     |
| Remove workspace    | `git worktree remove [--force] <path>` (+ optional `git branch -D`)                                 |
| Changes             | `git status --porcelain=v2 -z`, `git diff --name-status -z <merge-base>`                            |
| Diff content        | `git diff <merge-base> -- <file>`, `git show <rev>:<file>`                                          |

Decisions:

- **Worktree location**: outside the repo, under a Yardsort-owned root —
  `<data-dir>/worktrees/<project-slug>/<workspace-slug>`; configurable. Keeping it outside avoids
  polluting the repo and confusing tools that walk the tree. Keep the path **short** — Windows'
  260-char limit bites deep `node_modules` trees (also recommend `core.longpaths=true` there).
- **Branch naming**: `<prefix>/<workspace-slug>`, prefix default `ys`, configurable. Local
  extraction prefers an explicit task clause anywhere in the first message, skipping fenced code
  and recognized request lead-ins. It falls back to the opening prose, removes filler words and
  caps the slug at four words / 32 ASCII characters. This is a heuristic, not a model summary;
  existing workspaces are not renamed automatically.
- **Reconciliation**: on startup and on focus, compare the DB with `git worktree list`. Worktrees
  git has that the DB does not are adopted only when they sit under Yardsort's worktree root —
  ours, lost when a project was removed and re-added; any other worktree is the user's business
  until they import it, and a forgotten one is hidden (its row and sessions kept) rather than
  deleted, so importing it again finds its history. Worktrees
  deleted behind our back are marked _missing_, not silently dropped. A worktree whose _branch_
  was deleted too has nothing to restore from: it is marked _gone_, and Yardsort asks once whether
  to forget the workspace. Only the vanished case asks git about branches, so the usual listing
  costs no extra process.
- **Deleting** a workspace with uncommitted or unmerged work requires explicit confirmation that
  names what will be lost.
- Untracked-but-needed files (`.env`, etc.) don't exist in a fresh worktree. v1: document it.
  Later: per-project "copy these files" list / setup script.

## Data model (SQLite)

`yardsort.db` in the OS app-data directory (override with `YARDSORT_DATA_DIR`). Migrations are
numbered SQL files in `crates/core/migrations/`, applied in order and tracked with `user_version`; a
database written by a newer build is refused rather than touched.

```
projects    id, name, root_path (unique), sort_order, created_at
workspaces  id, project_id → projects (cascade), kind ('local' | 'worktree'), name, path,
            branch, base_branch, status ('active' | 'archived'), sort_order, created_at
ui_state    key, value (JSON)   -- selection, collapsed projects, last-used picks
sessions    id, workspace_id → workspaces (cascade), harness_id, model, effort,
            harness_session_id, title, forked_from, state ('running' | 'ended'), exit_code,
            pty_session_id, started_at, ended_at
```

`local` is a real row (`kind = 'local'`, `path = root_path`, exactly one per project) so the rest of
the code never special-cases it. What is _checked out_ in a workspace is never stored: it is asked
of git whenever projects are listed, because branches change behind our back. A project whose
folder has vanished is flagged `missing`, not dropped — it comes back by itself if the folder does.
Removing a project only forgets it; nothing on disk is touched. Harness definitions are _not_ in
the DB; they live in the settings file.

**Session records** are what let a conversation outlive its process. One row per _harness
conversation_ (shells have nothing to come back to): which harness, and the harness's own id for
the conversation when we chose it. The PTY session carries a `record` label pointing at its row.
When the process exits the core settles the row _before_ announcing the exit; rows still `running`
at startup died with the previous run and are ended without an exit code ("interrupted").
**Resume** reuses the row with `resume_args`; **Fork** makes a new row (`forked_from`) and passes
`{new_session_id}` so the copy is resumable too. An archived workspace keeps its rows — and its
path — so restoring it puts the folder back where the harness filed its conversations.

**Activity.** The PTY host reports `Busy` when output starts and `Quiet { busyMs }` once nothing
has been printed for 3 s. Agents animate a spinner while they think, so for them silence means
"waiting for you". The UI turns this into status dots, marks work that finished unwatched, and
sends a desktop notification for bursts of 8 s or more — never for shells, and never while you
are looking at the terminal. Nothing is ever parsed out of the output.

Live sessions themselves are not in the database — the PTY host owns them. Each carries a `workspace`
label, which is how terminal tabs find their workspace after a webview reload.

## Assist (optional, off by default)

Where ordinary code cannot tell what a change _means_, Assist asks TypeSafe's **Jev** — a model
that answers typed questions (a yes/no probability, one option out of a set, a level on a scale)
and never generates text. Two features use it: badges on the change list, and composer
suggestions. See [the guide](../guide/assist.md) for the user's view.

Shape of it, and the reasons:

- **Code owns the workflow.** git produces the diffs, `assist::review` asks one narrow question
  per property, and the thresholds that turn a probability into a badge live in Rust. The model
  decides nothing.
- **It never touches the terminal.** Only git diffs, file paths, the workspace's first messages
  and the composer's text are ever sent. Status, readiness and notifications still come from PTY
  activity alone — see the principle in [01-vision](01-vision.md). Whether Jev may ever read a
  screen to explain _why_ an agent went quiet is open question 16.
- **Consent per feature.** `[assist]` in `settings.toml` holds one switch per feature, both
  default false, and each switch names in the UI what it sends. No key, no requests.
- **The key is not settings.** It lives in the OS credential store (`keyring`), with
  `TYPESAFE_API_KEY` as the fallback for systems with no Secret Service. It crosses IPC inwards
  only: the webview can set or forget it, never read it.
- **Nothing is load-bearing.** Every failure degrades to "no badges": a bad key stops a review,
  anything else leaves the files already judged in place. Answers are cached per file content, so
  watching an agent work re-asks only about what changed.
- **A file whose name says "credentials"** (`.env`, `*.pem`, `id_*`…) is flagged by ordinary code
  and its contents are never sent. That check is a plain function with a plain test.
- One request per file, six at a time, diffs truncated: Jev reads 32k tokens of state, and
  [its documented weak spots](https://docs.typesafe.ai/model-jaggedness/jev-1.13) include large
  states full of irrelevant detail.

The model id is pinned (`jev-1.13.0`) rather than `jev-latest`, because the thresholds were chosen
against a specific version. The wording of the questions carries a version too, so cached answers
from older wording are not reused.

## Cross-platform notes & risks

| Platform    | Watch out for                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Linux**   | WebKitGTK is the weakest webview: xterm.js WebGL can be flaky → auto-fallback to the DOM renderer. Known blank-window issues on NVIDIA/Wayland (`WEBKIT_DISABLE_DMABUF_RENDERER=1`). The AppImage's launcher forces X11; `display.rs` puts it back on Wayland, and `env.rs` keeps the AppImage's variables out of sessions. Test on Hyprland (tiling, fractional scaling). Ship AppImage + deb + rpm, plus an AUR package.                                           |
| **macOS**   | PATH resolution (above). Code signing + notarization needed for a painless install. Universal binary.                                                                                                                                                                                                                                                                                                                                                                |
| **Windows** | ConPTY quirks (resize reflow, exit detection, the startup cursor query above, `ClosePseudoConsole` blocking — so teardown runs off-thread, `portable-pty` 0.9 reporting a successful kill as an error), needs Win10 1809+. WebView2 runtime bootstrapper. Path length. `git` must be installed — detect and guide. Harness CLIs may be `.cmd` shims (npm) which need `cmd /c` to spawn. Some harnesses officially support Windows only via WSL — see open questions. |
| **All**     | Keybindings: `Mod` = Cmd on macOS, Ctrl elsewhere — but Ctrl+C/V/etc. belong to the TUI. Copy/paste in the terminal needs per-OS conventions (Ctrl+Shift+C/V on Linux/Windows).                                                                                                                                                                                                                                                                                      |

The PTY + webview terminal path is the highest-risk piece and the one most likely to differ per OS,
which is why the [roadmap](05-roadmap.md) proves it on all three platforms before anything else.

## Repo layout

```
yardsort/
├─ Cargo.toml                # cargo workspace root (shared target/, lints, release profile)
├─ docs/
├─ src/                      # frontend
│  ├─ features/{shell,sidebar,workspace,changes,…}/
│  ├─ lib/ipc.ts             # the only module that talks to the core
│  ├─ lib/bindings.ts        # generated by tauri-specta — do not edit
│  └─ stores/
├─ src-tauri/                # the Tauri app crate: webview, commands, watcher, Assist
│  ├─ src/commands.rs        # the IPC surface, thin
│  └─ tauri.conf.json
├─ crates/
│  ├─ core/                  # store, git, projects, workspaces, settings, harnesses,
│  │  │                      # launching, the daemon client — no Tauri (M10)
│  │  └─ migrations/
│  ├─ pty-host/              # owns the sessions; no Tauri deps (M1)
│  └─ pty-ipc/               # the wire: framing, the daemon loop, the client (M9)
└─ .github/workflows/        # check + bundle matrix: ubuntu, macos, windows
```

TypeScript IPC types are generated from Rust with `tauri-specta`, so the boundary can't drift; CI
fails when the checked-in bindings are stale.
