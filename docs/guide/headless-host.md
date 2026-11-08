# A computer with no screen

> **Still under development, like the rest of [Remote](remote.md).** Expect it to change. From the
> next release, each one carries a tarball of it for Linux; until then, build it from source, as below.

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

Each release carries `yardsort-host-<version>-linux-x86_64.tar.gz` and
`yardsort-host-<version>-linux-aarch64.tar.gz`, from the
[releases page](https://github.com/joaoh82/yardsort/releases). Each holds `yardsort-host`, `ys`, a
systemd user service and a README. They are built on Ubuntu 22.04, and run where its glibc does,
2.35 or later: Ubuntu 22.04, Debian 12 or anything newer. Install them for the user that will run
them:

```sh
tar -xzf yardsort-host-<version>-linux-x86_64.tar.gz
cd yardsort-host-<version>-linux-x86_64
install -Dm755 bin/yardsort-host bin/ys -t ~/.local/bin/
install -Dm644 systemd/yardsort-host.service -t ~/.config/systemd/user/
```

`~/.local/bin` must be on that user's `PATH` in its login shell's profile, as it is by default on
Ubuntu and Debian once the folder exists: agents, and you, find `ys` by it.

Or build both from a checkout of the repository, with a current Rust:

```sh
cargo build --release -p yardsort-host -p yardsort-cli
install target/release/yardsort-host target/release/ys ~/.local/bin/
```

`scripts/host-package.sh` builds the same tarball a release carries.

Run them as an ordinary user, never as root. That user must own, or be able to write to, the
repositories you add, and its home holds the agents' own sign-ins. To run the host in a container
instead, see [In a container](#in-a-container).

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
paired devices reach it. If the network goes, it keeps trying by itself. Try it under `tmux` first;
then run it as a service, below.

**Stopping it (`Ctrl-C`, or `SIGTERM`) stops none of the agents.** It closes its link to the server
and its control socket, and exits. The agents run in Yardsort's terminal daemon, a process of its own,
and the next `serve` finds them still running. They stop when they finish, when you end them, or when
the computer shuts down.

Only one program runs on a profile at once: `serve` will not start while the app, or another `serve`,
has it, and the app will not start while `serve` has it.

### As a service

The tarball's `yardsort-host.service` is a systemd **user** service: it runs as you, not as root.
Sign in first, then:

```sh
systemctl --user daemon-reload
systemctl --user enable --now yardsort-host
sudo loginctl enable-linger "$USER"
```

The last line keeps your services running with nobody logged in, and starts them when the computer
does; without it, the host stops when your last SSH login ends. `journalctl --user -u yardsort-host`
shows what it prints.

The service is set to `KillMode=process`, so stopping or restarting it stops `serve` alone and leaves
the daemon and the agents running: `systemctl status` then still lists them under the service, which
is expected. A service of your own needs the same line, or stopping it ends the agents with it. To
sign in again, or sign in as another name, stop the service first, and start it after.

`yardsort-host status` says whether the host is running, whether it is signed in, how its link to the
server stands, how many devices are let in, and how many terminals are running.

## In a container

The repository has an image for the host, [`crates/host/Dockerfile`](../../crates/host/Dockerfile),
and an example of running it, [`crates/host/compose.yaml`](../../crates/host/compose.yaml). Build it
from the repository's root; no image is published yet.

```sh
docker build -f crates/host/Dockerfile --build-arg UID="$(id -u)" --build-arg GID="$(id -g)" -t yardsort-host .
```

- **It runs as an ordinary user**, `yardsort`, under `tini`, which reaps what the agents leave behind.
  The two build arguments give that user your own ids, so the repositories you mount are its, and what
  the agents write in them is yours. Without them, its ids are 1000.
- **No agent is in it**, and none of their sign-ins. Build an image of your own `FROM yardsort-host`
  that installs the agents you use, under their licences, and mount their sign-ins into
  `/home/yardsort`.
- **Its volumes**: `/data` the profile, with `settings.toml` in it; `/credentials` the host's sign-in;
  `/worktrees` the workspaces; and your repositories wherever you mount them, at the path you add them
  by. Keep each its own volume.
- **It publishes no port**: the host only reaches out. Never give it the Docker socket, which is root
  on the computer it runs on.

With the compose example, from `crates/host/`:

```sh
HOST_UID="$(id -u)" HOST_GID="$(id -g)" docker compose build
docker compose run --rm host sign-in https://remote.example.com --name "Build box"
docker compose up -d
docker compose exec host yardsort-host pair
docker compose exec host ys project add /repos/my-service
docker compose exec host ys workspace new my-service "Fix the flaky login test"
```

Sign in by pasting the address back, as above: forwarding the port does not reach into the container.
`pair` from `docker compose exec` is a terminal of your own, as an SSH login is.

**Stopping or restarting the container ends the agents**, since the terminal daemon lives inside it.
Their workspaces, worktrees and records stay in the volumes; start the agents again with `ys`, or
from the phone.

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

## Upgrading

Replace the two programs and restart `serve`:

```sh
install -Dm755 bin/yardsort-host bin/ys -t ~/.local/bin/
systemctl --user restart yardsort-host
```

The agents keep running. A terminal daemon of the same kind is kept and reused; one from an earlier
release keeps its agents running until they end, while new ones start in a new daemon, and
`yardsort-host status` counts both. In a container, build the image again and recreate the container,
which ends the agents.

## Backing up

What to keep:

- **The profile** (`~/.local/share/dev.yardsort.app`, or `/data` in the container): the database,
  with your projects, workspaces and devices.
- **`settings.toml`**, which on Linux is in `~/.config/dev.yardsort.app` unless the profile was named
  with `--data-dir` or `YARDSORT_DATA_DIR`, when it is in the profile. See
  [Where things are](#where-things-are).
- **Your repositories and the worktree root**, as you would anyway.

**Leave the sign-in out**, or at least do not count on it. It changes each time the host renews it,
and a restored copy older than the last renewal looks stolen to the server, which then signs this
computer out. After restoring, run `yardsort-host sign-in` again: it keeps the same host, with its
devices still paired. A backup of the sign-in is also a copy of this computer's identity, readable by
anyone who can read the backup.

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

Where `XDG_RUNTIME_DIR` is not set, as in a shell from `sudo -iu` or `su`, the sockets are looked for in
`/run/user/<uid>`, the folder logind keeps for that user, so that such a shell and the systemd service
reach the same host and the same daemon. A daemon that an earlier release started in such a shell is
listening at `daemon.sock` in the profile; it is still looked for there, and used for as long as it runs,
so upgrading does not start a second one beside it.

Give `ys` the same profile as `serve`, with the same `YARDSORT_DATA_DIR` or `--data-dir`, or it looks at
another one.
