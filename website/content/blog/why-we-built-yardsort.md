# Why we built Yardsort

Yardsort started with a practical frustration: using coding agents from different providers while
moving between Windows, Mac and Linux. There were tools for managing agents, but none that felt
right across all three operating systems. Switching machines should not mean rebuilding the way
you work.

That became the first requirement: a desktop app that runs on all three, keeps the agents' own
terminal interfaces, and gives each task a separate git worktree. The project's
[original design notes](https://github.com/joaoh82/yardsort/blob/main/docs/design/01-vision.md)
record those choices. The rest grew from using the app.

## Building Yardsort with Yardsort

After the initial version, Yardsort became the tool used to develop Yardsort itself. That changed
how features were chosen. A repeated interruption in the development workflow was a reason to
build something; a longer feature list was not enough on its own.

One example was switching agents. Having several providers available is useful, but another
terminal on the same branch does not automatically give the next agent the context it needs.
What was the task? What changed? What remains unfinished? Repeating that explanation is work,
and leaving it out makes the next conversation less useful.

## Handoffs carry the work forward

[Agent-to-agent handoff](/docs/guide/terminals-and-sessions/#handing-work-to-another-agent)
prepares a message for the next agent from the workspace's recorded task and current state.
You can read and edit it before starting the next session. The next agent works with the
workspace's existing files; it does not magically inherit the previous model's internal state.

That distinction matters. A handoff should make the transfer of context visible and useful,
without pretending two providers share a conversation format. You still decide what to pass on.

[Project memory](/docs/guide/memory/) addresses a different timescale. A lesson about the
repository should not have to be rediscovered in every new task. Approved notes can be shared
with future agents, with the user deciding what belongs in that record and when to include it.

## Repeated tasks became workflows

Another recurring pattern was asking a different agent to review work an agent had just done.
The steps were similar each time, even when the code was different.

Yardsort's [workflows](/docs/guide/workflows/) turn that routine into an inspectable YAML file.
You can write the file directly or ask a model to draft it. Generated YAML returns to the editor
for checking and saving; asking for a draft does not run the workflow. The built-in code review
is a starting point for putting a second agent to work on a concrete review task.

These features belong together: workspaces organize tasks, handoffs transfer the current work,
memory carries reviewed lessons forward, and workflows describe routines worth repeating.

## Local records and open source

The desktop app keeps project, workspace and session records on your computer. It is
[open source under GPL-3.0](https://github.com/joaoh82/yardsort/blob/main/LICENSE), and its
workspaces remain ordinary git branches and directories you can open with other tools.

Local records do not make a cloud coding agent offline. Your agents can send code and prompts
to their providers. Optional Assist and model-generated workflow drafts also involve provider
requests. The [data-handling overview](/docs/guide/questions/#what-stays-local-and-what-can-be-sent-to-a-provider)
and [workflow guide](/docs/guide/workflows/) explain those boundaries.

## Next: carrying context between machines

Yardsort Sync is in development. The intended workflow is sharing project memory and handing
work between machines, with an optional hosted service, end-to-end encryption, open-source code
and a self-hosting option. Those are plans for Sync, not capabilities of the current desktop
release or a claim that its security has already been validated.

The reason for it comes back to the original problem: work moves between operating systems and
machines. Useful context should be able to move with it, under the user's control.

To try the current workflow, follow [running Claude Code and Codex in parallel](/tutorials/claude-code-codex-parallel-worktrees/).
