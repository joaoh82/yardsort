# About Yardsort

## What is Yardsort, and who is it for?

Yardsort is an open-source desktop app for developers running AI coding agents in parallel.
Each task gets a git worktree and a terminal, so you can work on several branches, review the
changes and open pull requests from one place. Start with the [quick start](../quick-start.md).

## Which agents and operating systems does it support?

Yardsort runs on Linux, macOS and Windows. It has built-in configurations for Claude Code,
Codex, Grok, OpenCode, OMP, Cursor and Pi. You can add other terminal agents with a
[custom harness](settings.md#adding-your-own-harness).

## Is Yardsort free?

Yardsort is free to download and use, with source code under the
[GPL-3.0 license](../../LICENSE). Agent subscriptions or API usage are separate: each agent
uses your own account. Optional [Assist](assist.md) requests are billed to your TypeSafe account.

## Do I need to install the coding agents separately?

Yes. Install git and at least one coding agent CLI, and make sure the agent works in a fresh
terminal before starting Yardsort. Yardsort does not bundle agents; they use their own login.
The [installation guide](../quick-start.md#1-before-you-start) lists the prerequisites.

## How do git worktrees help agents work in parallel?

A git worktree checks out a branch in a separate folder. Yardsort gives each new task its own
worktree, keeping that task's file changes apart from work in other workspaces. Agents started
in the same workspace share its files. You still review changes and resolve any conflicts when
merging branches. See [workspaces](workspaces.md) and [reviewing changes](changes-and-files.md).

## Will my agents keep running if I close the window?

Yes, if you choose **Leave them running** when closing with agents still working. A local
background process owns the terminals, and reopening Yardsort reconnects to them. **Stop them**
ends the agents instead. This does not keep work running through a computer shutdown. See
[terminals and sessions](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window).

## What stays local, and what can be sent to a provider?

Yardsort stores its project, workspace and session records on your computer. The coding agents
you run can send prompts and code to their own providers under their own settings.

The [Usage](usage.md) view reads the token counts the agents keep in their own logs on this
machine, and the operating system's process figures; it sends nothing.

Optional Assist uses TypeSafe Jev. Depending on the features you enable, it sends changed-file
diffs and task context, composer text and harness descriptions, or memory proposals and approved
entries. Assist does not send terminal output. The [Assist guide](assist.md) explains what each
feature sends. Update checks and downloads also contact GitHub; see [updates](updates.md).
