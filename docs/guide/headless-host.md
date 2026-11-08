# A computer with no screen

> **Still under development, like the rest of [Remote](remote.md).** Expect it to change. There is no
> download of it yet: build it from source, as below. Packages and a container image come next.

`yardsort-host` runs Yardsort on a computer with no display, such as a server you reach over SSH, as a
host your phone reaches through a Yardsort server. You start agents there with [`ys`](cli.md), or from
the phone; you watch them and type into them from the phone. They keep running on that computer
whatever happens to the connection, to the phone, or to `yardsort-host` itself.

It is a separate program from the app and from `ys`, with no window and nothing to do with WebKit or
GTK. Nothing it runs listens on the network: it reaches out to the server, as the app does.

## What it needs

- **Linux.** It builds on macOS too, but pairing a device needs Linux, where it can check who is asking
  (see [Only a person can pair a device](#only-a-person-can-pair-a-device)). There is none for Windows.
- **A Yardsort server** you can sign in to, and a browser on any other computer to sign in with.
- **`git`**, and the agents you want to run, installed and signed in on that computer as the user who
  runs `yardsort-host`. Installing them, their sign-ins and their licences are yours to see to: none
  comes with Yardsort.

## Installing

Until there are packages, build both programs from a checkout of the repository, with a current Rust:

```sh
cargo build --release -p yardsort-host -p yardsort-cli
install target/release/yardsort-host target/release/ys ~/.local/bin/
```

Run them as an ordinary user, never as root. That user must own, or be able to write to, the
repositories you add, and its home holds the agents' own sign-ins.

To try it on a throwaway cloud server first, [A test box for Remote](../test-box.md) makes one and
builds it there for you.

## Signing in

```sh
yardsort-host sign-in https://remote.example.com --name "Build box"
```

`--name` is what phones call this computer; it defaults to its host name. The command prints an
address. Open it in a browser on any computer and sign in. The browser is then sent to an address that
starts `http://127.0.0.1:<port>/signed-in`, which goes nowhere on that computer: copy the whole address
from its address bar, paste it into the waiting command, and press Enter. A wrong address is said, and
asked for again.

Or, before opening the address, forward the port it names from the computer with the browser, and the
sign-in finishes by itself:

```sh
ssh -L <port>:127.0.0.1:<port> build-box
```

The address you paste holds a one-time code that is worth nothing without a secret `sign-in` keeps to
itself, so passing it through a clipboard gives nothing away.

Sign in while `yardsort-host serve` is stopped: one sign-in held by two programs gets it revoked at
the server, so `sign-in` refuses while anything runs on the profile. Signing in again later, as the
same name or another, keeps this computer as the same host, with its devices still paired.

**The sign-in is this computer's identity at the server.** It is kept in a folder only its user can
read: `credentials` in the profile, or the folder `YARDSORT_CREDENTIALS_DIR` names. Anyone who can read
that folder can act as this host, so keep it out of backups others can read, and out of images.

## Running the host

```sh
yardsort-host serve
```

It runs until stopped, and does what the open app does for this computer's terminals: keeps their
records true, takes in what agents report as they work, and keeps the link to the server's relay so
paired devices reach it. Run it under `tmux`, or as a service of your own; one is coming with the
packages.

**Stopping it (`Ctrl-C`, or `SIGTERM`) stops none of the agents.** They run in Yardsort's terminal
daemon, a process of its own, and the next `serve` finds them still running. If you run it as a
systemd service, set `KillMode=process`, or stopping the service ends the daemon and the agents with
it. The agents stop when they finish, when you end them, or when the computer shuts down.

Only one program runs on a profile at once: `serve` will not start while the app, or another `serve`,
has it, and the app will not start while `serve` has it.

`yardsort-host status` says whether the host is running, whether it is signed in, how its link to the
server stands, how many devices are let in, and how many terminals are running.

## Projects and agents

On a computer with no screen, [`ys`](cli.md) is how you add projects and start agents:

```sh
ys project add ~/src/my-service
ys workspace new my-service "Fix the flaky login test"
ys attach                        # watch it here; detaching leaves it running
```

A paired device that may type can start work too: a new workspace, or another agent in one, as
[Remote](remote.md#starting-work-from-the-phone) says. It offers the agents this profile has turned
on and can find.

**Workflows, smoke tests, schedules and triggers do not run on a headless host yet.**
`ys workflow run`, `ys test run` and `ys schedule run` say so and queue nothing, rather than queue work
nobody would carry out, and `ys trigger` says its triggers will not fire.

## Only a person can pair a device

```sh
yardsort-host pair
```

Run in a terminal of your own on that computer, an SSH login say, it shows a QR code and a code to
type, good once for five minutes. Enter it on the phone. When the phone asks, `pair` shows its name
and what it asks for, and asks you: **Allow viewing**, **allow viewing and typing**, or **deny**. With
more than one project, it then asks which ones it may reach, all of them if you just press Enter.
Nothing is let in without your answer.

`pair` is refused from any terminal Yardsort started, and from anything an agent runs: the host checks
who is at the other end of its socket, and refuses a process that carries a launched session's
markers, descends from the terminal daemon, or types at one of the daemon's terminals, and one whose
input is not a terminal at all. **This is a guard, not a wall.** A program running as the same user can
read everything the host can, its sign-in included. What the guard stops is an agent letting a device
in by accident, or because something it read told it to: nothing an agent is given can approve a
device.

```sh
yardsort-host devices            # the devices let in
yardsort-host revoke "My phone"  # by its name, or the id `devices` shows
```

Taking a device's access away needs no check: it is always safe. Its channel closes at once.

## Signing out

```sh
yardsort-host sign-out
```

Removes this computer at the server, signs its device out there, and forgets the kept sign-in. Every
paired device loses it. Works whether `serve` is running or not.

## Where things are

| What                     | Where                                                                                                                                                                               |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The profile              | `--data-dir`, else `YARDSORT_DATA_DIR`, else `~/.local/share/dev.yardsort.app`: the database, the daemon's log, the activity inbox and spool                                        |
| `settings.toml`          | In the profile when `--data-dir` or `YARDSORT_DATA_DIR` is set. Otherwise, on Linux, `$XDG_CONFIG_HOME/dev.yardsort.app`, else `~/.config/dev.yardsort.app`; elsewhere, the profile |
| The sign-in              | `YARDSORT_CREDENTIALS_DIR`, else `credentials` in the profile                                                                                                                       |
| Worktrees                | `~/yardsort`, or `YARDSORT_WORKTREE_ROOT`, or the setting in `settings.toml`, as for the app                                                                                        |
| `serve`'s control socket | `$XDG_RUNTIME_DIR/yardsort/<tag>.host.sock`, else `host.sock` in the profile; only its user can reach it                                                                            |
| The terminal daemon's    | `$XDG_RUNTIME_DIR/yardsort/<tag>.sock`, else `daemon.sock` in the profile                                                                                                           |

Give `ys` the same profile as `serve`, with the same `YARDSORT_DATA_DIR` or `--data-dir`, or it looks at
another one.
