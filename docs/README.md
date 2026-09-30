# Yardsort documentation

**New here?** Start with the [quick start](quick-start.md) — from download to your first agent in a
few minutes.

## Using Yardsort

For supported agents, platforms, costs, worktree isolation and data handling, read
[About Yardsort](guide/questions.md).

| Guide                                                         | What it covers                                                                              |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [Quick start](quick-start.md)                                 | Install, add a project, start your first workspace                                          |
| [Projects](guide/projects.md)                                 | Adding, creating, reordering and removing projects; the `local` workspace                   |
| [Workspaces](guide/workspaces.md)                             | The composer, branches and worktrees, rename / archive / restore / delete                   |
| [Terminals & sessions](guide/terminals-and-sessions.md)       | Tabs, shells, status dots, notifications, resume and fork, handing work to another agent    |
| [Changes & files](guide/changes-and-files.md)                 | Reviewing what an agent did: changed files and who wrote them, diffs, the file tree         |
| [Commits & pull requests](guide/commits-and-pull-requests.md) | Committing, pushing and opening a pull request; check results on a row                      |
| [Settings & harnesses](guide/settings.md)                     | Configuring agents, adding your own, worktree folder, editor                                |
| [Assist](guide/assist.md)                                     | Optional AI checks on changed files and suggestions in the composer                         |
| [Activity](guide/activity.md)                                 | The local record of what ran, what each agent reports, and the experimental timeline        |
| [Memory](guide/memory.md)                                     | Lessons about a project for its agents: written by you, proposed by agents, approved by you |
| [Outcomes](guide/outcomes.md)                                 | What became of each attempt, and each agent's history on your work                          |
| [Workflows](guide/workflows.md)                               | Named, reusable agent work in YAML: the built-in code review, writing and checking your own |
| [Updates](guide/updates.md)                                   | How Yardsort finds and installs new versions, and which copies can                          |
| [Keyboard shortcuts](guide/shortcuts.md)                      | Every shortcut, and why they look the way they do                                           |
| [The `ys` command line](guide/cli.md)                         | Installing it; starting, deleting and handing off workspaces from a terminal or a script    |
| [Troubleshooting](guide/troubleshooting.md)                   | "Command not found", blank windows, where your data lives                                   |

## Getting help

- Something broken or missing? [Open an issue](https://github.com/joaoh82/yardsort/issues/new/choose) —
  see [Troubleshooting](guide/troubleshooting.md) first for the common ones.
- Anything else — a question, help getting started, a word about what you are building — write to
  **[hello@yardsort.sh](mailto:hello@yardsort.sh)**.
- Found a vulnerability? Report it privately: [SECURITY.md](../SECURITY.md).

## Building and contributing

- [CONTRIBUTING.md](../CONTRIBUTING.md) — set up a dev environment, run the checks, send a change
- [Releasing](releasing.md) — how versions are cut and what the release workflow does
- [Design docs](design/README.md) — vision, architecture, harness model, roadmap, open questions
