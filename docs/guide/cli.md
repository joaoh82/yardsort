# The `ys` command line

`ys` drives Yardsort from a terminal. It reads the same database as the app, so a workspace made
with `ys` appears in the window and a workspace made in the window appears in `ys`. It does not
need the app to be running.

An agent started by `ys` belongs to the [daemon](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window),
not to the command — so it keeps working after `ys` returns, and you can open Yardsort later to
watch it. That is the whole point of it.

## Installing

`ys` comes with the app. Every copy of Yardsort carries it next to its own executable, built from
the same commit, so the two always agree about the database. Whether it is on your `PATH` depends
on how you installed Yardsort:

| How you installed Yardsort               | Where `ys` is                                                                                                                                                                     | How it is updated                                                                                                                                     |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.deb`, `.rpm`, or the AUR package       | `/usr/bin/ys`, installed with the app. Nothing to do.                                                                                                                             | By your package manager, with the app.                                                                                                                |
| Homebrew cask                            | Linked onto your `PATH` by Homebrew.                                                                                                                                              | With the app.                                                                                                                                         |
| macOS app (`.dmg`)                       | **Install ys** makes `/usr/local/bin/ys`, a link into `Yardsort.app`. macOS asks for your password when that folder needs it.                                                     | With the app: the link points into it. Signed and notarized with it, too.                                                                             |
| Linux AppImage                           | **Install ys** copies it to `~/.local/bin/ys`. An AppImage is mounted somewhere new every time it starts, so a link would stop working.                                           | When Yardsort starts and finds an older copy there, it replaces it.                                                                                   |
| Windows installer (`-setup.exe`, `.msi`) | **Install ys** copies it to `%LOCALAPPDATA%\dev.yardsort.app\bin\ys.exe` and adds that folder to your user `PATH`. Terminals opened afterwards find it; ones already open do not. | When Yardsort starts and finds an older copy there, it replaces it. A `ys.exe` that is running at that moment is renamed aside and removed next time. |

**Install ys** is in **Settings → General → Command line**. The first-run checklist on the welcome
screen has it too, while `ys` is missing or out of date. Both say where your terminal finds `ys`
and which version it is. It is optional: nothing in the app needs `ys`.

- **Something else is called `ys`.** If there is already a file at the install location and it is
  not Yardsort's `ys`, Yardsort asks before replacing it; **Cancel** leaves it alone. A `ys`
  already there, of any version, is replaced without asking. So is a broken link left by a `Yardsort.app` that has since
  moved; a broken link to anything else counts as something else.
- **Another `ys` comes first.** If your terminal finds a `ys` somewhere else — one you unpacked
  by hand, say — the panel names it. Installing then puts Yardsort's own copy at the install
  location, but the other one keeps winning until you remove it.
- **`~/.local/bin` is not on your `PATH`.** Most Linux distributions add it when it exists, but
  not all. The panel says so; add it in your shell's startup file (for example
  `export PATH="$HOME/.local/bin:$PATH"`) and press **Check again**.
- **macOS says to move Yardsort first.** A link into an app that is still on its disk image, or
  that macOS is running from a temporary copy, would break. Drag Yardsort into Applications and
  open it from there.

To remove it, delete the file or link at the install location, and on Windows the folder's
entry in your user `PATH`.

### Without the app

`ys` is also a separate download: a single binary in the assets of each
[release](https://github.com/joaoh82/yardsort/releases). Put it anywhere on your `PATH`.

```sh
# Linux
tar -xzf ys-*-linux-x86_64.tar.gz && sudo install ys /usr/local/bin/

