# Workspaces

A **workspace** is one line of work inside a project: a git branch, checked out in its own folder
(a [git worktree](https://git-scm.com/docs/git-worktree)), with the terminals running in it.
Because each workspace has its own folder, agents working in parallel never step on each other —
or on you.

Workspaces are plain git. Everything Yardsort does you can inspect with `git worktree list` and
`git branch`, and nothing stops you using those folders from any other tool.

## Starting one: the composer

Press **+** on a project row, or `Ctrl+Shift+N` / `⌘N` for the project you are in.

![The composer](../images/composer.png)

| Field       | What it does                                                                                                                                                      |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Message** | The agent's first prompt. `Enter` starts, `Shift+Enter` makes a new line. Leave it empty to just open the agent.                                                  |
| **Harness** | Which agent to run. The list comes from [settings](settings.md).                                                                                                  |
| **Model**   | Free text with suggestions. Empty means "let the agent use its default" — no flag is passed at all.                                                               |
| **Effort**  | Offered only for agents that have the concept.                                                                                                                    |
| **Branch**  | Under _New branch from_, pick the branch to start from. Under _Open existing branch_, pick a branch to check out in a workspace as it is — no new branch is made. |

Your last choices are remembered per project. **Nothing is created until you press Start** —
cancelling (`Esc`) leaves no trace.

On Start, Yardsort:

1. names the workspace from your message (`Add a --units flag…` → `add-units-flag`), making it
   unique if needed;
2. runs `git worktree add` with a new branch, `ys/add-units-flag` by default;
3. starts the agent there with your message.

If the agent cannot be started — not installed, say — the worktree and the new branch are taken
back, so a failed attempt leaves nothing behind. A branch that existed before is never deleted.

### Where the folders go

`~/yardsort/<project>/<workspace>` by default. Change the folder and the `ys/` branch prefix in
[Settings → Workspaces](settings.md#workspaces). Changing them affects new workspaces only.

### Files git does not track

A fresh worktree contains what is committed — so `.env` files, `node_modules` and other ignored
or untracked things are **not** there. Ask the agent to install dependencies, or copy what you
need in a shell tab.

## Worktrees made elsewhere

Worktrees you created by hand, or with another tool, show up as workspaces automatically the next
time the project is listed. Nothing is moved or changed; Yardsort just learns about them.

## The workspace menu

Hover a workspace and press **⋯**, or right-click it.

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

### Delete workspace…

Removes the folder and forgets the workspace. **The branch is always kept** — deleting a workspace
never throws away commits. Delete the branch yourself with git if you want it gone.

### Uncommitted work is protected

Archiving and deleting both remove a folder, and uncommitted changes live only there. If there
are any, Yardsort stops and asks again, saying plainly that the work will be lost for good.
Nothing is destroyed on the first click.

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
