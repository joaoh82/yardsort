# 01 — Vision & scope

## Problem

Tools like Superset and Conductor make it easy to run several AI coding agents in parallel, each
isolated in its own git worktree. They are effectively macOS-only. Anyone on Linux (this project was
born on Omarchy / Arch + Hyprland) or Windows has no equivalent.

## Product in one sentence

A desktop app where you pick a project, spin up a workspace, choose a harness + model + effort, type
the first message — and get an isolated worktree with that agent running in a real terminal, next to
a live view of the files and diff it is producing.

## Principles

1. **Cross-platform is a feature, not a port.** Linux, macOS and Windows ship together. CI builds and
   smoke-tests all three from the first milestone. No platform gets "later".
2. **The terminal is the truth.** We do not re-implement agent UIs or parse their output. The center
   panel is a real PTY running the real CLI. Whatever the harness can do in a terminal, it can do here.
3. **Harness-agnostic.** A harness is just configuration: a command and some argument templates.
   Adding a new agent must never require a code change.
4. **Git-native, no lock-in.** Workspaces are plain git worktrees and plain branches. Everything
   Yardsort does can be inspected and undone with ordinary `git` commands.
5. **Fast and light.** Tauri over Electron; idle cost should be close to a terminal emulator's.

## v1 scope

In:

- Add a project by **opening** an existing folder or **creating** a new one (name + parent path).
- Two-level sidebar: project → `local` + workspaces, with `+` to create a workspace.
- New-workspace composer: harness, model, effort, initial message.
- Worktree + branch creation on first message; harness launched in a PTY in that worktree.
- Right panel: file tree, changed files, diff viewer.
- Settings → Harnesses: label, command, prompt args, resume args, fork args, prompt transport
  (argv / stdin), restore defaults.
- Default harnesses: **Claude Code, Codex, Grok, OpenCode**.
- Resume a workspace's session after app restart. Archive / delete a workspace.

Out (for now, see [roadmap](05-roadmap.md) "Later"). Keeping agents alive while the app is
closed was on this list and has since shipped — see M9:

- PR creation / GitHub integration, review comments
- Per-project setup/run scripts
- Remote / SSH / cloud workspaces
- Our own chat UI over agent protocols (ACP etc.)
- Team features, accounts, telemetry

## Non-goals

- Replacing your editor. The right panel is for _reviewing_, with an "open in editor" escape hatch.
- Being a terminal emulator product. The terminal exists to host harnesses (and a utility shell).