# macOS (universal)
tar -xzf ys-*-macos-universal.tar.gz && sudo install ys /usr/local/bin/
```

On Windows, unzip `ys-*-windows-x86_64.zip` and put `ys.exe` in a folder on your `PATH`.

This copy of `ys` is not signed or notarized on macOS, unlike the one inside the app. A copy
downloaded with a browser is therefore quarantined and refused on first run. Either download it
with `curl`, or clear the flag:

```sh
xattr -d com.apple.quarantine ys
```

A copy installed this way does not update itself.

## Commands

Run `ys --help`, or `ys <command> --help`, for the full list. Every command takes `--json`, which
prints a machine-readable version instead of a table.

### `ys project list`

The repositories Yardsort knows about, with how many workspaces each has.

```
NAME       WORKSPACES  PATH
yardsort   3           /home/you/projects/yardsort
```

Projects are added in the app; `ys` does not create them.

### `ys workspace list`

Every workspace, including each project's own `local` checkout. `--project <name>` narrows it to
one project and `--all` includes archived ones. With `--json` each entry carries a `kind`, which
is `local` for a project's own checkout and `worktree` for one Yardsort made.

### `ys workspace new <project> "<prompt>"`

The main one. Creates a branch and a worktree, then starts an agent in it with your prompt as its
first message — the same thing the composer does in the app.

```sh
ys workspace new yardsort "fix the flaky login test"
```

```
created  fix-the-flaky-login-test
  branch ys/fix-the-flaky-login-test
  path   /home/you/yardsort/yardsort/fix-the-flaky-login-test
  agent  claude (a49add8f)

It keeps working after this command returns. Open Yardsort to watch.
```

| Option             | What it does                                                     |
| ------------------ | ---------------------------------------------------------------- |
| `--base <branch>`  | Branch to start from. Default: the repository's default branch.  |
| `--harness <id>`   | Which agent to run. Default: the first one found on your `PATH`. |
| `--model <model>`  | Model, for harnesses that take one.                              |
| `--effort <level>` | Effort or thinking level, for harnesses that take one.           |
| `--no-agent`       | Create the branch and worktree and start nothing.                |

The project can be named or given by id. If the harness you asked for is not configured or not
installed, nothing is created at all — no branch, no folder, no record.

### `ys workspace delete <workspace>`

Removes the folder and forgets the workspace, including its session history. **The branch is
kept** — commits are never thrown away, same as [Delete workspace](workspaces.md#delete-workspace)
in the app. Delete the branch yourself with git if you want it gone. A project's own `local`
checkout cannot be deleted.

```sh
ys workspace delete fix-the-flaky-login-test
```

```
deleted  fix-the-flaky-login-test
  branch ys/fix-the-flaky-login-test (kept)
  path   /home/you/yardsort/yardsort/fix-the-flaky-login-test
