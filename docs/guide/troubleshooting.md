# Troubleshooting

Use this guide when Yardsort cannot start, cannot find an agent or cannot display a terminal.
If you are setting it up for the first time, check the [quick-start prerequisites](../quick-start.md#1-before-you-start)
before changing your configuration.

## Yardsort cannot start

If something goes wrong before the window can open, Yardsort shows a **Yardsort cannot start**
dialog that says what, and quits when you close it. Started from a terminal, it prints the same
text there.

The one you are most likely to see says your projects and workspaces **were last opened by a
newer version of Yardsort**. Each release may upgrade the database the first time it opens it, and
an older release cannot read what a newer one wrote. That happens when you go back to an older
download, or have two installs of different versions (an AppImage and a package, say). Install
the latest release from the [Releases page](https://github.com/joaoh82/yardsort/releases/latest)
and everything is where you left it: the older version refused before writing anything.

### Going back to the older version instead

Before a release upgrades the database, Yardsort keeps a copy of it as it was:
`yardsort.db.before-upgrade`, beside the database (see [where](#where-yardsort-keeps-things)).
The version you had before that upgrade can open the copy. The dialog names the copy when there
is one this version can read. The copy has nothing you did after the upgrade: projects,
workspaces, session records and activity since then are not in it. Your repositories and worktrees
are on disk either way.

To use it, quit Yardsort and anything running `ys`, then in the database's folder:

1. Rename `yardsort.db` to `yardsort-newer.db`. If `yardsort.db-wal` and `yardsort.db-shm` are
   there, rename them to `yardsort-newer.db-wal` and `yardsort-newer.db-shm`. They belong to the
   newer database, and left beside the copy they would damage it.
2. Rename `yardsort.db.before-upgrade` to `yardsort.db`.

Nothing is deleted this way. To go forward again later, undo the renames before starting the
newer version, or it will upgrade the older copy instead and `yardsort-newer.db` stays unused.

Yardsort keeps one copy, from the most recent upgrade. Each upgrade replaces it.

Versions up to 0.13.0 did not show the dialog. They closed a second or two after launch, every
time, with nothing on screen. If an older Yardsort does that, this is the most likely reason; run
it from a terminal to see the message.

## "`claude` was not found on PATH"

Yardsort launches programs with the environment of your **login shell**, which it reads once at
startup. The status bar shows the result: `env: login shell · 34 PATH`.

1. Check the command works in a **new** terminal window: `which claude`.
2. If you installed it while Yardsort was running, click the **`env: …`** indicator in the status
   bar (or **Check again** on the welcome screen): Yardsort re-reads your environment without a
   restart.
3. If your `PATH` is set somewhere only some shells read, move it to your shell's profile
   (`~/.zprofile`, `~/.bash_profile`, `~/.config/fish/config.fish`).
4. Or put the full path in the harness's **Command** field —
   [Settings → Harnesses](settings.md#harnesses) shows where a command was found, or that it was not.

If the status bar says `shell environment unavailable`, hover it for the reason — usually a shell
startup file that waits for input or takes more than a few seconds. Yardsort then falls back to
the environment it was started with.

## The welcome screen says git or an agent is missing

That screen is a live check of what Yardsort can find with your login shell's `PATH`. Install
what it asks for using the command shown (or the linked instructions), then press **Check
again**. If the tool _is_ installed and works in a terminal, the problem is `PATH` — see the
section above. Agents you have switched off in Settings → Harnesses do not count.

## Windows

- **SmartScreen warning when installing** — builds are not code-signed yet. Choose
  **More info → Run anyway**.
- **git is required** — install [Git for Windows](https://git-scm.com/download/win).
- **Agents that need WSL** — some agents support Windows only through WSL. Running an agent
  inside WSL from Yardsort is not supported yet.
- **Path too long** — keep the [worktree folder](settings.md#workspaces) short (`C:\ys`) and run
  `git config --global core.longpaths true`.

## Linux

- **Blank or flickering window** (mostly NVIDIA on Wayland) — start with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1 yardsort`.
- **Terminal drawing looks wrong** — Yardsort uses the GPU (WebGL) and falls back to a slower
  renderer automatically if the GPU context is lost. The status bar shows which is active
  (`webgl` or `dom`).
- **No notifications** — you need a notification daemon (mako, dunst, or your desktop's own).
- **Wayland or X11.** Yardsort runs as a native Wayland window on a Wayland desktop. (The AppImage's
  launcher asks for X11. Yardsort overrides that, because under XWayland typing lags and dictation
  tools that type for you, such as Omarchy's, drop or garble characters.) If a Wayland start of
  the AppImage ever fails before the window appears, the next start uses X11 for that version by
  itself (a start that [stopped with a dialog](#yardsort-cannot-start) does not count). To choose,
  set `YARDSORT_GDK_BACKEND=wayland` or `YARDSORT_GDK_BACKEND=x11`. To let a version try Wayland
  again, delete the `wayland-failed` file beside the database.

## macOS

- **"Yardsort is damaged" / cannot be opened** — that is Gatekeeper reacting to a build that was
  not notarized. Official releases are signed and notarized; for a build you made yourself, run
  `xattr -dr com.apple.quarantine /Applications/Yardsort.app`.

## A workspace says "missing"

Its folder is gone. Use **Restore from its branch** or **Delete** from the workspace's menu — see
[Workspaces](workspaces.md#when-a-workspaces-folder-disappears).

## A workspace says "gone", and Yardsort offers to delete it

Its folder _and_ its branch were both removed outside Yardsort, so there is nothing left to check
out again. Delete the workspace when asked, or **Keep** the entry — Yardsort will not ask about
that workspace again. If you deleted the branch by mistake, `git reflog` can still point you at the
commit it was on; recreate the branch, and **Restore from its branch** comes back by itself.

## Agents stopped when I closed Yardsort

They should not: they run in a background process that outlives the window (see
[Terminals & sessions](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window)).
If they do, the status bar at the bottom right says **no daemon** — hover it for the reason.

- Started with `YARDSORT_NO_DAEMON=1`? That is what it does. Drop it.
- Otherwise Yardsort could not start or reach one, and fell back to running terminals inside
  itself so the app still works. `daemon.log`, next to the database (below), says why.

Two things that cause it: the socket's folder is not writable, or the path to it is too long.
Unix socket paths are limited to about a hundred characters — much shorter than any other path
limit — so a very deep `YARDSORT_DATA_DIR` can be the problem. Yardsort tries
`$XDG_RUNTIME_DIR/yardsort/` first, then the folder holding the database, then the temp folder,
taking the first one short enough; the log names what it settled on.

To look at the daemon yourself: the status bar shows its process id, and you can start one by
hand with `yardsort --yardsort-daemon <socket path>`, which logs to the terminal.

## "Agents are still running under a previous version"

An update replaced Yardsort while agents were working in the daemon the old version started.
Rather than kill work you did not agree to lose, Yardsort leaves that daemon alone and runs new
terminals inside itself until you are done with the old ones.

Finish or stop those agents, then restart Yardsort and everything is back to normal. This can
only happen when a release changes how the app and the daemon talk to each other, which is rare.

## Resume says the conversation was not found

The agent no longer has that conversation on disk — it was cleaned up, or it never saved one
(a session with no messages has nothing to save). Forget the entry and start a new session.

If this happens for **every** session, check whether you start Yardsort from a terminal that is
itself running inside an agent; versions before 0.1 leaked that agent's session markers into the
agents they launched, which stopped Claude Code from saving transcripts.

## The Pull requests view is empty, or says `gh` failed

The view reads GitHub through the [GitHub CLI](https://cli.github.com), and says on a line above
the list why a project has no rows. The fixes, in the order they usually apply:

- **`gh` is not installed, or nobody is logged in** — install it, run `gh auth login` in a
  terminal, then press **Refresh**. Yardsort looks for `gh` on the same `PATH` it finds your
  agents on, so if a terminal finds it and Yardsort does not, see
  ["`claude` was not found on PATH"](#claude-was-not-found-on-path) above: the cause is the same.
- **`HTTP 502` or `HTTP 504`** — GitHub took too long. It happens on repositories with a great
  many pull requests and checks; **Retry** usually succeeds, and the rows that did arrive stay.
- **The project is on GitLab, Bitbucket or Gitea, or has no remote** — the view is for GitHub.
- **A pull request you expected is not there** — the list holds every open one up to the 200
  most recently updated, and the newest fifty of any state. An older merged or closed one is on
  GitHub.

See [Pull requests](pull-requests.md#when-something-is-missing).

## The Tasks view is empty, or `ys task` says it cannot read tasks

Tasks are a project's GitHub issues, read through the same [GitHub CLI](https://cli.github.com),
so the first three causes above are the same and so are their fixes. Two more are its own:

- **The repository has issues switched off** — then it has no tasks. Forks often have.
- **A task you expected is not there** — the list holds every open issue up to the 200 most
  recently updated, and closed ones only on the **Closed** and **All** tabs, the 50 most
  recently updated. A pull request is not a task.

`ys task` says the same things in a sentence and exits with an error; when only part of a list
could be read it prints what arrived and says so on standard error.

See [Tasks](tasks.md#when-something-is-missing).

## The clone dialog has no list of repositories

**Clone a repository** lists your GitHub repositories through the same
[GitHub CLI](https://cli.github.com). Without it, or with nobody logged in, the dialog says so
on the line where the list would be, and the field still takes a URL or `owner/repository`.
Install `gh`, run `gh auth login` in a terminal, close the dialog and open it again.

- **A repository is not in the list** — it holds the 200 you pushed to most recently, of those
  you own or collaborate on. Organisation repositories are not listed at all: type three
  characters of the name and press **Search GitHub for “…”**.
- **The list stopped short** — a page GitHub did not answer: what arrived is shown, with the
  reason and **Retry**.
- **It is marked _Already added_ but you cannot see it** — the match is by push remote, so the
  project is there under whatever name its folder has; **Go to project** selects it.

See [Projects](projects.md#clone-a-repository).

## Coming from Switchyard

Yardsort was called Switchyard until v0.2. The first time Yardsort starts it **copies** your
database and settings from the Switchyard folders (`dev.switchyard.app`) into its own, so your
projects, workspaces and session history are all there. Nothing is moved or deleted, so the old
app keeps working; once you are happy, uninstall Switchyard and delete its folders.

- Workspaces stay where they are on disk (typically `~/switchyard/…`); new ones go to
  `~/yardsort/…`. Branches named `sy/…` keep their names; new ones are `ys/…`.
- `SWITCHYARD_DATA_DIR` and `SWITCHYARD_WORKTREE_ROOT` still work; prefer the `YARDSORT_` names.
- Panel sizes are per app and start fresh.

## `ys` says there is no database

```
No Yardsort database at /home/you/.local/share/dev.yardsort.app/yardsort.db.
Open the app once to create one, or point at another profile with --data-dir.
```

`ys` works the location out from the rules in the table below rather than asking the app, so this
means one of three things: Yardsort has never been run on this machine, you are in a different
user account, or `ys` resolved a directory the app does not use.

Run `ys doctor` — it prints the directory it tried. If that is not where your data actually is,
point at it directly and please [report it](https://github.com/joaoh82/yardsort/issues):

```sh
ys --data-dir ~/.local/share/dev.yardsort.app project list
```

`ys` will not create a database, on purpose. If it did, a wrong directory would look exactly like
a Yardsort you had never used.

## Where Yardsort keeps things

|                                                  | Linux                                      | macOS                                             | Windows                       |
| ------------------------------------------------ | ------------------------------------------ | ------------------------------------------------- | ----------------------------- |
| Database (projects, workspaces, session records) | `~/.local/share/dev.yardsort.app/`         | `~/Library/Application Support/dev.yardsort.app/` | `%APPDATA%\dev.yardsort.app\` |
| Settings                                         | `~/.config/dev.yardsort.app/settings.toml` | same folder as above                              | same folder as above          |
| Worktrees                                        | `~/yardsort/` (configurable)               |                                                   |                               |

`yardsort.db.before-upgrade`, when it is there, is the database as it was before the last
upgrade — the way back to the version before it ([above](#going-back-to-the-older-version-instead)).

`daemon.log` sits beside the database and holds the last run of the background process that owns
your terminals. It is replaced each time one starts.

`ys doctor` prints all of these for your machine, which is quicker than reading the table.

Yardsort stores no credentials and sends nothing anywhere: no telemetry, no account. Agents use
their own logins and talk to their own services.

To start from scratch, quit Yardsort and delete the database folder. Your repositories,
branches and worktrees are not touched.

## Reporting a bug

[Open an issue](https://github.com/joaoh82/yardsort/issues/new/choose) with your OS, the
Yardsort version (status bar, bottom right), the agent and its version, and what you did. If
the app misbehaves at startup, running it from a terminal shows its log.

Please report a security problem privately instead — GitHub's
[Report a vulnerability](https://github.com/joaoh82/yardsort/security/advisories/new) form, as
described in [SECURITY.md](../../SECURITY.md).

## Still stuck

If none of this helped, or your question is not a bug, write to
**[hello@yardsort.sh](mailto:hello@yardsort.sh)**. Say which OS and Yardsort version you are on,
and what you were trying to do.
