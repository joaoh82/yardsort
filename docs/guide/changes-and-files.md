# Changes & files

The right panel is for **reviewing** what happened in the selected workspace. It updates by
itself as files change — there is nothing to refresh. Toggle it with `Ctrl+Shift+Alt+B` / `⌥⌘B`.

![Changes and a diff](../images/overview.png)

## Changes

Two groups:

- **Uncommitted** — modified, added, deleted, renamed and untracked files in the working tree.
- **On this branch · vs `<base>`** — what the workspace's branch has committed since it left the
  branch it was started from. Shown for worktree workspaces; this is the view of "everything this
  agent did", whether or not it has committed yet.

Each row shows the kind of change (**M** modified, **A** added, **D** deleted, **R** renamed,
**U** untracked) and the lines added and removed. The tab shows the total count.

Click a file to see its **diff** below the list, with removed lines in red, added lines in green,
and the exact characters that changed highlighted within them. It starts **inline** — removals
folded into one pane — and **Side by side** puts the two versions in two panes instead, which is
easier on a wide diff. The choice sticks until you change it back.

| Button                        | Does                                                                          |
| ----------------------------- | ----------------------------------------------------------------------------- |
| **Side by side** / **Inline** | Two panes, or one with the removals folded in. Diffs only.                    |
| **Expand** / **Shrink**       | Give the viewer the whole panel, or go back to the split view.                |
| **Edit ↗**                    | Open the file in your editor — see [Settings → General](settings.md#general). |
| **×**                         | Close the viewer.                                                             |

Binary files and very large files are listed but not rendered.

### Who wrote it

With any **Capture what … reports** switch on in [Settings → General](settings.md#general),
the list also shows what the workspace's agents _said_ they wrote. A changed file an agent
reported writing carries a small badge with that agent's name; hover it for how many times it
reported writing the file, when it last did, and through what (`claude/hook`, `codex/notify`,
…). A line above the list counts the files with such a report and the files without one — you,
a script, or an agent run that was not reporting — and the expanded viewer's header says the
same for the open diff: _reported by claude · 2 writes · 14:02_, or _no agent reported writing
this_.

Two things it never says. Which **lines** came from which report: git shows every change since
the last commit, and no agent reports a line, so a file you and the agent both touched carries
the agent's badge over the whole diff. And anything at all while **no** agent run in the
workspace was reporting: then every file is unreported for the same reason, and the list reads
as above. A report that says the change failed — Codex says so for a patch that did not
apply — is not counted. Grok's log names the tools it ran and never the file, so a Grok run
counts as reporting but badges nothing. See [Activity](activity.md) for what each agent
reports.

### Assist badges

With [Assist](assist.md) switched on, files carry small badges — **off-task**, **secret**,
**tests**, **checks**, **credentials** — shortly after an agent stops writing, and a line above
the list says when they were last checked. Assist is off by default and needs an API key of your
own.

## Files

The workspace's folder as a tree. Click a folder to open it, a file to read it (with syntax
highlighting; read-only).

By default the tree shows what git would: `.git`, and everything ignored by `.gitignore` —
`node_modules`, build output, `.env` files — is left out, which keeps a big repository quick to
browse. Press **ignored** at the right of the tab bar to show those too; they appear dimmed so you
can tell them apart. The choice is remembered.

## Sending the work on

Under the list are the three steps that get a workspace's work out: commit, push, and open the
pull request. See [Commits & pull requests](commits-and-pull-requests.md).

Reviewing itself stays read-only. Yardsort will not stage part of a change or discard one — do
that the way you already do, with a shell tab (`Ctrl+Shift+T`), your editor or your git client.
The workspace is an ordinary git checkout at the path shown in the footer.
