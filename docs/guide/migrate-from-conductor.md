# Migrate from Conductor to Yardsort

Keep your Git branches and local worktrees while moving a parallel coding workflow to Yardsort, an open-source desktop app for Linux, macOS and Windows.

Conductor and Yardsort both organize independent tasks around Git worktrees. You can open the repository you already have and import its worktrees without moving their folders. Claude Code, Codex and the other agent CLIs you already installed run in real terminals. Their subscriptions or API usage stay with those providers.

This guide is for **local workspaces**. It does not import Conductor Cloud environments, Conductor's application settings, or Conductor's chats. Yardsort publishes this guide. The Conductor column follows Conductor's public documentation as checked on 8 November 2026. The Yardsort steps match release 0.22.2, the latest release that day, and the same import, Forget and project-settings behavior on the current main branch.

Importing an existing Git worktree keeps its folder, branch and files, skips preparation, and **Forget…** leaves those files in place. That behavior is covered by Yardsort's tests on a disposable repository. Those tests do not start the Conductor app, translate its scripts, or recover a chat. There is no automatic importer for Conductor settings or conversation history, and this page does not promise a search ranking.

## What maps to what?

| In Conductor | In Yardsort |
| --- | --- |
| Project opened from a local repository | Project opened from that same repository folder |
| Workspace on its own branch and Git worktree | Workspace backed by an ordinary Git worktree |
| Existing local worktree | **Import worktrees…**, which keeps the folder and branch |
| One workspace per independent Claude Code or Codex task | One workspace per independent task, with the CLI agent you choose |
| Multiple chats in one workspace, sharing that branch | Multiple agent tabs in one workspace, sharing its files |
| Files to copy, and a setup script | Project settings: **Files to copy**, **Setup executable** and **Setup arguments** |
| Run actions, including several named actions | One **Run executable** and **Run arguments**, started with **▶ Run** |
| `CONDUCTOR_ROOT_PATH` and `CONDUCTOR_WORKSPACE_PATH` in setup | `YARDSORT_PROJECT` and `YARDSORT_WORKSPACE`. Set for setup only |
| Allocated `CONDUCTOR_PORT` range | Configure distinct ports yourself. Nothing is assigned for you |
| Archive script | No archive script. Clean up external resources yourself |
| Diff review and pull request flow | **Changes**, then commit, push and open a pull request. GitHub checks use your existing `gh` login |
| Notes in a workspace `.context` folder | The files stay in the imported folder. They are not converted into [project memory](memory.md) |
| Conductor Cloud or the phone app | Not a finished replacement. [Remote](remote.md) is still under development |

## 1. Protect your current work

Let running agents finish, or stop them, before another agent edits the same files. From the repository's original checkout, list its worktrees:

```sh
git worktree list
```

Save a backup of any uncommitted or ignored files you need. Pushing a branch does not preserve those files. Keep any conversation export you want separately. Importing a Git worktree does not import a chat archive.

Do not remove an imported workspace's folder through Conductor while Yardsort is using that same folder. Both tools would be referring to the same files. Keep the workspace directories until you have deliberately finished with the work inside them.

## 2. Install Yardsort and check your agent

You need Git and at least one agent CLI that already works in a fresh terminal. For example:

```sh
claude --version
```

On macOS, install Yardsort with Homebrew:

```sh
brew install --cask joaoh82/yardsort/yardsort
```

Or download your system's build from the [latest release](https://github.com/joaoh82/yardsort/releases/latest):

