# Contributing to Yardsort

Thank you for being here. Bug reports, ideas, documentation fixes and code are all welcome, and
small contributions are as appreciated as large ones.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Contributions are
licensed under the project's [GPL-3.0 license](LICENSE).

## Ways to help

- **Report a bug** or **suggest a feature** — [open an issue](https://github.com/joaoh82/yardsort/issues/new/choose).
- **Try it on your system.** Yardsort targets Linux, macOS and Windows; reports from real
  machines — especially Windows, and Linux desktops other than the author's — are gold.
- **Add or correct a harness.** Agent CLIs change their flags often. If a built-in definition is
  out of date, a one-line fix in `src-tauri/src/harness.rs` helps everyone.
- **Improve the docs.** If something confused you, it will confuse the next person.

Not sure whether an idea fits? Open an issue first — it is cheaper than a pull request nobody
expected. The [vision](docs/design/01-vision.md) and [roadmap](docs/design/05-roadmap.md) say
where the project is heading and what it deliberately is not.

## Getting set up

You need [Rust](https://rustup.rs) (stable), [Bun](https://bun.sh), git,
[`just`](https://just.systems) (`cargo install just`) and Tauri's
[system dependencies](https://tauri.app/start/prerequisites/) for your OS.

```sh
git clone https://github.com/joaoh82/yardsort && cd yardsort
just setup
just dev
```

Useful while developing:

| Command                              | Does                                                                              |
| ------------------------------------ | --------------------------------------------------------------------------------- |
| `just dev`                           | Run the app with hot reload.                                                      |
| `just check`                         | Everything CI checks except the bindings: formatting, lints, types, all tests.    |
| `just test-rust` / `just test-web`   | One side's tests; both take extra arguments.                                      |
| `just fmt`                           | Format Rust and TypeScript.                                                       |
| `just bindings-check`                | Fail if the generated TypeScript bindings are stale. Run before pushing.          |
| `just lint-windows`                  | Clippy the PTY crates for Windows from any OS.                                    |
| `YARDSORT_DATA_DIR=/tmp/ys just dev` | Run against a throwaway database and settings file, leaving your real ones alone. |

## How the code is laid out

```
crates/pty-host/     Owns pseudo-terminals and the processes in them. No Tauri, no UI.
                     Tested against real processes in real PTYs.
crates/pty-ipc/      The wire between the app and the daemon that runs that host: framing,
                     the server loop, and a client that is itself a TerminalHost.
src-tauri/           The Tauri app (Rust core): projects, workspaces, git, harnesses,
  src/               sessions, changes, settings, the SQLite store, and thin IPC commands.
                     The same binary is the daemon, run with --yardsort-daemon.
  migrations/        Numbered SQL files. Never edit a shipped one — add a new one.
src/                 The React + TypeScript frontend.
  features/          One folder per area of the UI.
  stores/            Zustand stores.
  lib/ipc.ts         The only module that talks to the core.
  lib/bindings.ts    GENERATED from the Rust commands by tauri-specta. Never edit by hand.
docs/                User documentation; docs/design/ holds architecture and planning.
website/             The project website: its own Next.js app. It renders docs/ and
                     CHANGELOG.md, so it has no copy of them. See website/README.md.
```

Two principles explain most design decisions — more in [architecture](docs/design/03-architecture.md):

1. **The frontend holds no truth.** State lives in the Rust core; the UI renders it and sends
   intents. Paths, for instance, never come from the webview.
2. **The terminal is the truth.** Agents run in a real PTY and their output is never parsed.
   Status, readiness and notifications all derive from _activity_, not content.
3. **The app is a client.** The terminals belong to the daemon, not the window, so closing the
   window is a disconnect — never a kill.

## Making a change

1. Branch from `main`.
2. Make the change, **with tests**. The bar: a test should fail without your change. Rust logic
   is tested against real git repositories and real processes, not mocks; UI is tested with
   Testing Library through what the user sees and does.
3. **Update the docs.** If behaviour a user can see changed, the [user guide](docs/README.md)
   changes in the same pull request. Docs that lag the app are treated as bugs.
4. Changed a Rust command or a type that crosses IPC? Run `cargo test` (or `just dev`) to
   regenerate `src/lib/bindings.ts`, and commit it.
5. `just check && just bindings-check`.
6. Open a pull request describing **what** changed and **why**, and how you tested it.
   Screenshots help for anything visual.

CI runs the checks on Linux, macOS and Windows. Platform-specific failures are common in this
codebase — that is what the matrix is for — so do not be discouraged by a red Windows run; the
logs usually say exactly what differed.

### Style

- Rust: `rustfmt` and `clippy` with warnings as errors. TypeScript: Prettier and ESLint.
- Comments explain **why**, not what. Match the density of the code around you.
- Error messages are for users: say what happened and what they can do about it.
- Anything that can destroy user work — deleting, archiving, overwriting — needs an explicit
  confirmation that names what will be lost, and a test that proves a "no" is respected.
- App shortcuts go behind **Mod** (`⌘`, or `Ctrl+Shift`). Plain `Ctrl`+letter belongs to the
  program in the terminal — see [shortcuts](docs/guide/shortcuts.md).

### Commit messages

A short imperative summary line, then — if it helps — a body explaining why. No particular
convention is enforced.

## Releases

Maintainers cut releases by pushing a version tag; see [docs/releasing.md](docs/releasing.md).
