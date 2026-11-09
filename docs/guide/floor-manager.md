# Floor Manager

The Floor Manager is one agent with every project in view. Ask it what projects you have, what
the agents in them are doing, which pull requests nobody has reviewed, or which tasks are
waiting. Ask it to start work, and it suggests how, then starts it once you say yes.

It is an ordinary agent in an ordinary terminal. You pick the agent: Claude Code, Codex,
OpenCode, or any other you have set up. It runs in a folder of its own rather than in a
project, and it is told how to use [`ys`](cli.md), Yardsort's command line, which sees exactly
what the window sees. Yardsort does not read what it prints, as with every agent.

_Floor Manager_ is a working name.

![The Floor Manager, asked which pull requests need attention](../images/floor-manager.png)

## Opening it

- **Floor Manager** at the right end of the bottom bar, in the lower-right corner of the window.
  A dot before the name means one is running.
- **Mod+J** (`Ctrl+Shift+J`, or `⌘J` on macOS). You can change it in
  [Keyboard shortcuts](shortcuts.md).
- **Mod+K**, then _Floor Manager_.

It opens as a panel over the lower-right corner. The rest of the window keeps working beside it.
**×** closes the panel, and so does the button or the shortcut again. **Closing it leaves the
agent running**, the same as closing the window does: it belongs to the
[daemon](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window), and it is there
with what it said when you open the panel again.

## Starting it

The first time, the panel says what the Floor Manager is for and asks which agent to run it
with. Only agents that are switched on and installed are offered. Press **Start**. The choice is
remembered for next time.

There is only ever one. Opening the panel again shows the one that is running, whichever agent
it is.

The first time an agent runs in a new folder it may ask whether to trust it. Claude Code and
Codex both do. The folder is the Floor Manager's own (see [Where it lives](#where-it-lives)), so
saying yes is safe.

## What to ask

- _What projects do we have, and what is running in them?_
- _Which pull requests are open and not reviewed?_
- _Which pull requests need attention?_ That means failing checks, conflicts, changes requested,
  your review asked for, or no review at all.
- _Check the open pull requests and start reviewers on them._
- _What is waiting in the tasks for api-gateway?_
- _Start this task in yardsort: …. Is there a workflow for it?_

It answers from `ys`: `ys project list`, `ys session list`, `ys pr list`, `ys task list`,
`ys workflow list` and the rest. Pull requests and tasks are read from GitHub through your
[GitHub CLI](https://cli.github.com), so they need `gh` installed and logged in. With `gh`
missing, it says so.

## What it does, and what it asks first

The Floor Manager is told where the lines are:

- **It reads without asking.** Listing projects, workspaces, sessions, pull requests, tasks,
  workflows, schedules and triggers, and what an agent has on its screen.
- **It proposes, then waits for your yes,** before anything that starts an agent, makes a branch,
  runs something or writes to GitHub. That covers a new workspace (`ys workspace new`), handing
  a task to an agent (`ys task start`), opening a workspace on a pull request
  (`ys pr checkout`), running a workflow or a smoke test, and commenting on or closing a task.
  It says exactly what it will run. One yes covers what it proposed, not the next idea.
- **It never** deletes or archives a workspace, deletes a branch, forces anything, merges,
  closes or approves a pull request, edits files or commits in a project, types into another
  agent's terminal, or approves [memory](memory.md). It tells you where in Yardsort to do those
  yourself.

These lines are what the agent is _told_, in its instructions. They are not a sandbox. The
Floor Manager runs with the same access to your computer as any agent you start in a
workspace, and the agent's own permission prompts still apply. Claude Code, for one, asks
before it runs a command unless you have told it not to. An agent you run in auto mode does not
ask.

### Reviewing pull requests

For a pull request that a Yardsort workspace opened, the Floor Manager proposes the built-in
[code review](workflows.md#requesting-a-code-review) on that workspace.

For one somebody else opened, it first proposes `ys pr checkout <number> --project <name>`. That
makes a workspace on the pull request's branch, with no new branch and no agent. Then it proposes
the code review on that workspace. You can agree to the first and not the second. A reviewer starts there, reads and runs the code without
editing it, and posts its review on GitHub. You get a notification when it has.

A pull request from a fork gets a branch named `pr/<number>`. The built-in review finds a pull
request by its branch name, so it cannot review one of those yet. The Floor Manager says so.

### Workflows

Asked to start something, the Floor Manager reads `ys workflow list` and suggests a
[workflow](workflows.md) that fits, with the inputs it needs. If none fits, it suggests a plain
workspace instead.

### Running in the cloud

Yardsort does not run agents in the cloud. Everything the Floor Manager starts runs on this
computer, and it says so if you ask for a cloud run.

## Stopping it

**Stop**, in the panel's header, asks _End its conversation?_ first. **End it** stops it, and
**Keep it** leaves it running. You can also end it from inside, the way you would end that agent
anywhere, such as `/exit`.

When it has ended, by either way, the panel still shows its last screen and says _The Floor
Manager has ended._ **Start again** begins a **new** conversation. Picking up the old one is not
offered yet.

Closing Yardsort with the Floor Manager running asks the same question as with any agent that is
still working. It is listed there as _Floor Manager_.

## Where it lives

In a folder named `coordinator` inside your profile's data directory:

| OS      | Folder                                                       |
| ------- | ------------------------------------------------------------ |
| Linux   | `~/.local/share/dev.yardsort.app/coordinator`                |
| macOS   | `~/Library/Application Support/dev.yardsort.app/coordinator` |
| Windows | `%APPDATA%\dev.yardsort.app\coordinator`                     |

With `YARDSORT_DATA_DIR` set, it is `$YARDSORT_DATA_DIR/coordinator`.

At every start Yardsort writes two files there. `AGENTS.md` holds the instructions, which most
agents read. `CLAUDE.md` points Claude Code at them. **Both are replaced at every start**, so
an edit to them does not last. Anything else you put in the folder is left alone.

The agent is started with:

- `YARDSORT_DATA_DIR` set to your profile, so `ys` reads the same projects the window shows;
- the folder that holds this copy of Yardsort's own `ys` first on its `PATH`, so it uses the
  `ys` that matches the app, whether or not you [installed](cli.md#installing) one. A
  development build without `ys` built says so in the panel, and the agent then uses whichever
  `ys` it finds.

After Yardsort updates itself, **Start again** gives the Floor Manager the new instructions and
the new `ys`. The one already running keeps the old ones until it ends.
