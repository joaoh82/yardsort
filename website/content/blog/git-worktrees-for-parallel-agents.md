# What git worktrees solve—and what they don’t—for parallel coding agents

Two coding agents can work on the same repository without editing the same checkout. Give each
task a branch in its own git worktree, and each agent gets a separate set of working files.
That makes parallel work easier to inspect. It does not make every task independent.

## A branch needs a working directory

Opening two terminal tabs in the same folder gives you two processes looking at the same files.
If one agent rewrites a module while the other tests it, the test sees the changed file. Merely
creating another branch does not give the second process a different checkout.

Git worktrees let one repository have several checked-out working directories, each with its
own working state. They share repository data, so they are not separate repository copies or
security sandboxes. See the [Git worktree manual](https://git-scm.com/docs/git-worktree).

For example, keep a documentation change in one workspace and a small UI fix in another. You
can inspect each diff without mixing both edits into one working directory. In Yardsort,
[creating a workspace](/docs/guide/workspaces/) sets up the worktree and starts your chosen agent
there. Starting another agent _inside that workspace_ shares its files instead.

## Separate files do not mean separate services

A useful planning checklist is to ask what each task touches outside its checkout:

| Resource                               | A worktree is enough?                | What to decide                                              |
| -------------------------------------- | ------------------------------------ | ----------------------------------------------------------- |
| Tracked source files                   | Separate working copies              | Give each task a clear scope                                |
| Ignored configuration and dependencies | Not copied automatically by Yardsort | Prepare each worktree with project setup                    |
| A development server's port            | No                                   | Assign different ports                                      |
| A shared database                      | No                                   | Use separate development databases or coordinate migrations |
| Provider accounts and usage limits     | No                                   | Budget concurrent runs against the same account             |
| Credentials available to the agent     | No                                   | Use the agent's permission and sandbox controls             |

A worktree is a convenient unit of work, not an isolation boundary for the whole machine.
An agent with permission to access another path can still access it.

## Setup belongs in the workflow

A fresh Yardsort worktree starts with committed files. An untracked `.env` file or an installed
`node_modules` directory in the original checkout will not appear automatically. The common
symptom is an agent spending its first minutes diagnosing a setup problem you already solved
in another folder.

Configure [Files to copy and the setup command](/docs/guide/projects/#project-automation) for
repeatable preparation. Copy only the configuration needed for the task, and choose development
services deliberately. Installing dependencies in two folders does not isolate a database URL
that points both processes to the same database.

## Integration still takes judgment

Two branches can pass their own tests and fail together. A backend change can invalidate a UI
assumption without touching the same lines. Even a conflict-free merge needs tests of the
combined result.

A practical approach is to split work by independently reviewable outcomes. Ask one agent to
improve a diagnostic message while another documents an existing feature. If both need to
redesign the same interface, agree on that interface first or do the work in sequence.

Review each branch, merge one, then bring the other up to date using your usual git tools.
Resolve conflicts, rerun the relevant checks, and review what changed during integration.
Yardsort's [changes panel](/docs/guide/changes-and-files/) helps inspect the files; it cannot
establish that two product decisions are compatible.

## Context needs its own handoff

A second agent sees the files in its working directory, but that does not give it another
agent's reasoning or unfinished plan. For a review or continuation on the same work,
[hand off the workspace](/docs/guide/terminals-and-sessions/#handing-work-to-another-agent) with an
explicit summary. For a separate task, start a separate workspace. Use
[project memory](/docs/guide/memory/) for reviewed lessons that should survive either conversation.

Try the distinction yourself in the [parallel Claude Code and Codex tutorial](/tutorials/claude-code-codex-parallel-worktrees/).
