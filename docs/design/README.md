# Yardsort — design docs

Initial planning, written 2026-09-17. These are living documents: update them as decisions get made,
and move anything settled out of [open questions](06-open-questions.md) into the relevant doc.

| #   | Doc                                                                     | What it covers                                                            |
| --- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 01  | [Vision & scope](01-vision.md)                                          | What we're building, for whom, what is in and out of v1                   |
| 02  | [UX & flows](02-ux.md)                                                  | Three-panel layout, project tree, new-workspace flow, settings            |
| 03  | [Architecture](03-architecture.md)                                      | Tauri structure, PTY, git, storage, cross-platform concerns               |
| 04  | [Harnesses](04-harnesses.md)                                            | Harness config model, arg templating, verified defaults per CLI           |
| 05  | [Roadmap](05-roadmap.md)                                                | Milestones in build order, with exit criteria                             |
| 06  | [Open questions](06-open-questions.md)                                  | Decisions still to make                                                   |
| 07  | [Terminal benchmarks](07-terminal-benchmarks.md)                        | M1 go/no-go on webview terminal rendering, with numbers                   |
| 08  | [Manual checklist](08-manual-checklist.md)                              | The per-OS pass CI cannot do, and the record of having done it            |
| 09  | [Agent events & memory](09-agent-events-and-memory.md)                  | Proposal: a local event layer beside the PTY, handoffs, reviewed memory   |
| 10  | [Agent events, stage 0–1](10-agent-events-stage-1.md)                   | The fit report against the live tree, the decisions, and what shipped     |
| 11  | [Agent events, stage 2: Claude Code](11-agent-events-stage-2-claude.md) | Recorded hook fixtures, the per-launch settings file, the inbox, coverage |

## Vocabulary

| Term          | Meaning                                                                                                        |
| ------------- | -------------------------------------------------------------------------------------------------------------- |
| **Project**   | A git repository on disk that Yardsort knows about.                                                            |
| **Workspace** | One unit of parallel work inside a project: a git worktree + branch + harness session(s).                      |
| **Local**     | The always-present pseudo-workspace that points at the project's own checkout (the repo root), not a worktree. |
| **Harness**   | A terminal-based coding agent CLI (Claude Code, Codex, Grok, OpenCode, …).                                     |
| **Session**   | One run of a harness inside a workspace. Has a harness-side session id used for resume/fork.                   |