- Linux: AppImage, deb or rpm. For the AppImage, run `chmod +x Yardsort_*.AppImage`, then open it. The [quick start](../quick-start.md#linux-add-the-appimage-to-your-app-menu) shows how to add it to an app menu.
- macOS: open the universal dmg and drag Yardsort to Applications.
- Windows: use the setup exe or msi. Builds are not yet code-signed, so SmartScreen can warn: choose **More info**, then **Run anyway**. [Git for Windows](https://git-scm.com/download/win) is required.

A source build needs [Rust](https://rustup.rs) (stable), [Bun](https://bun.sh), Git, [`just`](https://just.systems) and Tauri's [system dependencies](https://tauri.app/start/prerequisites/). The [README](https://github.com/joaoh82/yardsort#build-from-source) then gives:

```sh
git clone https://github.com/joaoh82/yardsort && cd yardsort
just setup
just build
```

See the [quick start](../quick-start.md) for prerequisites and platform details. Agent subscriptions or API usage are separate from Yardsort.

## 3. Open the existing repository

In Yardsort, press **+** next to Projects, then **Open a folder**. Select the original Git repository checkout associated with your Conductor worktrees, rather than making an unrelated new clone.

The project appears with a **local** entry for that checkout. Opening the repository does not import worktrees that live outside Yardsort's worktree folder (`~/yardsort` by default). Yardsort does pick up worktrees it finds under that folder. Conductor worktrees normally sit elsewhere, so they stay out of the list until you import them.

## 4. Import your local worktrees

Open the project's menu and choose **Import worktrees…**. The composer for a new workspace also has it, in the line **Have worktrees already?**

The dialog lists every worktree Git knows about that is not already a workspace, with its branch and path, all ticked. Untick any you do not want. The button reads **Import 1 worktree**, or **Import N worktrees** when more than one is ticked. Yardsort records each chosen workspace without moving its directory or changing its checked-out branch. Your existing files remain in that directory. The workspace is named after its folder. Preparation does not run, including **Files to copy** and the setup command, on an imported worktree or when that worktree is later restored.

If a worktree is missing, compare the dialog with `git worktree list` from the original checkout. Do not delete folders to make them appear. See [Worktrees made elsewhere](workspaces.md#worktrees-made-elsewhere).

To stop tracking an imported workspace without removing its files or branch, use **Forget…**. Saved conversations stay unless you tick **Also delete its saved conversations**. **Archive…** and **Delete workspace…** are different: each removes the folder, after a second confirmation when there are uncommitted changes, and keeps the branch.

## 5. Reconfigure setup and run commands

Yardsort does not import Conductor's in-app scripts or `.conductor/settings.toml`. Open **Project settings…** and configure the commands you actually need. These settings are stored in Yardsort's local database for that project. Checking out a repository cannot turn a command on.

- **Files to copy:** one relative file path per line, copied from the original checkout into a newly prepared worktree. Examples are `.env` and `config/local.json`. Use `/` separators on every platform. Directories, symlinks, absolute paths, parent traversal and `.git` paths are rejected. Existing destination files are not overwritten.
- **Setup executable:** the program to run after copying, in the new worktree, before an agent starts. Leave it empty to disable setup.
- **Setup arguments:** one argument per line, without shell quotes. Blank lines are ignored. For `bun install`, set the executable to `bun` and the argument to `install`.
- **Run executable** and **Run arguments:** one command, started with **▶ Run** in a regular terminal tab. For `bun run dev`, set the executable to `bun`, with `run` and `dev` on separate argument lines. Conductor's several named run actions do not become several Yardsort commands. Pick the one you want, or start the others yourself in a shell tab.

Argument fields are passed directly to the program. Shell operators and environment-variable expansion are not interpreted. For a longer shell script, choose `bash` as the setup executable and put the script's relative path in the arguments, or `pwsh` with `-File` and `scripts/setup.ps1` on separate lines.

Inside a setup script, replace `CONDUCTOR_ROOT_PATH` with `YARDSORT_PROJECT` (the original checkout) and `CONDUCTOR_WORKSPACE_PATH` with `YARDSORT_WORKSPACE` (the worktree). Those two are set for setup, which is noninteractive and stops after ten minutes. They are not expanded in argument fields, and they are not a general rename of every Conductor variable. Conductor's docs also describe `CONDUCTOR_WORKSPACE_NAME`, `CONDUCTOR_PORT` (a range of ten ports) and `CONDUCTOR_IS_LOCAL`. Yardsort does not set those and does not allocate ports. Choose distinct development-server ports yourself when more than one workspace would otherwise use the same port.

There is no archive hook. If a Conductor archive script removed databases, certificates or other resources outside the worktree, run that cleanup yourself when you are done. Do not point **Archive…** at that job: in Yardsort, archive removes the worktree folder and keeps the branch.

Imported worktrees skip preparation. Settings apply when Yardsort creates or restores a worktree of its own, and when you open an existing branch in a new worktree. Read [Project automation](projects.md#project-automation) before translating a project-specific script.

## 6. Start an agent and review the result

Open an imported workspace and start the CLI agent you want from the tab bar, or press **+** on the project to open the composer. Do not assume Conductor's conversation appears in Yardsort. Git state and a provider's session history are different things. Keep the previous app's exports if that history matters. Use an agent's own recovery only when that agent's documentation says the session is still available. This guide does not promise that Yardsort's Resume will find a Conductor chat.

For a fresh independent task, press **+** on the project, describe the task, select an installed agent such as Claude Code or Codex, and start it. Yardsort creates a separate branch and worktree. Start another task in another workspace to work in parallel. Start another agent from the tab bar of the same workspace when both should share its files. Tabs in one workspace are not isolated from each other.

Review the changed files in **Changes**, then commit, push and open a pull request. With the GitHub CLI installed and logged in (`gh auth login`), Yardsort can open the pull request from here. Without it, the branch can still be pushed and the forge's own form opened in the browser. Yardsort holds no forge credentials of its own. You can also keep using your usual Git tools. See [Commits and pull requests](commits-and-pull-requests.md).

Once you are working in Yardsort, **Hand off…** can prepare an editable first message for the next agent from the context Yardsort has recorded in that workspace. It does not import Conductor's activity. See [Handing work to another agent](terminals-and-sessions.md#handing-work-to-another-agent).

## Frequently asked questions

### Will importing move my worktree or change my branch?

No. Import keeps the folder and the checked-out branch where they are. Because both apps can reference that same folder, avoid concurrent editing, and do not remove the folder from the previous app until you are finished with the files.

### Will Yardsort import my Conductor scripts and conversations?

Not automatically. Translate setup and run settings by hand. Importing worktrees does not import Conductor application settings or chats. A provider CLI's own session recovery is a separate step, and it is not guaranteed.

### Can I use Claude Code and Codex together?

Yes. Use separate workspaces for separate branches, or multiple agent tabs in one workspace when they should share files. Each CLI uses its own login and provider limits. Yardsort can also start other terminal agents you have installed.

### Does this replace Conductor Cloud or its phone app?

No. This guide is for local Git worktrees. [Remote](remote.md) is still under development, and its phone app is not in the app stores. Do not treat it as a finished Cloud migration.

### Is Yardsort free?

Yardsort is free software under GPL-3.0. Your coding agents' subscriptions or API usage remain separate. Optional [Assist](assist.md) uses your own TypeSafe account.

### Can I schedule checks after migrating?

Yes, in the current release. A schedule can start a workflow or a smoke test in a workspace you already have, while Yardsort is open. A time missed while it is closed can run once when Yardsort next opens, or be skipped, depending on that schedule. Saving a smoke-test schedule in the app shows the file's steps and approves that version. A later change to the file still waits for approval. A schedule does not run an unapproved test by itself, and it does not create a fresh worktree for each run. See [Schedules](schedules.md).

## Install Yardsort and try one workspace

[Download Yardsort](https://github.com/joaoh82/yardsort/releases/latest), import one local worktree, and check its files before moving the rest of your workflow.

If Yardsort is useful to you, you can [star the GitHub repository](https://github.com/joaoh82/yardsort).

For a product comparison rather than these steps, see [Yardsort vs Conductor](https://www.yardsort.sh/blog/yardsort-vs-conductor/). That article was last verified on 30 September 2026. It is not this migration guide.

### Sources and further reading

Checked on 8 November 2026:

- [Conductor workspaces and branches](https://www.conductor.build/docs/concepts/workspaces-and-branches)
- [Conductor parallel Claude Code sessions](https://www.conductor.build/docs/guides/parallel-agents/run-multiple-claude-code-sessions)
- [Conductor scripts](https://www.conductor.build/docs/reference/scripts)
- [Yardsort quick start](../quick-start.md)
- [Yardsort workspaces](workspaces.md)
- [Yardsort project automation](projects.md#project-automation)
