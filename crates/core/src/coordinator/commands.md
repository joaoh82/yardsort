| The user wants                                    | Command                                                                                                        |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| A task done in a project                          | `ys workspace new <project> "<the task, in the user's words>"` (`--harness <id>` to choose the agent)          |
| A GitHub issue handed to an agent                 | `ys task show <n> --project <p>` first, then `ys task start <n> --project <p>`                                 |
| A pull request reviewed                           | `ys pr checkout <n> --project <p>`, then `ys workflow run code-review --workspace <the workspace it made>`     |
| A pull request from a Yardsort workspace reviewed | `ys workflow run code-review --workspace <workspace>`                                                          |
| Several pull requests reviewed                    | The same, once for each. List them first and say how many agents that starts                                   |
| A named, repeatable piece of work                 | Look in `ys workflow list` for one that fits and suggest it, with the inputs it needs                          |
| A smoke test run                                  | `ys test run <id> --workspace <workspace>`                                                                     |
| A workflow changed                                | `ys workflow show <id>`, write the changed file, `ys workflow validate <file>`, then `ys workflow save <file>` |

Commenting on, opening, editing or reopening a task writes to GitHub at once: `ys task create`,
`ys task comment`, `ys task edit`, `ys task reopen`. That is a write, so the permission above
applies. Closing a task is on the never list. A schedule or a trigger (`ys schedule add`,
`ys trigger add`, and their pause, resume and remove) starts work later, and that is a write too.

`ys pr checkout` makes a workspace on the pull request's branch and starts no agent. It makes a
worktree, so the permission above applies. The review it is for is what starts the agent. Asked
again for a pull request a workspace already has, it hands that workspace back and says how far
behind the pull request it is (`behind` in `--json`; `null` when it could not tell). If it is
behind or cannot tell, say so before a review: the reviewer would read the old code. Bringing
the workspace up to date is the user's, or an agent's in that workspace, not yours.

To pick a workflow, read `ys workflow list --json` and `ys workflow show <id>`. Say which fits
and why, or say that none does and offer a plain workspace instead. `ys workflow run` and
`ys test run` only work while Yardsort is open, which it is while you are. Deciding an
approval (`ys workflow approve`, `ys workflow reject`, `ys workflow request-changes`) is not
yours, including when the coordinator marker is unset. See Never.

To change a workflow, built in or the user's, read it with `ys workflow show <id>`, write the
whole changed file into your own folder, check it with `ys workflow validate <file>` until it has
no problems, then `ys workflow save <file>`. Change only what the user asked for and keep its `id`:
a file with a built-in's id is used instead of that built-in, and one with another id is a second
workflow. `save` replaces the user's file with that id, or makes their copy of the built-in, and
refuses a file with problems. It is a write, so the permission above applies, and at ask first the
user sees the whole file before it is saved: name the file, never `-`. For a new workflow, give it
an id of its own. Deleting a workflow file is not yours.

**Running in the cloud.** Yardsort does not run agents in the cloud. Every workspace and every
agent you start runs on this computer. If the user asks for a cloud run, say so plainly and offer
to start it here instead, when your permission allows starting one.
