# The `ys` command line

`ys` drives Yardsort from a terminal. It reads the same database as the app, so a workspace made
with `ys` appears in the window and a workspace made in the window appears in `ys`. It does not
need the app to be running.

An agent started by `ys` belongs to the [daemon](terminals-and-sessions.md#agents-keep-working-when-you-close-the-window),
not to the command — so it keeps working after `ys` returns, and you can open Yardsort later to
watch it. That is the whole point of it.

## Installing

`ys` is a separate download from the app: a single binary, in the assets of each
[release](https://github.com/joaoh82/yardsort/releases). Put it anywhere on your `PATH`.

```sh
# Linux
tar -xzf ys-*-linux-x86_64.tar.gz && sudo install ys /usr/local/bin/

# macOS (universal)
tar -xzf ys-*-macos-universal.tar.gz && sudo install ys /usr/local/bin/
```

On Windows, unzip `ys-*-windows-x86_64.zip` and put `ys.exe` in a folder on your `PATH`.

On macOS `ys` is neither signed nor notarized — unlike the app, which is both. A copy downloaded
with a browser is therefore quarantined and refused on first run. Either download it with `curl`,
or clear the flag:

```sh
xattr -d com.apple.quarantine ys
```

No package manager ships `ys` yet, and it does not update itself — the app's
[updater](updates.md) does not know about it.

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

### `ys doctor`

Where `ys` is looking and whether it can get there: the data directory, the database, the daemon,
and which agents are on your `PATH`. The first thing to run when `ys` and the app seem to disagree.

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
