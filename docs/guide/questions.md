# About Yardsort

## What is Yardsort, and who is it for?

Yardsort is an open-source desktop app for orchestrating coding agents on Linux, macOS and
Windows. Parallel tasks get their own git worktrees; several agents can share one. Hand the work
to the next agent with recorded context and project memory, let agents read what the others said,
check the work with smoke tests an agent writes, then review changes and open pull requests from
one place. Start with the [quick start](../quick-start.md).

## Which agents and operating systems does it support?

Yardsort runs on Linux, macOS and Windows. It has built-in configurations for Claude Code,
Codex, Grok, OpenCode, OMP, Cursor and Pi. You can add other terminal agents with a
[custom harness](settings.md#adding-your-own-harness).

## Is Yardsort free?

Yardsort is free to download and use, with source code under the
[GPL-3.0 license](../../LICENSE). Agent subscriptions or API usage are separate: each agent
uses your own account. Optional [Assist](assist.md) requests are billed to your TypeSafe account.

## Can I see token usage and how much CPU or memory agents use?

Yes. Open **Usage** from the sidebar or command palette. **Token usage** reads Claude Code,
Codex and Grok's local logs for the last 7, 30 or 90 days, including sessions started outside
Yardsort. Compare agents, models and workspaces, inspect cache savings, and see Codex's last
reported plan limits. API cost estimates are for comparison and do not represent your
subscription bill; models with unknown prices still count toward tokens.

**Machine resources** shows CPU and memory for Yardsort and every terminal it runs, including
each agent's child processes. It samples every two seconds while the tab is open and the window
is visible. Both tabs stay local and need no extra key or activity capture. See [Usage](usage.md).

## Do I need to install the coding agents separately?

Yes. Install git and at least one coding agent CLI, and make sure the agent works in a fresh
terminal before starting Yardsort. Yardsort does not bundle agents; they use their own login.
The [installation guide](../quick-start.md#1-before-you-start) lists the prerequisites.

## How do git worktrees help agents work in parallel?

A git worktree checks out a branch in a separate folder. Yardsort gives each new task its own
worktree, keeping that task's file changes apart from work in other workspaces. Agents started
in the same workspace share its files. You still review changes and resolve any conflicts when
merging branches. See [workspaces](workspaces.md) and [reviewing changes](changes-and-files.md).

## Can my agents see what another agent said?

Yes, in the same workspace, unless you turn it off. `ys workspace context` prints every message you
typed and each turn's reply from the workspace's other conversations, read from the agents' own
transcripts when it runs. Tool output and reasoning are left out, and Yardsort keeps no copy. A new
agent is told the others are there. Untick **Share conversations between this project's agents** in
a project's Memory view to stop it. See
[what agents can read of each other](terminals-and-sessions.md#what-agents-can-read-of-each-other).

## Can agents test their own work?

Yes. An agent writes a [smoke test](smoke-tests.md), a short YAML file in your repository: start
the server, wait for it, hit an endpoint, open a page in a Chromium already on your machine, look
for a word. Yardsort runs it step by step and keeps every output, screenshot and reason for a
failure. A file from a repository never runs until you have approved it, new or changed, and
**Tests** in the sidebar lists every project's tests and runs.

## Can I use Yardsort from my phone?

Not as a finished feature yet. [Remote](remote.md) is still under development, and the phone app is
not in the app stores. It stays off, and connects to nothing, until you turn it on in Settings, and
whoever runs the server you sign in to can see what passes through it.

## Will my agents keep running if I close the window?

Yes, if you choose **Leave them running** when closing with agents still working. A local
background process owns the terminals, and reopening Yardsort reconnects to them. **Stop them**
ends the agents instead. This does not keep work running through a computer shutdown. See
[terminals and sessions](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window).

## What stays local, and what can be sent to a provider?

Yardsort stores its project, workspace and session records on your computer. The coding agents
you run can send prompts and code to their own providers under their own settings. That includes
what one agent reads of another's conversation, unless you
[turn sharing off](memory.md#sharing-conversations): Claude's conversation, read by Codex, goes to
OpenAI as part of Codex's work.

The [Usage](usage.md) view reads the token counts the agents keep in their own logs on this
machine, and the operating system's process figures; it sends nothing.

Optional [Remote](remote.md), off until you turn it on, passes what your paired devices see and type,
terminals included, through the server you sign in to. Whoever runs that server can see it.

Optional Assist uses TypeSafe Jev. Depending on the features you enable, it sends changed-file
diffs and task context, composer text and harness descriptions, or memory proposals and approved
entries. Assist does not send terminal output. The [Assist guide](assist.md) explains what each
feature sends. Update checks and downloads also contact GitHub; see [updates](updates.md).