```

The workspace can be a name or an id from `ys workspace list --json`. A name that matches more
than one workspace is refused, and the ids are printed so you can say which one. Archived
workspaces are included: deleting one removes it for good.

Uncommitted changes and untracked files are refused, and nothing is removed. They live only in
the folder, so there is no way back to them. `--force` is the confirmation that they should go:

```sh
ys workspace delete fix-the-flaky-login-test --force
```

A process that is already running in the workspace is not stopped. That is what lets a harness
delete the workspace it is standing in and still finish; the folder disappears underneath it and
the command returns. The branch is there either way. `ys` steps out of the folder before removing
it. On Windows a folder that is still some other program's current directory — the agent or shell
that launched the command — cannot be removed, and nothing is deleted.

### `ys session list`

Agent conversations. Running ones by default; `--all` includes those that have ended, and
`--workspace <name>` narrows it to one.

```
ID        WORKSPACE                 HARNESS  STATE    TITLE
6051993d  fix-the-flaky-login-test  claude   running  fix the flaky login test
```

The state column compares two things, because they can disagree:

| State      | Meaning                                                                                                                             |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `running`  | The record says running and the daemon confirms the process is there.                                                               |
| `gone`     | The record says running but the process is not — it died with nobody watching. Yardsort settles the record the next time it starts. |
| `running?` | The record says running and there is no daemon to ask.                                                                              |
| `ended`    | The conversation finished, and the record says so.                                                                                  |

### `ys attach [target]`

Puts a running session on your terminal: the screen is repainted where it got to, and what you
type goes to the agent. **`Ctrl-]` detaches**, and the agent carries on — detaching is not
stopping, exactly as closing the Yardsort window is not stopping.

```sh
ys attach                      # when only one thing is running
ys attach fix-the-flaky-login-test
ys attach 6051993d             # the start of an id from `ys session list`
```

With nothing named it attaches to the only running session, and asks which one you meant if there
is more than one. A shell has no workspace name, so reach it by id.

Attaching matches the session to your terminal's size, and follows it if you resize the window.
If the app has the same session open in a tab, that tab's size and yours are the same setting —
whichever changed last wins, and the app sets it again when you next focus the tab. `--no-resize`
leaves the size alone, at the cost of a program drawing to a width you cannot see.

You can attach to a session the app also has open; both see the same output, and either can type.
Only sessions that are still running can be attached to.

### `ys logs [target]`

Prints what a session has on its screen, without taking the terminal over — including sessions
that have **finished**, which is the one thing `attach` will not do.

```sh
ys logs                        # the only session, if there is one
ys logs fix-the-flaky-login-test
ys logs --lines 20             # just the end of it
ys logs --json                 # the text as a JSON field, for scripts
ys logs --raw                  # escape sequences and colours, exactly as the app would repaint
```

Without `--raw` the screen is replayed through a terminal emulator and handed back as plain text,
so it can be read, grepped or piped. Scrollback comes too, as far back as the session kept it.

**Screens live in the background process, not in the database.** It stops once nothing is
connected and nothing is running — so a finished agent's last screen is readable while Yardsort
is open, or while other agents are still working, and gone a short while after the last of them
stops. `ys logs` says so rather than reporting an empty screen. Nothing is written to disk: a
screen can hold anything the agent printed, and Yardsort does not keep a copy of that.

### `ys activity list`

The newest recorded [activity](activity.md): when each agent, shell or run command started in a
workspace and how it ended, newest first. `--workspace <name>` narrows it to one workspace,
`--limit <N>` changes how many rows (50 by default), and `--json` prints the same rows as a JSON
array with the event's full payload.

```
WHEN                  WORKSPACE      EVENT            SOURCE              DETAILS
2026-09-24 20:44:15Z  fix-the-login  process.exited   yardsort/lifecycle  exit 0, via spool
2026-09-24 20:44:02Z  fix-the-login  tool.completed   claude/hook         Edit src/login.rs, 12 ms
2026-09-24 20:43:59Z  add-a-footer   usage.reported   codex/session_file  29842 tokens
2026-09-24 20:43:41Z  rename-the-cli tool.completed   opencode/plugin     write src/main.rs
2026-09-24 20:43:12Z  tidy-the-tests approval.resolved grok/session_file  run_terminal_command, allow, waited 4210 ms
2026-09-24 20:43:05Z  add-a-footer   turn.completed   omp/extension       19963 tokens
2026-09-24 20:42:50Z  rename-the-cli file.reported_write cursor/hook      src/main.rs
2026-09-24 20:43:58Z  fix-the-login  process.started  yardsort/lifecycle  claude, opus, reporting via hook
```

Rows from `claude/hook`, `codex/session_file`, `opencode/plugin`, `grok/session_file`,
`omp/extension`, `pi/extension` and `cursor/hook` are what the agents themselves reported or
recorded, when
[capture](activity.md#what-claude-code-reports) is on for them; the rest is what Yardsort saw.

Any `ys` command that reads the daemon first takes the exits it kept while nothing was
connected into the database, so a `session list` after an agent finished on its own says `ended`
rather than `gone`.

### `ys activity export`

Every recorded event as **NDJSON** on stdout — one JSON object per line, oldest first, with the
same fields `--json` shows. `--workspace <name>` narrows it. Pipe it wherever you like; nothing
else exports it.

### `ys doctor`

Where `ys` is looking and whether it can get there: the data directory, the database, the daemon,
which agents are on your `PATH`, how much activity is recorded, and where the exit spool and the
agents' inbox are and whether anything is waiting in them. The first thing to run when `ys` and
the app seem to disagree.

## Profiles

`ys` looks for the same data directory the app uses. `--data-dir <dir>` points it at another one,
which is how you reach a throwaway profile:

```sh
ys --data-dir /tmp/ys project list
```

`YARDSORT_DATA_DIR` works too, as it does for the app.

`ys` never creates a database. If it cannot find one it says which path it tried, rather than
starting an empty one and telling you that you have no projects — see
[Troubleshooting](troubleshooting.md).
