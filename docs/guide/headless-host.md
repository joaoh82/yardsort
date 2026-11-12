# A computer with no screen

> **Still under development, like the rest of [Remote](remote.md).** Expect it to change. Each release
> since 0.23.0 carries a tarball of it for Linux. From the next release, each also publishes two container
> images of it; see [In a container](#in-a-container).

`yardsort-host` runs Yardsort on a computer with no display, such as a server you reach over SSH, as a
host your phone reaches through a Yardsort server. You start agents there with [`ys`](cli.md), or from
the phone; you watch them and type into them from the phone. They keep running on that computer
whatever happens to the connection, to the phone, or to `yardsort-host` itself.

It is a separate program from the app and from `ys`, with no window and nothing to do with WebKit or
GTK. Nothing it runs listens on the network: it reaches out to the server, as the app does.

## What it needs

- **Linux.** It builds on macOS too, but pairing a device needs Linux, where it can check who is asking
  (see [Only a person can pair a device](#only-a-person-can-pair-a-device)). There is none for Windows.
- **A Yardsort server** you can sign in to, and a browser on any other computer to sign in with. To run
  one of your own, see the
  [self-host recipe](https://github.com/joaoh82/yardsort/tree/main/crates/server/self-host).
- **`git`**, and the agents you want to run, installed and signed in on that computer as the user who
  runs `yardsort-host`. Their sign-ins and their licences are yours to see to.
  [`yardsort-host install-agent`](#installing-an-agent) installs one for you. The full container image
  comes with the agents whose licences allow it.

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

Each release publishes two images, for x86_64 (amd64) and arm64, in the GitHub Container Registry. Each
is tagged with the release's version and with `latest`:

- **`ghcr.io/joaoh82/yardsort-host-full`**, the one to start from. It has the tools agents use: Node.js,
  Bun, Go, Rust, Python, the GitHub CLI, `git`, `ripgrep`, `jq` and the build basics. It also has the
  agents whose licences let anyone hand them on: Codex CLI, OpenCode, Pi and Oh My Pi. Claude Code,
  Cursor's CLI and Grok are not in it: [`install-agent`](#installing-an-agent) installs those from each
  vendor, into the home volume, where a new container finds them.
  `/usr/share/doc/yardsort-host-full/NOTICE.md` in the image lists what is in it, and under which licence.
- **`ghcr.io/joaoh82/yardsort-host`**, the host alone, with `git`, to build an image of your own on.

The example [`crates/host/compose.yaml`](../../crates/host/compose.yaml) runs the full image. Copy it, and
from its folder:

```sh
docker compose pull
docker compose run --rm host sign-in https://remote.example.com --name "Build box"
docker compose run --rm host install-agent claude
docker compose up -d
docker compose exec host claude          # sign Claude Code in, once
docker compose exec host yardsort-host pair
docker compose exec host ys project add /repos/my-service
docker compose exec host ys workspace new my-service "Fix the flaky login test"
```

`YARDSORT_VERSION=0.30.0 docker compose pull` takes a given release rather than `latest`. Sign in by
pasting the address back, as above: forwarding the port does not reach into the container. `pair` from
`docker compose exec` is a terminal of your own, as an SSH login is.

- **It runs as an ordinary user**, `yardsort`, under `tini`, which reaps what the agents leave behind.
  In the published images its ids are 1000. If your repositories' owner has other ids, build the images
  yourself, as below, so that what the agents write in them is yours.
- **Its volumes**: `/data` the profile, with `settings.toml` in it; `/credentials` the host's sign-in;
  `/worktrees` the workspaces; `/home/yardsort` the agents' home, with what `install-agent` installed,
  their sign-ins and their conversations; and your repositories wherever you mount them, at the path
  you add them by. Keep each its own volume. The home volume holds the agents' sign-ins: guard it as
  you would `/credentials`.
- **It publishes no port**: the host only reaches out. Never give it the Docker socket, which is root
  on the computer it runs on.

**Stopping or restarting the container ends the agents**, since the terminal daemon lives inside it.
Their workspaces, worktrees and records stay in the volumes, and so do their conversations, which
the agents keep in the workspaces and your home: [resume them](#after-a-restart) with `ys`.

**Checking an image.** Each is signed with Sigstore by the release workflow, with no key to keep, and
carries an SBOM and the build's provenance. With [cosign](https://docs.sigstore.dev/cosign/):

```sh
cosign verify ghcr.io/joaoh82/yardsort-host-full:0.30.0 \
  --certificate-identity-regexp '^https://github.com/joaoh82/yardsort/.github/workflows/release.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

**Building them yourself**, from the repository's root, the slim image first, with your own ids:

```sh
docker build -f crates/host/Dockerfile --build-arg UID="$(id -u)" --build-arg GID="$(id -g)" -t yardsort-host .
docker build -f crates/host/Dockerfile.full --build-arg SLIM=yardsort-host -t yardsort-host-full .
```

Then set `image: yardsort-host-full` in the compose file. To add something of your own, build `FROM`
either image.

## Installing an agent

```sh
yardsort-host install-agent claude      # or cursor, grok, codex, opencode, pi, omp
```

It installs an agent into `~/.local`, as you, or brings one up to date. Run it as the user that runs
`serve`, since that is where the agents it starts look; it refuses to run as root.

- **Claude Code, Cursor's CLI and Grok** come from each vendor's own installer: `https://claude.ai/install.sh`,
  `https://cursor.com/install` and `https://x.ai/cli/install.sh`. It names the vendor and where their
  terms are before it runs one. Installing one is accepting those terms; Yardsort hands on no copy of them.
  `--installer <file>` runs an installer you downloaded and read yourself instead.
- **Codex CLI, OpenCode, Pi and Oh My Pi** come from npm, as their newest release. The full image has
  them already, at the versions its release pinned; this is how to take a newer one between releases.
  It needs `npm`, from Node.js 22 or later. Oh My Pi also needs Bun.

`~/.local/bin` comes first on the full image's `PATH`, so what you install there is what the agents run.
On a computer of your own it must be on your login shell's `PATH`, as Ubuntu and Debian put it once the
folder exists; `install-agent` says when it is not. Sign the agent in as its own guide says, as the same
user, before starting it with `ys`.

## Projects and agents

On a computer with no screen, [`ys`](cli.md) is how you add projects and start agents:

```sh
ys project add ~/src/my-service
ys workspace new my-service "Fix the flaky login test"
ys attach                        # watch it here; detaching leaves it running
```

A paired device that may type can start work too: a new workspace, another agent in one, or a
conversation that ended taken up again, as [Remote](remote.md#starting-work-from-the-phone) says, from
the phone or from the desktop app on another computer
([Reaching another computer](remote.md#reaching-another-computer)). It offers the agents this profile
has turned on and can find.

## After a restart

What a restart does to the agents depends on what restarted:

| What restarted                                      | The agents                                               |
| --------------------------------------------------- | -------------------------------------------------------- |
| The service, `serve` alone, or an upgrade           | Keep running. The phone reaches the same computer again. |
| The computer, the container, or the terminal daemon | End with it. Their conversations can be resumed.         |

The host started again settles what ended: `ys session list --all` shows those agents as `ended`. To carry on with one, name it by the start of the id that list prints:

```sh
ys session list --all
ys session resume 6051993d      # the same conversation, in a new terminal
ys session fork 6051993d        # a copy of it that goes its own way
```

Either keeps running after the command returns, and the phone sees it as it would any agent. A paired
device that may type can resume one too, with **Resume** beside it. A
resume picks up the conversation where the agent left it, as **Resume** does in the app, so it works
only for an agent that can resume one by its id or the most recent one in its folder; `ys` says why
when one cannot.

When an agent there asks for a permission, a phone that allows notifications gets **An agent needs
you**, and opening it opens that agent's terminal; see [Notifications](remote.md#notifications).

**Workflows, smoke tests, schedules and triggers do not run on a headless host yet.**
`ys workflow run`, `ys test run` and `ys schedule run` say so and queue nothing, rather than queue work
nobody would carry out, and `ys trigger` says its triggers will not fire.

## Only a person can pair a device

```sh
yardsort-host pair
```

Run in a terminal of your own on that computer, an SSH login say, it shows a QR code and a code to
type, good once for five minutes. Enter it on the phone. When the phone asks, `pair` shows its name
and what it asks for. When the phone asked to see workflow runs, watching includes reading them.
It asks you: **Allow viewing**, **allow viewing and typing**, or **deny**. If you allow it, and
the phone asked to approve or answer workflows, it then asks whether it may also do that. With
more than one project, it then asks which ones it may reach,
all of them if you just press Enter. Nothing is let in without your answer.

The device can be a phone, or the desktop app on another computer, signed in under **Settings → Remote →
Other computers** ([Reaching another computer](remote.md#reaching-another-computer)). The app asks to
watch and to type, as a phone does, and `pair` asks the same questions it asks a phone.

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
`yardsort-host status` counts both. In a container, pull the new image, or build it again, and recreate
the container, which ends the agents: `docker compose pull && docker compose up -d`. What
`install-agent` installed stays in the home volume.

## Backing up

What to keep:

- **The profile** (`~/.local/share/dev.yardsort.app`, or `/data` in the container): the database,
  with your projects, workspaces and devices.
- **`settings.toml`**, which on Linux is in `~/.config/dev.yardsort.app` unless the profile was named
  with `--data-dir` or `YARDSORT_DATA_DIR`, when it is in the profile. See
  [Where things are](#where-things-are).
- **Your repositories and the worktree root**, as you would anyway.
- **In a container, the home volume**, `/home/yardsort`, if you want the agents' conversations back. It
  also holds their sign-ins, so a backup of it is a copy of those: keep it as you would keep them.
  What `install-agent` put there can be installed again instead.

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

| What                     | Where                                                                                                                                                                                                                                                        |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The profile              | `--data-dir`, else `YARDSORT_DATA_DIR`, else `~/.local/share/dev.yardsort.app`: the database, the daemon's log, the activity inbox and spool                                                                                                                 |
| `settings.toml`          | `YARDSORT_CONFIG_DIR` when that is set, unless `--data-dir` named the profile. Otherwise in the profile when `YARDSORT_DATA_DIR` is set. Otherwise, on Linux, `$XDG_CONFIG_HOME/dev.yardsort.app`, else `~/.config/dev.yardsort.app`; elsewhere, the profile |
| The sign-in              | `YARDSORT_CREDENTIALS_DIR`, else `credentials` in the profile                                                                                                                                                                                                |
| Worktrees                | `~/yardsort`, or `YARDSORT_WORKTREE_ROOT`, or the setting in `settings.toml`, as for the app                                                                                                                                                                 |
| `serve`'s control socket | `$XDG_RUNTIME_DIR/yardsort/<tag>.host.sock`, else `host.sock` in the profile; only its user can reach it                                                                                                                                                     |
| The terminal daemon's    | `$XDG_RUNTIME_DIR/yardsort/<tag>.sock`, else `daemon.sock` in the profile                                                                                                                                                                                    |

Where `XDG_RUNTIME_DIR` is not set, as in a shell from `sudo -iu` or `su`, the sockets are looked for in
`/run/user/<uid>`, the folder logind keeps for that user, so that such a shell and the systemd service
reach the same host and the same daemon. A daemon that an earlier release started in such a shell is
listening at `daemon.sock` in the profile; it is still looked for there, and used for as long as it runs,
so upgrading does not start a second one beside it.

Give `ys` the same profile as `serve`, with the same `YARDSORT_DATA_DIR` or `--data-dir`, or it looks at
another one.
