# Workspaces

A **workspace** is one line of work inside a project: a git branch, checked out in its own folder
(a [git worktree](https://git-scm.com/docs/git-worktree)), with the terminals running in it.
Separate folders keep each task's file changes apart. Agents started in the same workspace share
its files, and changes on different branches can still conflict when you merge them.

Workspaces are plain git. Everything Yardsort does you can inspect with `git worktree list` and
`git branch`, and nothing stops you using those folders from any other tool.

## Starting one: the composer

Press **+** on a project row, or `Ctrl+Shift+N` / `⌘N` for the project you are in.

![The composer](../images/composer.png)

| Field       | What it does                                                                                                                                                                                                                                                                                                                                  |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Message** | The agent's first prompt. `Enter` starts, `Shift+Enter` makes a new line. Leave it empty to just open the agent. Drop a file here to insert its path — see below. After **Hand off…** it starts as what Yardsort recorded about the workspace — see [handing work to another agent](terminals-and-sessions.md#handing-work-to-another-agent). |
| **Harness** | Which agent to run. The list comes from [settings](settings.md).                                                                                                                                                                                                                                                                              |
| **Model**   | Free text with suggestions. Empty means "let the agent use its default" — no flag is passed at all.                                                                                                                                                                                                                                           |
| **Effort**  | Offered only for agents that have the concept.                                                                                                                                                                                                                                                                                                |
| **Branch**  | Under _New branch from_, pick the branch to start from. Under _Open existing branch_, pick a branch to check out in a workspace as it is — no new branch is made.                                                                                                                                                                             |

Under the pickers, **Import worktrees…** opens the same dialog as the project menu, for worktrees
that already exist — see [Worktrees made elsewhere](#worktrees-made-elsewhere).

When the project [shares its memory](memory.md#giving-agents-the-memory), a line under the pickers says its approved entries go after your message — or, while nothing is approved yet, that the agent will be asked to propose some — **Show** reads what is added, and unticking leaves it out of this one launch. Your message stays the task on record.

Under the pickers, the composer also shows your history with the agent you picked, across your projects — _kept 3 of 5_, or _too few to say yet_ — from the [outcomes](outcomes.md) you have recorded. It is shown, never used to choose for you.

Your last choices are remembered per project. With [Assist](assist.md) switched on, a line under
the pickers may offer a harness and an effort for what you are typing; **Use** applies it, and
ignoring it does nothing.

**Drop a file on the composer** to put its path in the message, at the cursor, so the agent starts
already pointed at it. Drop several and each path is added, separated by spaces. The composer is
outlined while the file is over it. The path is written as plain text in the prompt, and one that
contains a space is quoted. The same gesture on a
[terminal](terminals-and-sessions.md#typing-to-an-agent) pastes the path into a program that is
already running, quoted for the shell.

**Nothing is created until you press Start** —
cancelling (`Esc`) leaves no trace.

On Start, Yardsort:

1. names the workspace from your message (`Add a --units flag…` → `add-units-flag`), making it
   unique if needed;
2. runs `git worktree add` with a new branch, `ys/add-units-flag` by default;
3. starts the agent there with your message.

Naming looks for an explicit task in the message, including after introductory background:
`I've been looking at the settings screen. Could you please add dark mode?` becomes
`add-dark-mode`. It skips fenced code and common request lead-ins, then keeps up to four words
and 32 characters. This is local text extraction, not an AI summary; unrecognized wording falls
back to the opening words. Empty or unusable messages get a railway station name. Names are
chosen when the workspace is created; later messages do not rename it.

If preparation or the agent launch fails, the worktree and its branch are kept, so script output
and copied files remain available. The error names its folder. Fix the problem and open the
workspace to start an agent, or delete it through the workspace menu if it is no longer needed.

### From a pull request

**Start workspace** on a pull request in the [Pull requests](pull-requests.md) view opens this
same composer with the pull request's branch already chosen under **Open existing branch**. The
agent, the message and **Start** are yours as usual, and nothing is created until you press it.
The line under the box says when the branch comes from a fork — Yardsort will not push it — and
when a branch you already had is behind the pull request. See
[Start a workspace from a pull request](pull-requests.md#start-a-workspace-from-a-pull-request).

### From a task

**Delegate** on a task in the [Tasks](tasks.md) view opens this same composer with the message
written from the task — its title, description and latest comments, marked as text other people
wrote. A line above the box says which task, and to read the message before you start. The
workspace is named after the task rather than the message, and remembers it: its row shows the
task's key until it has a pull request. See
[Handing a task to an agent](tasks.md#handing-a-task-to-an-agent).

### Where the folders go

`~/yardsort/<project>/<workspace>` by default. Change the folder and the `ys/` branch prefix in
[Settings → Workspaces](settings.md#workspaces). Changing them affects new workspaces only.

### Files git does not track

A fresh worktree contains what is committed — so `.env` files, `node_modules` and other ignored
or untracked things are **not** there by default. Configure **Files to copy** and a setup command
in [Project settings](projects.md#project-automation) to prepare each new or restored worktree
automatically, or install dependencies and copy files yourself in a shell tab.

## Worktrees made elsewhere

Not every worktree of a repository is a workspace. Yardsort shows the ones it made, and picks up
on its own only worktrees under its own folder (`~/yardsort` by default, see
[Settings → Workspaces](settings.md#workspaces)) — which is how a project's workspaces come back
when the project is removed and added again. A worktree anywhere else — made by hand, by another
tool, or by a script — is left alone until you ask for it.

To bring those in, choose **Import worktrees…** from the [project menu](projects.md#the-project-menu)
or from the composer. The dialog lists every worktree git knows about that is not a workspace,
with its branch and its path, all ticked; untick what you do not want and press **Import**.
Nothing is moved or changed: the folder stays where it is, on the branch it has, and Yardsort just
learns about it. An imported workspace is named after its folder; rename it if you like.

To go the other way, **Forget…** in the workspace menu takes a workspace out of Yardsort without
touching its folder or its branch.

## The workspace menu

Hover a workspace and press **⋯**, or right-click it.

### Run workflow… and Request code review…

Start a [workflow](workflows.md) in this workspace: **Run workflow…** asks which, **Request code
review…** goes straight to the built-in review. Both open the same dialog, which asks the
workflow's inputs and shows the pull request it will use. The `local` workspace has them too,
from its right-click menu.

### Rename…

Changes the **label only**. The folder and the branch keep their names on purpose: agents file
their conversations by folder, so moving it would orphan them — and would pull the rug from under
anything running there.

### Archive…

For work you are done with _for now_. The folder is removed from disk; the **branch, its commits
and the session history are kept**. The workspace moves to a collapsed **archived (n)** group at
the bottom of its project.

**Restore workspace**, from the archived entry's menu, checks the branch out again _at the same
path_ — which is what makes its old conversations resumable.

### Forget…

For a worktree that is not Yardsort's to remove — one you imported, or one another tool owns.
The workspace leaves the list and **nothing on disk is touched**: the folder and the branch stay
exactly as they are. Terminals running in it are closed.

Its saved conversations are kept by default, so if the worktree is ever imported again they are
back with it. Tick **Also delete its saved conversations** in the dialog to drop them instead.

### Delete workspace…

Removes the folder and forgets the workspace. **The branch is always kept** — deleting a workspace
never throws away commits. Delete the branch yourself with git if you want it gone.

`ys workspace delete <name>` does the same thing from a terminal, which is how a harness can
remove a workspace it is finished with. See [the command line](cli.md#ys-workspace-delete-workspace).

### How did it go?

After an archive or a delete, the bottom of the sidebar asks how the attempt went: **Kept**,
**Partly** or **Discarded**. One optional click; **×** leaves it for later. Deleting keeps the
attempt's record, so it can be judged afterwards in **Outcomes…** — see [Outcomes](outcomes.md).

### Uncommitted work is protected

Archiving and deleting both remove a folder, and uncommitted changes live only there. If there
are any, Yardsort stops and asks again, saying plainly that the work will be lost for good.
Nothing is destroyed on the first click. On the command line the same refusal is the default:
`ys workspace delete` removes nothing until `--force` is passed.

## When a workspace's folder disappears

If the folder is deleted behind Yardsort's back, the workspace is marked **missing**. Its menu
offers **Restore from its branch** (check it out again in the same place) or **Delete**.

### When the branch goes too

Removing a worktree _and_ its branch with git — `git worktree remove` followed by `git branch -D`,
say — leaves a workspace with nothing behind it: no folder to open, no branch to restore from. It
is marked **gone**, and its menu offers only **Rename…** and **Delete workspace…**.

Yardsort notices this the next time it reads the project — at startup, and whenever its window
comes back into focus — and asks whether to delete the workspace from the app as well. Saying
**Delete workspace** forgets the workspace and its session history; nothing on disk is touched,
because there is nothing left there. Saying **Keep** leaves the entry alone and is remembered, so
you are not asked about the same workspace twice.

## Workspace previews

Hover or keyboard-focus a workspace row for its name, branch and pull request details, and the
[task it was started from](tasks.md#handing-a-task-to-an-agent) when there is one. The PR
preview includes review status, line counts and expandable checks; see
[Commits & pull requests](commits-and-pull-requests.md).

The harness pill counts open harness tabs, including finished tabs whose output is still open;
shell tabs are excluded. Hover or focus the pill to see each harness and whether it is working,
waiting, finished or failed. Select a harness in the preview to open that workspace and its
existing terminal. Escape dismisses the preview; Arrow Down from the focused pill enters it.
