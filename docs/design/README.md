# Yardsort — design docs

Initial planning, written 2026-09-17. These are living documents: update them as decisions get made,
and move anything settled out of [open questions](06-open-questions.md) into the relevant doc.

| #   | Doc                                                                     | What it covers                                                                                                          |
| --- | ----------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| 01  | [Vision & scope](01-vision.md)                                          | What we're building, for whom, what is in and out of v1                                                                 |
| 02  | [UX & flows](02-ux.md)                                                  | Three-panel layout, project tree, new-workspace flow, settings                                                          |
| 03  | [Architecture](03-architecture.md)                                      | Tauri structure, PTY, git, storage, cross-platform concerns                                                             |
| 04  | [Harnesses](04-harnesses.md)                                            | Harness config model, arg templating, verified defaults per CLI                                                         |
| 05  | [Roadmap](05-roadmap.md)                                                | Milestones in build order, with exit criteria                                                                           |
| 06  | [Open questions](06-open-questions.md)                                  | Decisions still to make                                                                                                 |
| 07  | [Terminal benchmarks](07-terminal-benchmarks.md)                        | M1 go/no-go on webview terminal rendering, with numbers                                                                 |
| 08  | [Manual checklist](08-manual-checklist.md)                              | The per-OS pass CI cannot do, and the record of having done it                                                          |
| 09  | [Agent events & memory](09-agent-events-and-memory.md)                  | Implemented through stage 6: a local event layer, handoffs, reviewed memory, outcomes                                   |
| 10  | [Agent events, stage 0–1](10-agent-events-stage-1.md)                   | The fit report against the live tree, the decisions, and what shipped                                                   |
| 11  | [Agent events, stage 2: Claude Code](11-agent-events-stage-2-claude.md) | Recorded hook fixtures, the per-launch settings file, the inbox, coverage                                               |
| 12  | [Agent events, stage 2: Codex](12-agent-events-stage-2-codex.md)        | Why not its hooks; `notify` as the trigger, the session file as the substance                                           |
| 13  | [Agent events, stage 2: OpenCode](13-agent-events-stage-2-opencode.md)  | A plugin given per launch through the environment; the last of the three named                                          |
| 14  | [Agent events, stage 2: Grok](14-agent-events-stage-2-grok.md)          | Its own metadata-only session log, read where Yardsort knows to look; nothing installed                                 |
| 15  | [Agent events, stage 2: OMP and Pi](15-agent-events-stage-2-pi-omp.md)  | One extension for the two forks, given on the command line per launch                                                   |
| 16  | [Agent events, stage 2: Cursor](16-agent-events-stage-2-cursor.md)      | Hooks through a per-launch plugin directory; recorded, and what the recording changed                                   |
| 17  | [Agent events, stage 3: review](17-agent-events-stage-3-review.md)      | Reported writes joined to git's change list at read time; no line is ever claimed                                       |
| 18  | [Agent events, stage 4: handoffs](18-agent-events-stage-4-handoff.md)   | The next agent's first message, written from the record and edited in the composer                                      |
| 19  | [Agent events, stage 5: memory](19-agent-events-stage-5-memory.md)      | Lessons for a project's agents: you write, agents propose, only you approve                                             |
| 20  | [Agent events, stage 6: outcomes](20-agent-events-stage-6-outcomes.md)  | What became of each attempt: your label, backed by merges; history that waits for a sample                              |
| 21  | [Workflows](21-workflows.md)                                            | Proposal: named agent sequences in YAML, a DAG of steps, run from the app or `ys`; manual only                          |
| 22  | [Pull requests](22-pull-requests.md)                                    | Every project's pull requests in one view. All four slices built: list, actions, Summary, Code, writing                 |
| 23  | [Tasks](23-tasks.md)                                                    | Every project's GitHub issues in one view, and `ys task`. All three slices built                                        |
| 24  | [Repositories](24-repositories.md)                                      | Pick a repository from the forge, clone from any host, start from an issue or pull request link. Built                  |
| 25  | [Remote, mobile & sync](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md) | Proposal: reach your agents from a phone or another computer; self-hostable; durable sync after that                    |
| 26  | [Remote, R0–R1](26-remote-r0-r1.md)                                     | The fit report, the versioned protocol and a read-only host facade. Not wired; no terminals yet                         |
| 27  | [Remote, R2](27-remote-r2.md)                                           | Identity and relay. Done: sign-in, pairing, the relay, staging                                                          |
| 28  | [Remote infrastructure](28-remote-infrastructure.md)                    | Local, staging and production; Fly.io, ZITADEL Cloud, PostgreSQL. Staging live                                          |
| 29  | [Conversation context](29-conversation-context.md)                      | Proposal: every agent in a workspace can read the others' conversations, as a digest from their transcripts; no model   |
| 30  | [Open source and the managed service](30-open-source-and-managed.md)    | The server is AGPL, the phone app MPL, the rest GPL; production runs the public server unmodified; what stays private   |
| 31  | [Remote, R3](31-remote-r3.md)                                           | The phone app. Planned: a TypeScript client, an Expo app, seven slices and a device run                                 |
| 32  | [Smoke tests](32-smoke-tests.md)                                        | Proposal, and slice 1 built: a test file in the repository an agent writes and Yardsort checks, approved before it runs |

## Vocabulary

| Term          | Meaning                                                                                                        |
| ------------- | -------------------------------------------------------------------------------------------------------------- |
| **Project**   | A git repository on disk that Yardsort knows about.                                                            |
| **Workspace** | One unit of parallel work inside a project: a git worktree + branch + harness session(s).                      |
| **Local**     | The always-present pseudo-workspace that points at the project's own checkout (the repo root), not a worktree. |
| **Harness**   | A terminal-based coding agent CLI (Claude Code, Codex, Grok, OpenCode, …).                                     |
| **Session**   | One run of a harness inside a workspace. Has a harness-side session id used for resume/fork.                   |
