# The host dashboard

> **New in this release, and part of [Remote](remote.md), which is still under development.**
> Expect it to change.

The host dashboard is a web page a computer serves to itself, for looking after it as a host
from a browser: how it stands, the agents installed on it, Git and GitHub, and the commands that
set it up. It is what you would otherwise do over SSH, a command at a time, after
[installing `yardsort-host`](headless-host.md#installing).

![The host dashboard's Overview: the host signed in as Build box with its link connected, the computer's processors, memory and disk as bars, and two projects](../images/host-dashboard.png)

It is not another Yardsort. There are no workspaces, terminals or conversations in it, and it
starts no agent: that is the [phone's and the desktop's](remote.md). It does not need Remote
either, so a server that is not signed in to anything yet serves it all the same.

- **On a computer with no screen**, `yardsort-host serve` serves it unless you turn it off.
- **In the app**, it is off until you turn it on under **Settings → Remote → Host dashboard**.

## Who can reach it

**It listens on that computer alone**, on `127.0.0.1`, port 28680. Nothing on the network can
connect to it. From another computer you reach it one of two ways: a port forwarded over SSH, or
a [tunnel](#a-tunnel) you turn on.

**A browser must sign in**, with a one-time link that only that computer hands out, and only to
a person: at a terminal of your own there, or in the app's window. There is no password to
guess. A link is good once, for ten minutes; the browser then stays signed in for thirty days.

**Whoever signs in can run commands on that computer, as the user the host runs as.** That is
what it is for. Treat a sign-in link as you would treat a shell on that computer.

It lets no device in, approves no [memory](memory.md) note, turns no sync on, and neither exports
nor deletes your account. Those stay at a
terminal of your own, or in the app, so that nothing an agent could reach can do them, and a
command you run from the dashboard is refused them too: `yardsort-host pair` typed into
**Commands** does not pair a device.

## Opening it on a computer with no screen

Log in to that computer over SSH, as the user that runs the host, and ask for a way in:

```sh
yardsort-host dashboard
```

```
Dashboard: http://127.0.0.1:28680
Tunnel: off

Sign a browser in, within 10 minutes, by opening:

  http://127.0.0.1:28680/#code=7K2M-9QXD-4TRA-B6WN

Or open the address alone and type the code: 7K2M-9QXD-4TRA-B6WN

From another computer, without a tunnel, forward the port first:
  ssh -L 28680:127.0.0.1:28680 <this computer>
```

That address is the server's own. To open it in the browser in front of you, forward the port
from your computer, in another terminal, and then open the link there:

```sh
ssh -L 28680:127.0.0.1:28680 build-box
```

`yardsort-host dashboard` is refused from any terminal Yardsort started, from anything an agent
runs, and from a script, as [`pair`](headless-host.md#only-a-person-can-pair-a-device) is, and
like `pair` it needs Linux. `yardsort-host status` says where the dashboard is and how many
browsers are signed in, and needs no terminal.

## Opening it from the app

Open **Settings**, then **Remote**, and under **Host dashboard** tick **Serve the host
dashboard**. The section then says where it is served.

- **Open the dashboard** opens it in this computer's browser, signed in.
- **Copy a sign-in link** puts a one-time link on the clipboard, for a browser elsewhere. It is
  this computer's own address unless the tunnel is up, so from another computer forward the port
  first, as above.
- **Sign every browser out** ends every session at once.

Unticking the box stops serving it, and it stays off the next time Yardsort starts.

## What is in it

### Overview

How the host stands: the program and its version, the user it runs as, whether Remote is on and
its link to the server, how many devices are let in, and how many terminals are running. Beside
it, the computer: its system, how busy its processors are, its memory, and how full the disks
under your home and your workspaces are. Below, the projects there.

### Agents

The agents Yardsort can start on that computer, as its login shell finds them: where each is and
its version, or that it is not there.

**Install** puts one into `~/.local`, as that user, and **Update** brings one up to date. It is
what [`yardsort-host install-agent`](headless-host.md#installing-an-agent) does: Claude Code,
Cursor's CLI and Grok come from each vendor's own installer, and the page names the vendor and
where their terms are, and asks, before it runs one; Codex CLI, OpenCode, Pi and Oh My Pi come
from npm. What the installer prints is shown as it runs. Installing is offered on Linux and
macOS.

**An agent still has to be signed in**, once, as its own guide says, at a terminal on that
computer: the dashboard has no terminal.

### Git

![The dashboard's Git tab: a name and email for commits, and a GitHub sign-in under way, its job showing the one-time code and the address to type it at](../images/host-dashboard-git.png)

- **Commit identity**: the name and email that computer's commits carry.
- **GitHub**: whether the [GitHub CLI](https://cli.github.com) is signed in, in its own words.
  **Sign in with a browser** shows a one-time code and an address: open the address in any
  browser and type the code, and nothing has to open on the server. **Sign in with a token**
  hands that computer less than your whole account, as
  [Git and GitHub](headless-host.md#git-and-github) describes; the token goes straight to `gh`
  and is kept by `gh` alone. **Sign out here** signs `gh` out; the token is still good at GitHub
  until you delete it there.
- **Repositories**: give a repository's address and **Clone** clones it with that computer's own
  Git sign-in and adds it as a project. It goes into `~/Projects`, or the folder
  [`projects_dir`](headless-host.md#projects-and-agents) names.

### Commands

**Run a command** runs a line as that computer's shell would, from the home folder, as the user
the host runs as, and shows what it prints as it runs. **Stop** ends it, and everything it
started.

**Nothing is on a command's input.** One that asks a question gets no answer and fails, so pass
`-y`, or whatever the command has for that: `sudo -n apt-get install -y ripgrep`, not
`sudo apt-get install ripgrep`.

**A command is not you at a terminal.** `yardsort-host pair`, `yardsort-host dashboard`,
`yardsort-host account export` and `account delete`, and the `sync` and `memory` commands that [only a person may run](headless-host.md#only-a-person-can-pair-a-device)
are refused to a command the dashboard ran, and to anything that command starts.

**Jobs** lists what has been run since the host last started, an agent's install and a GitHub
sign-in included, each with its output. They are kept in memory: restarting the host forgets
them, and stops any still running.

### Access

The dashboard's addresses, its [tunnel](#a-tunnel), and how many browsers are signed in. **Sign
every browser out** signs out this one too, and voids the links not yet used.

## A tunnel

A [rustunnel](https://github.com/joaoh82/rustunnel) tunnel gives the dashboard an `https://`
address that needs no SSH. The host runs rustunnel's client, which reaches out to a tunnel
server; nothing new listens on that computer.

> **Not yet run through a real tunnel.** This part was built against rustunnel's documented
> client and tested with a stand-in for it. If the real one behaves otherwise, the dashboard says
> what the client said, under **Access**.

**Read this before turning one on:**

- **Anyone who has the address can reach the sign-in page.** Only a sign-in link from that
  computer lets them further.
- **The tunnel's server can read everything that passes through it**: what the page shows, what
  you type into it, and the cookie that keeps you signed in. The connection is encrypted as far
  as that server, not end to end. Use a tunnel server you trust with a shell on that computer,
  or [one of your own](https://github.com/joaoh82/rustunnel).

It needs the `rustunnel` client installed for the user that runs the host, with its token:

```sh
rustunnel setup          # asks for a region and your token; keeps them in ~/.rustunnel/config.yml
```

Yardsort keeps no token of its own. Then, at a terminal of your own on that computer:

```sh
yardsort-host dashboard tunnel on --subdomain build-box
yardsort-host dashboard            # shows the tunnel's address once it is up, and a link through it
yardsort-host dashboard tunnel off
```

`--subdomain` asks for a name of your choosing; without one the server picks one, new each time
the tunnel starts. `--region` is `auto`, `eu`, `us` or `ap`, and `--server` names a rustunnel
server of your own as `host:port`; without them, what `rustunnel setup` chose is used. Turning a
tunnel on is refused to anything but a person at a terminal, as a sign-in link is. Turning it
off never is.

The same settings are on the page, under **Access**, for a browser that is already signed in,
and in the app that is where they are. The page asks before giving the dashboard a public
address, and before changing a tunnel you are using to reach it.

The tunnel is kept up while the host runs, and comes back by itself after a lost connection. If
the client fails, for a wrong token say, **Access** and `yardsort-host dashboard` say why in the
client's own words, and it is tried again at longer and longer waits, up to five minutes. Through
the tunnel the dashboard answers over HTTPS only.

## Turning it off

```sh
yardsort-host dashboard off        # and `on` again, at a terminal of your own
yardsort-host dashboard sign-out   # sign every browser out, and leave it on
```

`off` stops serving it at once, ends its tunnel, and keeps it off when the host next starts. In
the app, untick **Serve the host dashboard**. Either writes it to
[`settings.toml`](settings.md), which you can also edit while the host is stopped:

```toml
[dashboard]
enabled = false      # leave it out for each program's own default
port = 28680         # another port, if you would rather

[dashboard.tunnel]
enabled = true
subdomain = "build-box"
region = "eu"
server = "tunnel.example.com:4040"
```

## In a container

The dashboard is served inside the container, on the container's own `127.0.0.1`, which a
published port does not reach. Until that changes it can be used there only through a tunnel,
from an image of your own with `rustunnel` in it. This has not been tried yet.

## If something is wrong

- **"That code is not one this computer gave…"** The link was used already, is more than ten
  minutes old, or every browser was signed out since. Ask for another.
- **The port is taken.** The dashboard takes the next free one of the ten from 28680, and
  `yardsort-host status` and the app's Settings say which. If all ten are taken, the host runs
  without it and says so when it starts: set another `port`.
- **The page says "This computer is not answering."** The host stopped, or the forward or the
  tunnel dropped. It keeps trying, and carries on when the host is back; a browser stays signed
  in across a restart.
- **The tunnel is "not up".** The line says why, in the client's own words when it gave any. No
  `rustunnel` on that user's `PATH`: install it. No token, or a wrong one: `rustunnel setup`.
  When the host runs as a service, the client is found on the `PATH` that user's login shell
  has, as agents are.
- **"Signing in to the dashboard needs a person at a terminal"**, or **"… is refused from a
  terminal Yardsort started"**, or **"… is refused to a command the dashboard ran"**. You ran
  `yardsort-host dashboard` from a script, an agent's terminal or the dashboard's own
  **Commands**. Run it in an SSH login of your own. Turning the dashboard on and opening a
  tunnel are refused the same way, each named for what was asked. On a computer that is not
  Linux it says it needs Linux, where the host can check who asks.
- **"Signed out for now, but … could be neither written nor deleted"** The profile's
  `dashboard` folder cannot be changed, so the sign-out holds only until the host restarts. Fix
  the folder's owner or the full disk, or delete the file it names.

## Where things are

| What                 | Where                                                                              |
| -------------------- | ---------------------------------------------------------------------------------- |
| Who is signed in     | `dashboard/sessions.json` in the profile, by a hash of each session, never its key |
| Its settings         | `[dashboard]` in `settings.toml`                                                   |
| The tunnel's sign-in | rustunnel's own: `~/.rustunnel/config.yml`, or `RUSTUNNEL_TOKEN`                   |
| What was run         | Nowhere: jobs and their output are kept in memory, and go when the host stops      |

How it was decided, and what is left open: [the design record](../design/47-host-dashboard.md).
