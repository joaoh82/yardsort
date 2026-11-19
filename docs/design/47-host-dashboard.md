# 47. The host dashboard

Status: built on 2026-11-17, in one piece, at the owner's request, and reviewed the same day:
the review's four findings are fixed, and are in [After the review](#after-the-review). Merged as
#277. On 2026-11-19 the owner answered its six open questions, and each was built. Pairing and
signing in to a server from the page came first, as #285: [Remote from the page](#remote-from-the-page).
The other four are in [The owner's other answers](#the-owners-other-answers-2026-11-19): a
terminal in the page, a record of what was done, and rustunnel alone, in the full image. Its
tests pass, its page was looked at in a real browser against a stand-in host, and
`yardsort-host` was run with it on a throwaway profile. **Not yet run through a real tunnel, on
a real server, or by the owner.** What was and was not checked is in [Checked](#checked).

## What was asked

After [#267](https://github.com/joaoh82/yardsort/pull/267) made a server one line to install,
what is left is done over SSH, a command at a time: install an agent, give Git a name, sign `gh`
in, clone the repositories, run whatever else the server needs. The owner asked for a web page
for that, in these words:

1. `yardsort-host` comes with a web dashboard to manage the server, install agents, set Git up
   easily, run setup commands and see the server's status.
2. It is not another UI for Yardsort, only a host management UI. Once the host runs, it serves a
   web interface on some port and makes it available through a tunnel, ideally rustunnel with a
   subdomain.
3. The desktop app can turn the same web server on in Settings, and has it off by default.
4. The headless server has it on by default.
5. It runs on a port that is not commonly used, so that it does not clash with other programs.

## What it is, and is not

A page with eight tabs: **Overview** (the host, the computer, the projects), **Remote**,
**Agents**, **Git**, **Commands**, **Terminal**, **Access** and **Activity**. It reads how things stand
and starts _jobs_: an agent's install, a GitHub sign-in, a clone, a command. A job's output is
shown as it comes. The first build had five of them; Remote, Terminal and Activity came with the
owner's answers: [Remote from the page](#remote-from-the-page) and
[the other four](#the-owners-other-answers-2026-11-19).

It has no workspaces, agents' terminals, conversations, pull requests or tasks, and starts no
agent. Those are the phone's and the desktop's, through Remote. It needs no Remote either: a host
that is not signed in to any server serves it all the same, which is when it is most useful, and
is signed in from it.

## Decisions

Taken before the code, by the agent that built it. The owner has not been asked about any of
them; the ones most likely to be changed are again under [Open questions](#open-questions).

### One crate, served by two programs

`crates/dashboard` (`host-dashboard`) holds the server, the sign-in, the jobs, the tunnel and the
page. It knows nothing of either program. What a program knows of itself reaches it through one
trait, `Source`: how the host stands, the launch environment, the settings, the projects, and a
clone. `yardsort-host` implements it over its `HostHome` (`crates/host/src/dashboard.rs`), the app
over its `AppState` (`src-tauri/src/dashboard.rs`).

`yardsort-host install-agent` moved with it: where each agent comes from and the programs that
install it are `host_dashboard::agents`, and the command and the page's **Install** run the same
ones, one at a terminal and one into a job.

### 127.0.0.1, port 28680

It listens on 127.0.0.1 and nowhere else. There is no setting for another address: a dashboard
that can run commands, listening on a server's public address over plain HTTP, is the mistake
this would make easy.

The port is 28680, or `port` under `[dashboard]`. It was chosen to be below every system's range
of passing ports (32768 up on Linux, 49152 up on macOS and Windows), so that no outgoing
connection is ever found holding it; registered to nothing in the services list; and not one of
the round numbers development servers take. When it is taken all the same, the next free one of
the ten from it is used, and `serve`, `status`, and the page all say which. When all ten are
taken the host runs without the dashboard and says so.

### Signing in: a one-time code, handed out only where a person is

A browser signs in with a code: sixteen characters, 80 bits, good once, for ten minutes. A code
buys a session, kept by the browser as a cookie for thirty days.

- **On a host**, `yardsort-host dashboard` prints the address and a link with a code in it. It
  is behind the same guard as `pair` (`crates/host/src/guard.rs`): only a person at a terminal of
  their own on that computer, never an agent, a script, or a terminal Yardsort started.
- **In the app**, the window makes one: **Open the dashboard** opens the browser signed in, and
  **Copy a sign-in link** is for a browser elsewhere.

There is no password. A password is something to guess at leisure, to reuse, and to store; a
code that lapses in ten minutes needs no rate limit to be safe from guessing, and there is
nothing to keep but sessions. A session is kept by the SHA-256 of its token, in
`<profile>/dashboard/sessions.json`, readable by its user alone, so that a restart, which every
upgrade is, signs nobody out, and the file is no session to whoever reads it. The file is
written with the sessions locked, so that two changes at once cannot leave the older of them in
it. A sign-out that cannot be written deletes the file instead, which signs everyone out at the
next start; when it can be neither written nor deleted, the sign-out says so, since a restart
would then undo it.

Signing in with the Yardsort server's account was considered and left out: the dashboard has to
work with Remote off, which is exactly when a new server needs it, and a second use of the
identity provider is a cost per sign-in ([28](28-remote-infrastructure.md)).

The link carries its code after the `#`, which a browser sends to nobody. The page takes it out
of the address before it does anything else.

### What the dashboard runs is not a way past the operator's guard

**Changed on 2026-11-19.** As first built, nothing the guard keeps for a person at a terminal
could be done from the dashboard at all. The owner has since chosen that the page itself lets a
device in and signs the host in to a server ([Remote from the page](#remote-from-the-page)).
What this section says still holds for everything the dashboard _runs_: a job, and now a
terminal. It no longer holds for the page's own Remote tab.

The dashboard runs commands as the host's user, so a signed-in browser can do what that user
can at a shell, with one exception that is the point of this section: what the operator's guard
keeps for a person at a terminal. Approving a note, turning sync on, and opening the dashboard
itself up are not in the dashboard, and they and `yardsort-host pair` are refused to a command
the dashboard runs.

Leaving them out of the page was not enough, and the first build stopped there. A command typed
into **Run a command** is a child of the host, not of the terminal daemon, and carries none of a
launched session's markers; given a terminal of its own with `script`, it was exactly what the
guard takes for a person. The review showed it: such a command was handed a new sign-in code,
and pairing sits behind the same check. So the guard now also refuses:

- **a process that carries the mark every dashboard job is given**, `YARDSORT_DASHBOARD_JOB`,
  which a job's programs and everything they start inherit; and
- **a process that descends from the host itself**, which every job does, mark or no mark.

This matters because of who can reach the dashboard. An agent on the same computer can reach
127.0.0.1, and as the same user can read and write the sessions file; with jobs let through,
getting into the dashboard would have been a way round the guard for it. As it is, the guard is
as strong against a dashboard job as against an agent's terminal, and no stronger: a job that
sheds both its mark and its ancestry gets past, as an agent that sheds its own does. It is a
guard, not a wall, here as there ([35](35-remote-r4-r5.md)).

So the sign-in protects the computer from the network, and the guard still protects pairing
from the agents, and from what the dashboard runs. Since 2026-11-19 it does not protect pairing
from the dashboard's own page: whoever is signed in can pair there, by the owner's choice.

### What the server takes from a request

Every request passes one check first (`crates/dashboard/src/http.rs`, `guard`), in this order:

1. **The name it was sent to** must be `127.0.0.1`, `localhost` or `[::1]`, at any port, or the
   tunnel's own host name. Any port, since an SSH forward listens on whichever port was free at
   the other end. This is what stops a page on another site reaching the dashboard under a name
   of its own that it has pointed at 127.0.0.1.
2. **Through the tunnel, HTTPS only.** rustunnel's server also answers plain HTTP for a tunnel.
   A request that came that way is sent to the HTTPS address if it only reads, and refused if it
   carries anything. The scheme is read from `X-Forwarded-Proto`, which the tunnel's server sets
   over whatever the request carried, and is believed only for a request addressed to the
   tunnel.
3. **A request that changes something** must carry the header `X-Yardsort-Dashboard: 1`, which
   no form or link can add and no other site's script can add without asking first, and must
   not say it comes from another site. The cookie is `SameSite=Strict` as well.
4. **Everything under `/api`**, but whether the browser is signed in and the sign-in itself,
   needs a session.

The cookie is `HttpOnly`, and `Secure` when it was given through the tunnel. Its name ends in a
few characters that are that dashboard's alone, because a browser keeps one jar of cookies for
127.0.0.1 whatever the port, and two servers reached through two forwarded ports would otherwise
sign each other out. The page is served with a content security policy that allows its own
three files and nothing else, and may not be framed.

### Jobs

A job is programs run in turn, or a piece of the host's own work, with everything printed kept
as text (`jobs.rs`). A program is started as everything in Yardsort is: an argv array, in the
environment the login shell resolved, in a process group of its own so that **Stop** ends what
it started too. Nothing is read out of a job's output; it is only shown.

- **Nothing is on a program's input.** A command that asks a question fails rather than waits.
  The one exception is a secret that must be in no command line: a GitHub token goes to `gh` on
  its input, and is in no argument, no title and no output.
- **A command is the one place a line is handed to a shell.** **Run a command** gives the line
  to `sh -c`, or to `cmd.exe /C` on Windows. The rule that programs are never spawned through a
  shell string is about what Yardsort decides to run; here the line is the person's own and
  running it as their shell would is the feature. The shell itself is still an argv array.
- **Jobs live in memory**: the last thirty, four running at once, the newest half megabyte of
  each one's output. A host that stops ends the ones still running rather than leave them
  writing to nobody.

### Agents

The list is the harnesses in force (`harness::resolve_all`), each looked for on the login
shell's `PATH` and asked for its version, once per change of its program. **Install** and
**Update** are offered for the seven Yardsort knows how to install, on Linux and macOS. Before a
vendor's installer runs, the page names the vendor and where their terms are and asks, as
`install-agent` says it before running; the job says it again.

An agent still has to be signed in, once, at a terminal on that computer. See
[Open questions](#open-questions).

### Git

`git` and `gh` themselves, with nothing kept by the dashboard (`git.rs`):

- **Commit identity**: `git config --global user.name` and `user.email`.
- **GitHub**: `gh auth status` says whether it is signed in, in its own words. **Sign in with a
  browser** runs `gh auth login --web`, which prints a one-time code and an address and waits;
  both are in the job's output, and nothing has to open on the server. **Sign in with a token**
  is for handing the computer less than a whole account, and runs `gh auth setup-git` after.
- **Repositories**: **Clone** goes through the path a paired device's clone already takes
  (`remote::start`), into `remote.projects_dir`, and adds the project.

### The tunnel

Turned on, the host runs the `rustunnel` client as a child (`tunnel.rs`):
`rustunnel http <port> --local-host 127.0.0.1 --json --no-inspect`, with `--subdomain`,
`--region` and `--server` when set. Its `--json` events are its interface for programs, and they
alone say how the tunnel stands: `tunnel_ready` gives the address, `reconnecting` and `error` the
rest. The client reconnects by itself; when it ends instead, it is started again at longer and
longer waits, up to five minutes, so that a wrong token is not tried all day at full speed.

- **Yardsort keeps no token.** The client finds its own, in `~/.rustunnel/config.yml` or
  `RUSTUNNEL_TOKEN`. Nothing of the tunnel's sign-in is in `settings.toml`, in an argument or in
  the page.
- **Turning it on needs a person**, on a host: `yardsort-host dashboard tunnel on` is behind the
  guard. It is also in the page, for a browser already signed in. Turning it off needs nobody.
- **The tunnel's server can read what passes**, since TLS ends there. The guide and the page say
  so. `--server` names one of your own.
- **The client ends with the host**: when the tunnel is turned off, when the host stops, and, on
  Linux, when the host dies without stopping (`PR_SET_PDEATHSIG`), since systemd's
  `KillMode=process` would leave it otherwise.
- **A client belongs to the run of the supervisor that started it.** Turning the tunnel off and
  on starts a new run while the last may still be reading its dead client's output; it only
  ever takes its own client, and none is waited for with the lock that stopping needs.
- **What is typed for it cannot become an option**: a subdomain, a region and a server are
  checked before any of them is an argument.

The tunnel is off until asked for. It cannot be on by default: it needs an account at a tunnel
server, and a public address is not something to give a computer unasked.

### On by default on a host, off in the app

`[dashboard] enabled` is unset in a new profile, and each program has its own answer for that:
`yardsort-host serve` serves it, the app does not. `yardsort-host dashboard off` and the app's
checkbox write the answer down.

**A host upgraded to this release starts listening on 127.0.0.1:28680**, without having been
asked. That is what "on by default" means, and the changelog says it first.

### The page

Plain HTML, CSS and JavaScript, three files built into the program (`crates/dashboard/ui`). No
framework and no build step: the host's tarball gains no Node, and the page is small. It uses
Yardsort's own colours and sizes, follows the browser's light or dark setting, and folds to one
column on a phone. It asks the host every few seconds; nothing is pushed. Everything a job
prints, and every name, is set as text and never as markup. The terminal added later is the one
part drawn by a library, xterm.js, kept beside the page; what that did to the page's policy is
[below](#a-terminal-in-the-page-question-2-yes).

## Settings

```toml
[dashboard]
enabled = true     # unset: on for yardsort-host, off for the app
port = 28680

[dashboard.tunnel]
enabled = true
subdomain = "build-box"          # unset: the server picks one, new each time
region = "eu"                    # auto, eu, us or ap; unset: as rustunnel is set up
server = "tunnel.example.com:4040"   # a rustunnel server of your own
```

## Where it is

| What                                         | Where                                                      |
| -------------------------------------------- | ---------------------------------------------------------- |
| The server, its guard and the page's answers | `crates/dashboard/src/http.rs`                             |
| Codes and sessions                           | `crates/dashboard/src/auth.rs`                             |
| Jobs                                         | `crates/dashboard/src/jobs.rs`                             |
| Terminals                                    | `crates/dashboard/src/terminal.rs`, `ui/vendor/`           |
| Remote from the page, and the QR code's dots | `crates/dashboard/src/remote.rs`                           |
| What was done                                | `crates/dashboard/src/activity.rs`                         |
| Agents and how each is installed             | `crates/dashboard/src/agents.rs`                           |
| Git and the GitHub CLI                       | `crates/dashboard/src/git.rs`                              |
| The tunnel                                   | `crates/dashboard/src/tunnel.rs`                           |
| The computer's load, memory and disks        | `crates/dashboard/src/system.rs`                           |
| The page                                     | `crates/dashboard/ui/`                                     |
| The host's part, and its commands            | `crates/host/src/dashboard.rs`, `control.rs`, `main.rs`    |
| The host signing in and pairing for the page | `HostRemote` in `crates/host/src/dashboard.rs`             |
| The app's part, and Settings                 | `src-tauri/src/dashboard.rs`, `HostDashboardSettings.tsx`  |
| A stand-in host, to look at the page         | `cargo run -p host-dashboard --example dashboard-stand-in` |
| The settings                                 | `DashboardSettings` in `crates/core/src/settings.rs`       |

## Checked

- **The crate's tests** (`crates/dashboard`): codes and sessions; jobs with real processes,
  stopped with their children; the tunnel against a stand-in `rustunnel` that prints its events;
  Git in a home of its own; and `tests/http.rs`, the dashboard served for real on 127.0.0.1 and
  asked over real HTTP: who is refused (no session, another name, another site, plain HTTP
  through the tunnel), and that an agent is installed by a stand-in vendor's installer, a token
  reaches a stand-in `gh` on its input, a command runs and is stopped, and sessions outlast a
  restart.
- **The host's tests** (`crates/host/tests/dashboard.rs`): the real `yardsort-host serve` on a
  profile of its own serves it, says where, and refuses every command that would open it up
  when asked by something with no terminal.
- **The real host, by hand**, on a throwaway profile, through the steps of its smoke test
  (`.yardsort/tests/host-dashboard.yaml`, which waits for the owner's approval to run as one):
  `serve` said where the dashboard was; a script was refused a link; a terminal of the
  operator's own was given one, which signed in once; the dashboard ran a command as the host's
  user; `dashboard off` left nothing listening.
- **The page**, in headless Chromium against the stand-in host: the sign-in by link, every tab,
  at a desk's width and a phone's.
- **The app's Settings section**, through Testing Library.
- **The real app, by hand**, a development build on a throwaway profile with the dashboard
  turned on in its settings, driven through the window's inspector: it served the dashboard as
  it started; a link from the window signed in; the dashboard named the app as its host, found
  the agents, Git and `gh`, and ran a command; **Sign every browser out** ended the session;
  turning it off stopped it listening and turning it on served it again; closing the window let
  the port go.

Not checked, and said in the guide where it matters:

- **A real tunnel.** The tunnel was tested against a stand-in for rustunnel's client that prints
  its documented events. What rustunnel's server does to a request, that it keeps the `Host` it
  was asked for and sets `X-Forwarded-Proto` itself, was read in its source
  (`crates/rustunnel-server/src/edge/http.rs`) and not seen. The agent that built this was not
  allowed to open a tunnel from the owner's computer, rightly: it is the owner's to try.
- A real server, with systemd: that the client ends with the service, and that the dashboard
  comes back with it.
- macOS and Windows. It was built and run on Linux alone; CI compiles and tests it on the other
  two. The page's **Install** is not offered on Windows, nor for anything but the seven agents.
- A container. See below.

## After the review

A second agent read the whole change the day it was built and reproduced four faults. All four
were right, and are fixed, each with a test that fails without its fix.

1. **Restarting the app from a command panicked.** Leaving stopped the dashboard by blocking on
   the async runtime, and an update's restart and **Replace the daemon** reach that from
   commands, which run on the runtime: Tokio panics rather than block there, with the dashboard
   off too. The restart now awaits the stop, and leaving from one of the runtime's own threads
   drops the dashboard rather than wait (`src-tauri/src/dashboard.rs`).
2. **Sessions signed out at once could come back after a restart.** Each sign-out wrote a copy
   of the sessions taken before the others had finished, through one shared temporary file. See
   _Signing in_, above (`auth.rs`).
3. **A command the dashboard ran could pass the operator's guard**, by giving itself a terminal.
   See _A session in the dashboard is not a way past the operator's guard_, above
   (`crates/host/src/guard.rs`, `jobs.rs`). The test runs it on the real host.
4. **A tunnel turned off and on could not then be stopped.** The supervisor of the first client
   took the second's and waited for it, holding the lock. See _The tunnel_, above (`tunnel.rs`).

## Remote from the page

Added on 2026-11-19, when the owner answered questions 3 and 4: the host should do most if not all
of what the clients do. A **Remote** tab, after Overview.

**What it does.** On `yardsort-host`, what `sign-in`, `sign-out` and `pair` do at a terminal:

- **Signing in**, while Remote is off: the server and the name, then the sign-in page's address to
  open, and the address the browser lands on pasted back, or the port forwarded, as `sign-in`
  takes them. Run by `serve` itself, which holds the profile, so there is no stopping it first;
  the link starts at once (`account::enable`, then `account::connect`). The sign-in's verifier
  stays in the host's process; the page holds the address and the code it pastes, which is worth
  nothing without it.
- **Giving a sign-in up** stops it while it waits for its browser, and not after: once the
  code is in hand the host redeems it, registers itself and keeps the sign-in, and stopping
  between those would leave a host at the server that nothing here knows of. So from there it
  runs to its end, **Cancel** says that it does, and another sign-in is not begun over it. The
  check and the code's taking are one step under the sign-in's lock, so one given up a moment
  earlier redeems nothing. As first built the task was aborted wherever it was, registering
  included. A second build of this tab, made the same day on the branch that added the
  terminal and since dropped for this one, did the opposite and never stopped a sign-in once
  begun; a review reproduced that one signing in after its cancel, with a server slow to
  answer. The rule here is the answer to both.
- **Signing out**, after the page asks: `account::disconnect` and `account::disable`, as the
  control socket's `signOut`.
- **Pairing**: an offer, its code and its QR code, drawn by the page from the modules the host
  sends (`Pairing::Offered.qr`, a row of `0` and `1` each); the first device that asks, with what
  it asks for and the projects to choose among; and the operator's answer, the same choices as
  `pair` gives, sent with the device's grant id. An answer for any other grant is refused.

**The interface.** `host_dashboard::Remote`, given by `Source::remote`. The app gives none: its
window does all three, so the tab says to use **Settings → Remote** and every route answers so.
Routes: `GET /api/remote`; `POST /api/remote/pair`, `/pair/answer`, `/pair/stop`,
`/sign-in`, `/sign-in/pasted`, `/sign-in/stop` and `/sign-out`. Each needs a session, the page's
header and no other site's `Origin`, as every other change does. The host's side is
`crates/host/src/dashboard_remote.rs`.

**Who it lets pair.** Question 3 said the dashboard's sign-in would become part of what guards
pairing. It does, and nothing more was put in front of it: a browser signed in there already runs
any command as the host's user, which can read the host's sign-in and do everything pairing does
by hand. A second check would guard nothing that first one does not open. What still holds:

- **No job pairs.** A command the dashboard runs is refused `pair` by the guard, as before, and
  holds no session cookie to call the page's routes with; sessions are kept by their hash.
- **No agent pairs**, for the same two reasons: an agent's terminal is refused by the guard, and
  has no session.
- **Nothing is let in unanswered**, and the answer names the device, as at a terminal.

**Proof.** `crates/dashboard/tests/http.rs`, on a stand-in `Remote`: refused signed out and from
another site's page; an offer with its code and QR code; the device asking, an answer for another
device refused, and the answer chosen passed through; signing in by the address pasted, a wrong
one refused, and signing out; and the app's case, every route saying where to go instead. The
host's QR rows checked in `dashboard_remote.rs`. The smoke test `host-dashboard-remote`, against
the stand-in server with a real `yardsort-host`: signed in from the page after a wrong address,
linked with no restart, `pair` refused to a job, a phone paired from the page and reaching the
computer, signed out. Its steps passed by hand on 2026-11-19. The page was looked at in a browser
against the dashboard's stand-in example (`--remote`, `--remote-off`), in each state.

## The owner's other answers, 2026-11-19

The six questions this was left with were put to the owner once it had merged, and all six were
answered. Questions 3 and 4 are [Remote from the page](#remote-from-the-page), above. These are
the other four. Each is a decision now.

### A terminal in the page (question 2: yes)

**Terminal** is a shell in a pseudo-terminal: the user's own login shell (`$SHELL -l`, or
`COMSPEC` on Windows), in the home folder, with the login shell's environment, as an SSH login
would be. It is for what a job cannot do, answer a question: an agent's sign-in, `sudo` with a
password, `rustunnel setup`.

- **It is the dashboard's, like a job.** It carries the job's mark and is the host's own child,
  so the guard refuses it what it refuses a job. A real terminal on its input changes nothing:
  the guard never took a terminal alone for a person.
- **In memory, and bounded.** Four open at once; the newest megabyte of each one's output is
  kept, so a browser that comes back, or another one, sees what was printed meanwhile. A
  terminal ends when it is closed, when every browser is signed out, and when the host stops:
  a shell nobody can reach is not left running. Closing hangs up on the shell and lets go of
  the pseudo-terminal, and a second and a half later kills what is left: the shell's process
  group, and the group it had in the foreground when it was closed, which the pseudo-terminal
  names and which job control keeps apart from the shell's. That is done whether or not the
  shell went, and before the shell is waited for (`waitid` with `WNOWAIT` sees it go), so that
  its id, which is its session's, is still its own and tells its foreground group from a
  stranger's that took the number. As first built the kill was skipped once the shell had
  gone, and a foreground program deaf to a hang-up outlived its terminal; a review found it.
  A background job that ignores a hang-up is left, as an SSH login leaves it, and when the
  host stops there is only the hang-up.
- **The page asks; nothing is pushed.** Output is read by a request that waits up to twenty
  seconds for there to be any, and what is typed goes up one request at a time, in order. That
  keeps the server at HTTP/1.1, through a tunnel as well as a forward, at the cost of a round
  trip a keystroke.
- **Drawn by xterm.js**, the library the app's own terminals use, kept as three files beside the
  page (`ui/vendor`), since the dashboard has no build step and is compiled where there is no
  `node_modules`. A test fails when they are not the ones the app installs.
- **The page's policy changed for it.** xterm.js writes style sheets into the page and colours
  single characters with a `style` attribute, both of which `style-src 'self'` refuses. So the
  page is now served with a nonce, new each time, that its script gives to the style sheets made
  there (`style-src 'self' 'nonce-…'`), and `style` attributes are allowed
  (`style-src-attr 'unsafe-inline'`), which reach no further than their own element. Scripts are
  as they were: the dashboard's own files and nothing else. Looked at in Chromium: no violation,
  and true colour drawn.
- **Nothing typed or shown is recorded**, anywhere.

### A record of what was done (question 6: yes)

**Activity** lists what was done through the dashboard, the newest first: sign-ins and refused
ones, each job's start and end, each terminal opened and ended, the tunnel and Git's identity
changed, and what the Remote tab did: a sign-in to a server started and come through, a
sign-out, a pairing code shown, a device let in or turned away. The last 500, in
`dashboard/activity.json` in the profile, mode 0600, across restarts.

- **A sentence each, never output.** No job's output, no sign-in or pairing code, no token. A
  command is recorded as typed, so a secret typed into one is in it, as in a shell's history;
  the guide says so.
- **The same thing in a row is one line, counted**, and is written to the file at most every
  five seconds: anyone who can reach the sign-in page can try a wrong code, and must not be able
  to fill the record or make the host write a file per try.
- **A record, not a control.** Whoever is signed in has a shell and can delete it. It is for the
  operator to read.

### rustunnel, and only rustunnel (question 5), and in the full image (question 1)

- **No other tunnel and no proxy of one's own.** The dashboard answers to a loopback name or to
  the host name its own rustunnel client reported, and to nothing else; that stays. The page
  says so, and links to [rustunnel.com](https://rustunnel.com) for the client and an account.
- **The full image carries the `rustunnel` client**, pinned by version and checksum like the
  rest of it (0.8.9, AGPL-3.0, unmodified, listed in its notice). A container's own 127.0.0.1 is
  not reached by a published port, so a tunnel is how a container's dashboard is opened: give it
  `RUSTUNNEL_TOKEN`, then `docker exec -it … yardsort-host dashboard tunnel on`. The slim image
  stays as it was, and the images still publish no port.

### Checked, for these

- **The crate's tests**: a terminal that is a real shell, answers a question, resizes, wakes a
  waiting reader, and ends with what it ran when closed and when every browser is signed out,
  a program deaf to the hang-up included; the page's nonce; the activity, without its secrets,
  across a restart, and what the Remote tab did in it, a sign-in recorded once.
- **The host's test**: on the real `yardsort-host`, `pair` and `dashboard` typed into the page's
  terminal are refused, and the refusal of `pair` names the Remote tab.
- **The page**, in headless Chromium against the stand-in host: the terminal typed into as a
  keyboard would, and the Activity tab.

Not checked: **a real tunnel**, still; the full image **built** with rustunnel in it (CI's
image job does that; here only the pinned download was run and its checksum matched); the
terminal on macOS and Windows, which CI only compiles.

## Open questions

Also in [06-open-questions](06-open-questions.md). The six this was first left with are settled,
above. What they leave:

1. **A real tunnel has still not been run.** It is the owner's to try, or to ask for on the
   test box.
2. **The app's dashboard does not manage Remote.** Its Settings do. If the app's dashboard is
   ever used from another computer for that, `Source::remote()` is where it would be given.
3. **A terminal does not outlast the host.** An agent's terminal in Yardsort does, in the
   daemon. Putting the dashboard's there would make it one of Yardsort's terminals, which a
   paired device sees; kept apart on purpose.
4. **The Remote tab does not list the devices let in, or take one off.** `yardsort-host devices`
   and `revoke` do, and run from **Commands**. The Overview counts them.
