# Troubleshooting

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

## Resume says the conversation was not found

The agent no longer has that conversation on disk — it was cleaned up, or it never saved one
(a session with no messages has nothing to save). Forget the entry and start a new session.

If this happens for **every** session, check whether you start Yardsort from a terminal that is
itself running inside an agent; versions before 0.1 leaked that agent's session markers into the
agents they launched, which stopped Claude Code from saving transcripts.

## Coming from Switchyard

Yardsort was called Switchyard until v0.2. The first time Yardsort starts it **copies** your
database and settings from the Switchyard folders (`dev.switchyard.app`) into its own, so your
projects, workspaces and session history are all there. Nothing is moved or deleted, so the old
app keeps working; once you are happy, uninstall Switchyard and delete its folders.

- Workspaces stay where they are on disk (typically `~/switchyard/…`); new ones go to
  `~/yardsort/…`. Branches named `sy/…` keep their names; new ones are `ys/…`.
- `SWITCHYARD_DATA_DIR` and `SWITCHYARD_WORKTREE_ROOT` still work; prefer the `YARDSORT_` names.
- Panel sizes are per app and start fresh.

## Where Yardsort keeps things

|                                                  | Linux                                      | macOS                                             | Windows                       |
| ------------------------------------------------ | ------------------------------------------ | ------------------------------------------------- | ----------------------------- |
| Database (projects, workspaces, session records) | `~/.local/share/dev.yardsort.app/`         | `~/Library/Application Support/dev.yardsort.app/` | `%APPDATA%\dev.yardsort.app\` |
| Settings                                         | `~/.config/dev.yardsort.app/settings.toml` | same folder as above                              | same folder as above          |
| Worktrees                                        | `~/yardsort/` (configurable)               |                                                   |                               |

Yardsort stores no credentials and sends nothing anywhere: no telemetry, no account. Agents use
their own logins and talk to their own services.

To start from scratch, quit Yardsort and delete the database folder. Your repositories,
branches and worktrees are not touched.

## Reporting a bug

[Open an issue](https://github.com/joaoh82/yardsort/issues/new/choose) with your OS, the
Yardsort version (status bar, bottom right), the agent and its version, and what you did. If
the app misbehaves at startup, running it from a terminal shows its log.
