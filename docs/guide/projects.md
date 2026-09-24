# Projects

A **project** is a git repository on your computer that Yardsort knows about. Projects live in
the left panel; everything else hangs off them.

## Adding a project

Press **+** next to _Projects_.

### Open a folder

`Ctrl+Shift+O` / `⌘O` goes straight here. Pick a folder and:

| The folder is…                                  | What happens                                                                                                                                                    |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| a git repository                                | It is added.                                                                                                                                                    |
| _inside_ a repository (say `repo/packages/web`) | The repository's root is added instead, and a note tells you so.                                                                                                |
| not a repository                                | Yardsort asks whether to initialise git there. Saying yes runs `git init` and makes an empty first commit — your files are not changed. Saying no adds nothing. |
| already one of your projects                    | Nothing is duplicated; the existing project is selected.                                                                                                        |

Yardsort needs a repository because every workspace is a git worktree, and it needs at least one
commit because a worktree has to branch from something.

### Create a new project

Give it a **name** and a **location**. Yardsort creates `<location>/<name>`, runs `git init`
and makes an empty first commit. The location is remembered for next time.

The name becomes a folder name, so characters that are illegal on some system (`/ \ : * ? " < > |`)
are refused, and an existing folder is never touched. If any step fails, the half-made folder is
removed again.

## The `local` workspace

Every project has a **local** entry, always first. It is your repository's own checkout — not a
worktree — and the branch checked out there is shown next to it. It cannot be renamed, archived or
deleted.

Clicking it selects it, and the middle panel **asks what to open** rather than picking for you,
because both answers are reasonable in your own checkout:

- **Open Terminal** — a shell in the project root, on the branch you have out.
- **Open Composer** — the composer, but running the agent **in this checkout**: no new branch, no
  new worktree. Everything else is the same — harness, model, effort, your first message — and the
  conversation is recorded so it can be resumed and forked like any other.

It only asks when there is nothing running there and no earlier conversation to come back to;
otherwise clicking brings what is already there into view. That is the same rule a worktree
workspace follows, except a worktree opens a shell instead of asking.

An agent in `local` has no isolation: it edits the branch you have checked out, like you would.
That is the point of it — for a quick fix, or work you intend to commit where you are — but it is
why workspaces exist, and why they are the default.

The branch label follows reality: switch branches in another tool and it updates the next time
Yardsort's window gets focus.

## The project menu

Hover a project and press **⋯**, or right-click it:

- **Project settings…** — files to copy into new worktrees, a setup command, and a run/dev-server command. See [Project automation](#project-automation).
- **New workspace** — same as the **+** on the row. See [Workspaces](workspaces.md).
- **Import worktrees…** — make workspaces of worktrees that already exist in the repository, made
  by hand or by another tool. See
  [Worktrees made elsewhere](workspaces.md#worktrees-made-elsewhere).
- **Reveal in file manager**
- **Move up / Move down** — the order is remembered.
- **Remove from Yardsort…** — takes the project off the list and closes its terminals.
  **Nothing on disk is deleted**: the folder, its branches and its worktrees all stay. By
  default Yardsort also keeps its own record — the workspaces, imported ones included, and their
  saved conversations — so adding the same folder again brings everything back as it was. Tick
  **Also delete its workspaces and their saved conversations** in the dialog to start over
  instead; the next time the folder is added, only worktrees under Yardsort's own folder are
  picked up (see [Worktrees made elsewhere](workspaces.md#worktrees-made-elsewhere)).

Click a project's arrow to collapse it. Collapsed projects, your selection and the panel sizes are
all restored the next time you start Yardsort.

## When a folder goes missing

If a project's folder is moved, deleted or on a drive that is not mounted, the project is shown
struck through and marked **missing**. It is not removed — it comes back by itself when the folder
does. If it is gone for good, remove the project from its menu.

## Project automation

Open **Project settings…** from the project menu, or **Project settings** above a workspace's
terminal tabs. These settings are stored locally in Yardsort's database, per project; checking
out a repository cannot enable a command. Removing a project with its history kept also keeps
these settings. Deleting its record clears them.

**Files to copy** takes one relative file path per line, for example `.env` or
`config/local.json`. Files come from the project's original checkout, including ignored files.
Use `/` separators on all platforms. Directories, symlinks, absolute paths, parent traversal and
`.git` paths are rejected. Existing destination files are never overwritten; a missing source
or conflicting destination stops preparation and leaves the worktree available for inspection.

**Setup executable** and **Setup arguments** run after copying, in the new worktree, before the
agent starts. Enter one argument per line without shell quotes; blank lines are ignored. For
example, use executable `bun` and argument `install`. For a script, use `bash` with
`scripts/setup.sh`, or `pwsh` with `-File` and `scripts/setup.ps1` on separate lines. Arguments
are passed directly; shell operators and environment-variable expansion are not interpreted.
Leave the executable empty to disable it.

Preparation also runs when opening an existing branch in a new worktree, or restoring a worktree
created by Yardsort. Imported and adopted worktrees do not run preparation, including on restore.
It does not run on `local` or merely selecting an existing workspace.
The CLI's `ys workspace new`, including `--no-agent`, uses the same preparation.

Setup uses the resolved launch environment, with `YARDSORT_PROJECT` set to the original checkout
and `YARDSORT_WORKSPACE` to the new worktree. It is noninteractive and has a ten-minute limit.
Output is saved as `yardsort-setup-….log` in the worktree’s private Git directory, outside the
checkout. It cannot be staged by `git add -A` and does not make the worktree dirty. Git removes
the log when the worktree is archived or deleted; inspect or save it first if needed. To locate
that directory from the worktree, run `git rev-parse --absolute-git-dir`. Logs may contain secrets,
so inspect them before sharing. If preparation fails, the agent is not started and the error
names the retained worktree. For an unsuccessful setup process, it also gives the log’s full
path. Open a shell in the worktree to fix the problem and rerun your setup command manually;
read the log using that full path, or locate it with `git rev-parse --absolute-git-dir`.
A timeout stops the setup process; check for any child processes it started before retrying.
If the agent fails to launch after preparation, the prepared worktree is kept too.

**Run executable** and **Run arguments** configure **▶ Run** above the terminal tabs. For example,
use `bun` with `run` and `dev` on separate lines. Run starts in the selected workspace, including
`local`, in a regular terminal tab: output remains visible and the process keeps running when
you switch workspaces or close the window. Press Run again to focus an existing running server;
after it exits, Run starts a new one. Use the terminal's Ctrl+C to stop it, or close its tab.
If no command is configured, Run opens Project settings. Each workspace runs its own server;
configure ports in your project if multiple servers would otherwise use the same port.
