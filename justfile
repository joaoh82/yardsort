# Yardsort task runner. `just` lists everything; recipes run from the repo root.

set windows-shell := ["pwsh", "-NoLogo", "-Command"]

# List available recipes
default:
    @just --list --unsorted

# --- run ------------------------------------------------------------------------------------

# Install dependencies (JS packages + Rust crates)
setup:
    bun install
    cargo fetch

# Run the app with hot reload
dev:
    bun tauri dev

# Run the app forcing a terminal renderer: webgl or dom
dev-renderer $YARDSORT_RENDERER:
    bun tauri dev

# Run the frontend alone in a browser tab (no Rust core; terminals won't work)
web:
    bun run dev

# The `ys` command-line client, against whatever profile the arguments name
cli *args:
    cargo run -q -p yardsort-cli -- {{args}}

# --- check ----------------------------------------------------------------------------------

# Formatting, lints, types and all tests. (CI additionally runs `bindings-check`.)
check: fmt-check lint typecheck test

# All tests, Rust and frontend
test: test-rust test-web

# Rust tests (also regenerates src/lib/bindings.ts)
test-rust *args:
    cargo test --workspace {{ args }}

# Frontend tests
test-web *args:
    bun run test {{ args }}

# Frontend tests in watch mode
test-watch:
    bun run test:watch

# Lint Rust (clippy, warnings are errors) and the frontend (eslint)
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    bun run lint

# Clippy the PTY crates for Windows from any OS (`yardsort-core` and the app bundle SQLite, which
# needs the MSVC toolchain to compile; CI covers those on real Windows)
lint-windows:
    bun run lint:windows

# Typecheck the frontend
typecheck:
    bun run typecheck

# Format everything
fmt:
    cargo fmt --all
    bun run format

# Check formatting without changing files
fmt-check:
    cargo fmt --all -- --check
    bun run format:check

# Regenerate TypeScript bindings from the Rust commands
bindings:
    bun run bindings

# Fail if regenerated bindings differ from what is staged/committed — run before pushing
bindings-check: bindings
    git diff --exit-code -- src/lib/bindings.ts

# --- build ----------------------------------------------------------------------------------

# Build installers for this OS (target/release/bundle/)
build:
    bun tauri build

# Build only the given bundle types, e.g. `just bundle deb` or `just bundle appimage,deb`
bundle types:
    bun tauri build --bundles {{ types }}

# Debug build of the app, no installers
build-debug:
    bun tauri build --debug --no-bundle

# Cut a release: bump the version, commit, tag and push; CI builds and publishes it (docs/releasing.md)
[unix]
release version:
    scripts/release.sh {{ version }}

# --- measure --------------------------------------------------------------------------------

# Terminal rendering benchmarks (see docs/design/07-terminal-benchmarks.md)
[unix]
bench:
    scripts/bench/run.sh

# --- ci & housekeeping ----------------------------------------------------------------------

# Latest CI runs
ci:
    gh run list --limit 5

# Follow the most recent CI run until it finishes
ci-watch:
    gh run watch "$(gh run list --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status

# Download installers from the latest successful CI run on main into ./artifacts
ci-artifacts:
    gh run download "$(gh run list --branch main --status success --limit 1 --json databaseId -q '.[0].databaseId')" --dir artifacts

# Run the website (website/) with hot reload
site-dev:
    cd website && bun install && bun run dev

# Build the website as static files in website/out
site-build:
    cd website && bun install && bun run build

# Lint, typecheck and build the website, as CI does
site-check:
    cd website && bun install --frozen-lockfile && bun run lint && bun run typecheck && bun run build

# Remove build output (Rust target, frontend dist, downloaded artifacts)
[unix]
clean:
    cargo clean
    rm -rf dist artifacts

# Remove build output (Rust target, frontend dist, downloaded artifacts)
[windows]
clean:
    cargo clean
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue dist, artifacts
