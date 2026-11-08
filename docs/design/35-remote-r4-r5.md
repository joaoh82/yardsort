# Remote, R4 and R5: a computer with no screen, the desktop as a client, and the phone hardened

**Status:** planned on 2026-11-06. The first slice, the host's account moved out of the app, was
built the same day (§10); its local smoke tests and the owner's phone check against staging passed.
The second, `yardsort-host`, was built on 2026-11-07 (§11); its local smoke test and the owner's run on a box against staging passed. The third, its packaging, was built on 2026-11-08 (§12); its local smoke test, run from the tarball with an empty environment, and the owner's run of it as a service on the box, passed. Checked on
2026-11-08 against where Remote is going, several machines an account owns and the desktop choosing
where a task runs (§3, "Where agents run"); what comes after R5 is in §9. This is the plan for the last two
phases before sync: what each has to show, what it starts from, the decisions taken before any
code, the slices in the order they are built, and what each slice's smoke test needs. Like
[31](31-remote-r3.md), it becomes the record of the slices as they are built. The proposal is
[25](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md); R0 and R1 are in [26](26-remote-r0-r1.md),
whose ledger (§8) says which phase is where; R2 is in [27](27-remote-r2.md), R3 in
[31](31-remote-r3.md).

**Why R4 and R5 are planned together.** The proposal lets R4 be tested privately while R5 is
built (§13). The owner's priority on 2026-11-06 is to run Yardsort on a Linux computer with no
screen and reach it, from the phone first and then from the desktop app. So the slices are ordered
for that: R5's headless host comes first, then the desktop as a client, then R5's restart and
resume work, and R4's hardening slices after and around them. Each phase keeps its own gate.

## 1 · What the two phases have to show

**R5, headless and desktop remote** (proposal §11, §13). The gate:

- **the same session is usable from both clients**: one agent's terminal, watched from the phone
  and from the desktop app on another computer, with control passing between them and the host;
- **the host works without a GUI**: on a Linux computer with no display, no WebKit and no
  desktop session, started from a shell or a service manager;
- **persistent data survives a service restart**: the host's identity, its sign-in, its grants,
  its projects, workspaces and recorded sessions, and a terminal still running.

In R5, a person can:

- install a host program on a Linux box, sign it in from another computer's browser, and start it
  as a service, without a display;
- pair a phone with it from an SSH session, approving at the box;
- add a repository there and start an agent there, from that SSH session (`ys`), and watch and
  control it from the phone;
- sign the desktop app in as a client too, pair it with that host, and see the host's workspaces,
  sessions and terminals in the desktop app, named as that host's, and watch or control them;
- restart the host's service, or the box, and find it the same computer, still paired, with the
  agents it was running still running, or resumable where the restart ended them.

R5 does **not** include starting a new agent, resuming one, browsing files or writing to Git from a
remote client. Each needs a scoped API of its own (proposal §11), and none comes from exposing local
commands. One of them, starting and resuming an agent remotely, is planned as R5's last slice (§4,
slice 7), after the gate, because the gate does not need it. Part of it came first, from real use
on 2026-11-06: the phone starts a workspace or another agent, under `terminal:control`, in protocol
minor 5 ([31 §19](31-remote-r3.md#19--after-first-real-use-scrolling-starting-work-pictures)).
Slice 7 keeps the rest: a scope of its own if starting should be granted apart from typing, resume,
and the desktop as a client, in minor 6.

**R4, mobile beta hardening** (proposal §13). The gate:

- **lock/resume and network changes pass** on the phones;
- **generic push works**: a notification with no content reaches a locked phone, and opening it
  shows the current state;
- **offline host state is honest**;
- **no payload leaks**: no terminal output, input, title, path or memory in logs, push payloads,
  caches or diagnostics.

And three items moved from R3's device run on 2026-11-04 ([31 §18](31-remote-r3.md#18--open-after-this-plan)):
input to its echo at p95 under 300 ms at 100 ms of round trip, a usable screen within 5 seconds of
coming back to the foreground, and an external keyboard.

R4 does **not** include store listings, external testers or billing (R7), or a Yardsort-operated
push gateway for self-hosted servers (§3, "Push").

## 2 · Where it starts

Checked against `main` at `b374ca5` on 2026-11-06. No open pull request or branch touches this
work.

**Everything a headless host needs is already Tauri-free, except four things.** The daemon, the
store, the launcher, hook and inbox ingestion, workflow and schedule logic, the `HostFacade`, the
outbound connector and the file credential store live in crates that do not link Tauri. What is
missing is glue, each piece of it a `tauri::AppHandle` or `AppState` wrapper around core functions:

| Piece                                                       | Where it is today                                                                                                                                                                                                                                                                          | What a headless host needs                                                             |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| The host's sign-in, session, pairing and grants             | `src-tauri/src/remote.rs`: `Remote` (`:69`), `connect` (`:77`), `AppAccess` and `ensure` (`:115`, `:989`), `AppServing` (`:153`), `asked` (`:199`), and the commands `remote_enable`, `remote_pair`, `remote_requests`, `remote_answer`, `remote_revoke`, `remote_disable` (`:547`–`:899`) | The same, without Tauri                                                                |
| Draining the activity inbox as it fills                     | `src-tauri/src/activity.rs`: `watch_inbox` (`:116`), `drain_inbox` (`:31`), on a `notify` watcher                                                                                                                                                                                          | The same loop. Without it, nothing records a headless agent's events until `ys` drains |
| A daemon's events (an exit, the spool)                      | `src-tauri/src/lib.rs:424`–`:464`: `import_spool` on each `Exited`, `import_inbox` at start                                                                                                                                                                                                | The same handling                                                                      |
| Resume and fork                                             | `src-tauri/src/sessions.rs`: `continue_session` (`:126`)                                                                                                                                                                                                                                   | The ledger's prerequisite: in the core, so `ys` and the host can resume after restart  |
| The threads that drive workflows, schedules and smoke tests | `src-tauri/src/workflows/mod.rs` (`start` `:73`, `AppHands` `:129`), `src-tauri/src/schedules/mod.rs`; the logic is the core's (`workflow::driver::Driver::tick`, `schedule::fire::tick`)                                                                                                  | Not in the first milestone (§3, "One copy")                                            |

**The daemon.** `yardsort --yardsort-daemon <socket> [<spool>]` (`crates/core/src/daemon.rs:22`, `:35`).
The app and `ys` both call `run_daemon_and_exit_if_asked` first thing, and either starts it from
its own executable under `<data dir>/daemon.lock`, detached with `setsid`. One socket per data
directory (`pty_ipc::Endpoint::for_data_dir`): `$XDG_RUNTIME_DIR/yardsort/<tag>.sock`, else
`<data dir>/daemon.sock`. A box with no login session may have no `XDG_RUNTIME_DIR`, which lands on
the second, still private. Protocol 3 (`pty_ipc::PROTOCOL`; [26](26-remote-r0-r1.md) §2 still says 1,
which is history). It exits on its own ten seconds after its last client goes, if nothing runs.

**The `yardsort` binary cannot be the headless host.** It links WebKitGTK and GTK dynamically, so it
does not even load on a box without them. Its early exits (print-env, daemon, hook, `main.rs:7`–`:11`)
run before any display code, but only once those libraries are installed. `ys` links none of them,
and already handles `--yardsort-daemon` and `--yardsort-hook`; it does not handle `--yardsort-print-env`,
so a session `ys` starts falls back to the process's own environment rather than the login shell's.

**Signing a host in.** OIDC authorization code with PKCE only (`crates/server/src/oidc.rs`). The
host's client listens on `127.0.0.1:<port>`, and the server sends the browser back there
(`remote-client` `SignIn`, `:297`). The server has no device authorization grant (RFC 8628).

**Approving a device.** Only the host's own device credential can decide (`own_host`,
`crates/server/src/pairing.rs:313`), and a refresh credential works once: presenting a spent one
revokes the device (27 §3). So only the one process holding the host's credential can approve, and
an operator's command has to ask that process, not hold a copy. Today that process is the window,
and the guide says so: "Nothing but this window can say yes: not `ys`, and not an agent."

**The desktop is never a client today.** It signs in only as `DeviceKind::Host` and opens only host
tickets. The server already has `DeviceKind::Computer` ("another computer, as a client of hosts",
`remote-proto/src/account.rs:159`), and the server treats it as it treats a phone; nothing uses it but a
test. **The only full device client is TypeScript** (`packages/remote-client`): `Channel`,
`Attachment` and `Control` with lease renewal and resuming. The Rust `remote-client` covers HTTP and
raw relay sockets only; the one Rust device that attaches and types is ad hoc JSON in
`crates/server/examples/pair.rs`. The desktop's WebView may not reach the network (CSP
`connect-src 'self' ipc: …`), and "only `src/lib/ipc.ts` talks to the core".

**The desktop terminal has one seam.** `src/features/terminal/TerminalView.tsx` calls `ipc.ptyWrite`,
`ptyResize`, `ptyAttach` and `ptyDetach` directly (`:91`, `:174`, `:191`, `:214`); bytes arrive on a
Tauri `Channel<ArrayBuffer>`. The centre panel is chosen by flags in `src/stores/projects.ts`; there is
no router.

**Nothing exists for push**, on the server or the phone. What the host does have is a structured
signal: `agent.notified` from Claude's `Notification` hook, with `permission_prompt` or `idle_prompt`
in its detail (`crates/core/src/activity/claude.rs:208`), which devices already receive through
`activity.subscribe`. PTY quiet is activity, never "waiting for permission" (`TerminalActivity`).

**The phone app** has no `expo-notifications`, no network-change listener, no diagnostics and no
hardware key handling; its composer is a native `TextInput` and a row of keys
(`apps/mobile/src/ui/composer.tsx`). Its lifecycle (`src/ui/live.tsx`, `src/lib/presence.ts`) holds a
channel only while a screen is shown and the app is in the foreground, and retries with backoff.

**Nothing measures remote latency.** `just bench` times the desktop's own echo
([07](07-terminal-benchmarks.md)). The pretend phone in `.yardsort/tests/support/phone.ts` can pair
and reach a computer, not attach or type.

**Packaging.** One Dockerfile, the server's (`crates/server/Dockerfile`, distroless, non-root).
Releases build the desktop for three platforms and attach `ys-<v>-linux-x86_64.tar.gz`
(`.github/workflows/release.yml`). No host image or host binary.

**Stale words found on the way.** `YARDSORT_REMOTE_PREVIEW` was removed in #167, when Remote was shown
to everyone; 27 and 31's smoke steps still set it, harmlessly. They are history and stay as written.

## 3 · Decisions

**A new binary, `yardsort-host`, in a new crate `crates/host`.** Not `ys`, and not the app.

- The app cannot load without WebKitGTK. Making it start without one would mean a second build of
  it, for no gain over a small binary.
- Not `ys`: agents are told about `ys` and run it all the time, and the line "nothing in `ys`
  approves a device" is worth keeping as it is. The host is a separate program with a separate name,
  started by the operator.
- It handles `--yardsort-daemon`, `--yardsort-hook` and `--yardsort-print-env` first thing, as the
  app does, since all three re-run `current_exe()`. So it is its own daemon, and a host's daemon is
  the same build as the host.
- It is GPL-3.0-only, as the desktop and `ys` are. Linux is its supported platform. It is written so
  it builds on macOS too (the host runtime it uses is the desktop's), and its control socket is
  `cfg(unix)`; Windows is not a headless target (proposal §11), and the crate compiles to a stub
  there so `cargo build --workspace` stays green on all three.

**The host's account moves out of `src-tauri` first, as a pure move** (slice 1). Sign-in, the
session and its rotating refresh credential, registering or rebinding the host, offers, requests,
deciding and keeping grants, revoking, and the connector's `Access` and `Serving` become
`remote_host::account`, a Tauri-free module that the app and `yardsort-host` both call. It sits in
`crates/remote-host`, which already depends on the core and `remote-client` and was kept apart from
the app for this (its own `Cargo.toml` says so). Each program says where its records, settings and
credential are, and how a device asking is told to a person, through one trait (`Home`). The app
keeps its commands as thin wrappers, and its own words ("Sign in again in Settings → Remote."). The
inbox drain and the daemon's events move in slice 2, where the headless host first needs them.
Doing the move first, with nothing new in it, keeps the desktop's behaviour reviewable on its own,
and means there is one host runtime, not two.

**Signing a headless host in: from another computer's browser, by pasting the address back.**
`yardsort-host sign-in https://server --name "Build box"` prints the sign-in address. The operator
opens it on any computer and signs in. The browser is sent to `http://127.0.0.1:<port>/signed-in?code=…`,
which on that computer goes nowhere. The operator copies that address from the browser's address bar
and pastes it into the waiting command. The host checks it is its own loopback redirect, and finishes the sign-in with the verifier it kept. Also accepted, for those who prefer it: an SSH
port forward (`ssh -L <port>:127.0.0.1:<port> box`, the port printed), so the redirect arrives by
itself. Why this and not the device authorization grant: it needs **no server change**, so it works
against staging and every self-hosted server today; and the code is worth nothing without the host's
verifier, so pasting it through a clipboard gives nothing away (27 §2). A device-code flow can be added
to the server later if pasting proves awkward; it is not needed for R5.

**Approving a device on a headless host: at the box, in a terminal, by the operator.** The host
process holds the credential, so it does the deciding; the operator asks it to, over a control socket
only the host's user can reach:

- `<data dir>/host.sock`, created `0600` in the private data directory, Unix only. Never a TCP port,
  never the network. The daemon's socket is not exposed either; nothing listens on the network at all.
- `yardsort-host pair` asks the host for an offer and prints its code to type and a QR code drawn
  in the terminal, then waits. When a device asks, it shows the device's name and what it asks for,
  and asks **Allow viewing / Allow viewing and typing / Deny**, and, if the host has more than one
  project, which projects, as the window does. Nothing is decided without that answer.
- **A guard against an agent doing it.** The host refuses to approve unless the caller's standard
  input is a terminal, that terminal is not one of the daemon's, and the caller does not descend from
  the daemon or carry a launched session's markers (`YARDSORT_SESSION_RECORD_ID` and the rest). The
  host checks the caller over the socket (`SO_PEERCRED`, then `/proc`). **This is a guard, not a
  boundary:** on a headless box, a process running as the host's user can read everything the host
  can, its credential file included, and the guide says so. It is the same strength as the desktop's
  rule, where an agent could in principle drive the window: it stops an agent approving by accident,
  or because something it read told it to, and it keeps approval out of every API an agent is given.
- Revoking (`yardsort-host devices`, `yardsort-host revoke <device>`) needs no guard: taking access
  away is always safe.

**One copy per profile, and the host is it.** `yardsort-host serve` takes `presence::AppLock`, as the
app does, so the app and the host can never run on one profile at once and share a refresh
credential (the bug 0.22.0 fixed). Holding the lock tells `ys` an app is running, so `ys workflow run`,
`ys test run` and `ys schedule run` would queue runs. The first milestone's host does not drive them;
so the lock file says which kind of program holds it, and `ys` refuses to queue on a headless host
with "this host does not run workflows, smoke tests or schedules yet" instead of queuing work nobody
runs. Driving them on a headless host is later (§9), and needs nothing new from the core.

**Projects and agents on a headless host come from `ys`, over SSH.** `ys workspace new` already
starts an agent without the app (`LaunchedBy::Cli`). What `ys` lacks is adding a project; slice 2 adds
`ys project add <path>`, beside `ys project list`, with the same checks the app makes. The operator
starts agents; the phone watches and controls them, and since minor 5 starts them too. Resuming
one from a remote client, and the desktop starting one, are slice 7.

**Credentials on a headless host live in a directory** (06 #35, settled): `YARDSORT_CREDENTIALS_DIR`,
`0700` with files `0600`, from its own mount. A host with no `YARDSORT_CREDENTIALS_DIR` uses
`<data dir>/credentials` rather than an OS credential store, since a box with no desktop session has
no Secret Service. The guide says where it is and that it is the host's identity.

**Packaging: a Linux tarball first, then a container image.** Both carry `yardsort-host` and `ys`.

- The tarball is what the first milestone uses, built for x86_64 and aarch64 on the release
  workflow's oldest supported runner, so it runs on any current distribution. A systemd **user**
  unit is shipped beside it, with `KillMode=process`, so restarting the service does not end the
  agents: the daemon is its own session (`setsid`) and keeps them, and the new host attaches to it.
  `loginctl enable-linger` keeps the user's services up with nobody logged in. Shutdown: `SIGTERM`
  closes the relay link and the control socket and exits; the daemon and the agents stay. Stopping
  the agents is `ys` or the box's own shutdown.
- The container image (`crates/host/Dockerfile`): Debian slim with `git`, non-root (a UID set when
  it is built, `--build-arg UID=… GID=…`, 1000 by default, to match the repositories' owner), the host as PID 1's
  child under `tini` so the daemon's children are reaped. Mounts: the data directory, the worktree
  root, the repositories, and the credentials directory, each its own volume. No harness is in the
  image, and none of its credentials: an operator builds `FROM` it to add the harnesses they are
  licensed for, and mounts their sign-ins. **Never the Docker socket.** No port is published. A
  container restart ends the agents with it, since the daemon lives inside; the guide says so, and
  says sessions can be resumed (slice 6).
- Upgrades: replace the binary or the image and restart the service. A daemon of the same protocol
  is reused; one from an earlier protocol keeps running its agents until they end, as on the desktop
  (27 §21), and `yardsort-host status` says so.
- Backup: the data directory (the store) and `settings.toml`, which on Linux is in the config
  directory unless the profile is named by `--data-dir` or `YARDSORT_DATA_DIR`; not the
  credentials directory. The credential
  is the host's identity; a restored copy that is older than the last rotation is a spent credential,
  which the server takes for a stolen one and signs the host out. `yardsort-host sign-in` again keeps
  the same host and its devices (0.22.0's **Sign in again**). The guide says so rather than pretending
  a backup of a rotating credential can be current.

**The desktop as a client: a device of kind `Computer`, in Rust, with the network kept in the core.**

- The WebView does not reach the network, so the TypeScript client is not used there. Its channel
  layer (`Hello`, requests and replies, attach and resume, the lease and its renewal, input, resize,
  subscribe) is written in Rust in `crates/remote-client`, beside the HTTP client it already has,
  following the TypeScript one and pinned by the same fixtures in `remote-proto/fixtures/v1/`. The
  pairing tool's ad hoc device (`examples/pair.rs`) moves onto it.
- The desktop signs in a **second** device, `kind: Computer`, separate from its host device: the
  server allows only host devices to register hosts, and only device tickets to reach other hosts. Its
  refresh credential is a separate entry in the same credential store. A computer can be a host, a
  client, both or neither.
- **The other computer is always named.** Remote hosts appear in the sidebar under their own heading,
  each by its name and with its state (online, reconnecting, offline since). Opening one shows its
  workspaces with a banner across the top naming the host, and every terminal tab from it carries the
  host's name. Nothing from a remote host is mixed into the local projects.
- **A remote path is never treated as local.** No reveal in the file manager, no open in an editor,
  no Changes panel, no Git actions, no `ys` hand-off on a remote workspace: those surfaces are not
  shown for it at all, rather than shown and failing. The remote facade carries no paths anyway (R1);
  this rule keeps the desktop from inventing one.
- The terminal reuses `TerminalView` behind a small source interface (attach, write, resize, detach),
  so the remote one renders with the same xterm.js, renderer and input handling. A remote terminal is
  view-only until **Take control**, which acquires a lease, as on the phone; typing does not acquire
  one by itself, since the person at the host must be able to see who has it.

**Where agents run: on a host, and each workspace belongs to one.** Checked against the code on
2026-11-08, after the owner set out where Remote is going (several machines an account owns, the
desktop choosing where a task runs, handover between machines, managed sandboxes). What is built,
what these slices plan, and what is only proposed:

| What                                                                                                                                                                                                                                                                                                                                                                                                                          | State                                              |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| **An account owns any number of hosts.** A host is a machine that runs agents and connects out: the desktop app with Remote on (Linux, macOS, Windows) or `yardsort-host` (Linux). A VM the person runs is a host of the second kind; nothing tells it apart, and nothing needs to. The phone and a desktop signed in as `Computer` are clients: they never run an agent, and what they start runs on the host they asked     | Built (R2, slice 2); the desktop client is slice 4 |
| **Finding a host is not being let in.** `GET /v1/hosts` lists the account's hosts to any of its devices, with a name, when it was made and when it was last seen, and nothing else. Reaching one takes a grant its own person approved: the server refuses a ticket without one (`relay.rs`, `ticket`), and the host checks the grant again before it answers. Being on the account grants nothing, and no slice changes that | Built (R2)                                         |
| **Whether a host is online** is known only to a device it let in, from a channel to it being open (`apps/mobile/src/lib/presence.ts`); the server keeps no presence, and a host a device is not paired with shows as not paired, not as online or offline                                                                                                                                                                     | Built (R3); the desktop does the same in slice 4   |
| **Each workspace and session lives in one host's SQLite and nowhere else.** A client shows it from that host's facade, names the host beside it, and keeps none of it. Local work and a remote host's work are in the same app, under separate headings, never one list                                                                                                                                                       | Planned (slices 4, 5)                              |
| **Starting work on another host from the desktop**, in that host's view: a new workspace in one of its projects, or another agent in one of its workspaces, choosing the project and the agent by that host's ids, as the phone does since minor 5                                                                                                                                                                            | Planned (slice 7)                                  |
| **Choosing where a new task runs from the composer**: one task, and a choice of **This computer** or a host it is let into, with each host's state, an offline one shown and not offered                                                                                                                                                                                                                                      | Proposed, after R5 (§9)                            |
| **Handing a task to another machine**, and **managed sandboxes**                                                                                                                                                                                                                                                                                                                                                              | Proposed, deferred phases (§9)                     |

Three things follow, and they hold for every slice here:

- **The choice of machine in R5 is made by where you start.** Open a host, start work there. That is
  enough for the gate, and slice 7 is where the desktop gets it. A picker in the composer is not in
  R5: a local project and a project on another host are unrelated records on two machines, and
  nothing says they are the same repository. The only mapping planned is R6's explicit linking
  (proposal §10). A picker without it would guess from a remote URL or a folder name.
- **Execution access stays a per-host grant.** Nothing coming (a picker, handover, sandboxes) may
  let in a device because it is on the account, nor let an agent ask for one: a grant is still
  approved by the host's person (§3, approving a device), and starting work still needs the control
  a grant gives.
- **A headless host on macOS does not pair a device in R5.** `yardsort-host` builds there, but its
  guard reads `/proc`, so `pair` is refused off Linux. A Mac is a host through the desktop app, whose
  window approves. Pairing a headless Mac needs a guard of its own (§9).

**Push: APNs and FCM directly, behind a provider trait on the server, with no Expo account.** The app
uses `expo-notifications` only to ask permission and get the phone's own token
(`getDevicePushTokenAsync`). The server keeps one push route per device, sends with APNs token
authentication (a `.p8` key) and FCM HTTP v1 (a service account), and drops a route the provider says
is gone. The host sends a **notification intent** over its relay link: a session's id and a kind
(`asking` from a structured event such as Claude's `permission_prompt`, `finished` from an exit), never
text. The server deduplicates by event, coalesces, applies the device's preferences and the grant's
projects, and sends a fixed sentence: "An agent on Build box needs you" is too much (a computer's name
is the person's content); "Yardsort: an agent needs you" is the default. Tapping opens the app, which
signs in and fetches the current state; it approves nothing. PTY quiet never becomes "needs you".

**Self-hosted servers and the official app's push** (proposal §16, due at R4). Decided: option (b) of
§8 now, and (a) left for R7. A self-hosted server works fully without push; its operator who wants
push builds the app under their own identifiers with their own APNs and FCM credentials, as the README
will document. A Yardsort-operated gateway that relays generic pushes for self-hosted servers is a
managed-service feature with its own privacy review, and belongs with R7. The app says when a server
cannot send it notifications, rather than staying silent.

**Measuring the budgets.** Input to echo: a small timing tool on the TypeScript client
(`packages/remote-client/tools/echo-time.ts`) takes control of a `cat` and times 200 keystrokes to their
echo, through a local TCP delay proxy that adds 50 ms each way, reported as p50, p95 and p99. The same
tool runs against staging from this computer and from the remote box, for real round trips. On the
iPhone, the owner adds 100 ms with the Network Link Conditioner (Settings → Developer). The phone's
own figure needs the phone, so the app gains a hidden timing readout in development builds that shows
the last keystroke's round trip and the last return to the foreground's time to a live screen.

## 4 · Slices

Each slice ends with a smoke test, as in R2 and R3: locally first, as `.yardsort/tests/<id>.yaml` driven
with `ys test run`, then on staging when the slice touches the server, then whatever only the owner can
run, written out with exact paths. **Staging needs a deploy only in slices 10 and 11.** The others
change no server code.

**Milestone A: the phone reaches a computer with no screen** (slices 1 to 3).
**Milestone B: the desktop shows a host on another computer** (slices 4 and 5). That is R5's first gate
check.

| #   | Slice                                    | Phase | What it adds                                                                                                                                                                                                                                                                                                                                                                                                                   | Its smoke test                                                                                                                                                                                                                                                                                                                                   |
| --- | ---------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | The host runtime out of the app          | R5    | The move in §3: host sign-in and session, registering, offers, requests, deciding, revoking, `Access` and `Serving`, out of `src-tauri` into `remote_host::account`. The app's commands become wrappers. Nothing a person sees changes                                                                                                                                                                                         | `remote-sign-in-again` and `remote-earlier-daemon`, unchanged, pass in the real window. The phone, already paired, still watches and types through staging                                                                                                                                                                                       |
| 2   | `yardsort-host`                          | R5    | The inbox drain and the daemon's events moved into the core. `crates/host`: `sign-in` (paste or forward), `serve` (the lock, the daemon, the inbox, the connector), `pair` with its guard, `devices`, `revoke`, `status`, `sign-out`, over `host.sock`. `ys project add`. `ys` refuses to queue runs on a headless host. A guide, `docs/guide/headless-host.md`                                                                | `headless-host.yaml`, with no display variables at all: sign in against the stand-in by pasting, pair a pretend phone, refuse an approval from inside an agent's terminal, start a shell with `ys`, the phone types into it and reads the echo. Restart `serve`: same computer, still paired, the shell still running. Then the owner's run (§6) |
| 3   | Packaging the host                       | R5    | The Linux tarball for x86_64 and aarch64 in the release workflow, with the systemd user unit. The container image and a compose example. The guide's install, UID, backup, upgrade and shutdown sections                                                                                                                                                                                                                       | The tarball built here, unpacked, run with `env -i` and only `HOME` and `PATH`, passes `headless-host.yaml`'s steps. The image builds in CI only (Docker is refused here) and its own CI job runs `yardsort-host --version` and a stand-in sign-in in it                                                                                         |
| 4   | The desktop signs in as a client         | R5    | The Rust channel layer in `remote-client`, pinned by the fixtures; the pairing tool moved onto it. The desktop's `Computer` device: Settings → Remote → **Other computers**, sign in, pair by code. The sidebar's remote hosts, a host's workspaces, sessions and activity, read-only, with the banner                                                                                                                         | `desktop-client.yaml`: a `yardsort-host` on the stand-in, the real window signs in as a client, pairs with it (approved at the host's `pair`), and lists its workspace and its shell under the host's name, with no Changes panel or Git action on it                                                                                            |
| 5   | The desktop watches and controls         | R5    | `TerminalView` behind a source interface; a remote terminal attached, resumed after a drop, a new snapshot on a new epoch; **Take control**, typing, resize while in control, losing control when the phone or the host takes it                                                                                                                                                                                               | `same-session.yaml`: one shell on the headless host, watched by the pretend phone and the window; the window takes control and types, the phone sees it; the phone takes control, the window shows it lost it. Then the owner's run: the iPhone and this computer on the real box through staging (R5 gate, check 1)                             |
| 6   | Resume and fork in the core; restarts    | R5    | The ledger's prerequisite: `continue_session` and its rules into the core; the app calls it; `ys session resume` and `ys session fork`. The host after a restart of each kind: service (agents kept), box or container (agents ended, sessions resumable)                                                                                                                                                                      | `host-restart.yaml`: an agent stand-in running, restart `serve`: still running, phone reaches it; stop the daemon too: `ys session resume` brings the conversation back, the phone sees it. The desktop's own resume and fork, through the window, unchanged                                                                                     |
| 7   | Starting and resuming from a client      | R5    | A scope of its own, `session.start`, asked for by a device and shown at approval; `session.resume` in the protocol (minor 6), beside the `workspace.start` and `session.start` minor 5 already has (31 §19), choosing a harness and a workspace by id only, never a path or arguments. Phone and desktop get **Start an agent** and **Resume**                                                                                 | A device without the scope is refused; one with it starts a shell stand-in in a workspace and resumes one that ended                                                                                                                                                                                                                             |
| 8   | Lifecycle, networks and the time budgets | R4    | The phone reconnects at once on a network change (`expo-network`) rather than waiting out its backoff; the development build's timing readout; `echo-time.ts` and the delay proxy. Budgets measured and recorded with how                                                                                                                                                                                                      | Locally: p95 through the proxy at 100 ms. On staging: from here and from the box. On the iPhone (owner): 31 §17's parts N and W, and the two budgets with the Network Link Conditioner                                                                                                                                                           |
| 9   | External keyboard                        | R4    | A small native module in `apps/mobile/modules/` for hardware keys (`UIKeyCommand` on iOS, key events on Android): Esc, Tab, the arrows, Ctrl and Alt combinations, typed into the controlled terminal; Cmd shortcuts left to the system                                                                                                                                                                                        | Unit tests of the key mapping; the Android emulator with its hardware keyboard. On the iPhone (owner), a Bluetooth keyboard: `vim`, `Ctrl-C`, arrows in Claude's menu. A new TestFlight build                                                                                                                                                    |
| 10  | Honest offline, no leaks, diagnostics    | R4    | A leak audit with tests: a marker typed into a terminal and printed by it, and a marker title, must appear in no server log, no host log, no phone cache or diagnostics. Offline states reviewed against 31 §5. **Diagnostics** in the app's Settings: versions, server id, states and error codes with times, shown in full before sharing through the share sheet. The server's logs checked under load for the same markers | The marker tests in `just check`; the stand-in and staging logs grepped after a session; on the iPhone, a diagnostics export read before it is shared                                                                                                                                                                                            |
| 11  | Generic push                             | R4    | Migration `0006_push_routes.sql`; the provider trait with APNs and FCM; the host's notification intents over the link; deduplication, coalescing, preferences; the app's permission, token and **Notifications** settings; the self-host notes                                                                                                                                                                                 | A fake provider in server tests; the stand-in with a recording provider. On staging, with the owner's APNs key: a locked iPhone gets "Yardsort: an agent needs you" when Claude asks for permission on the headless host, and opening it shows that session. Android only once there is an FCM project and a phone                               |

Slices 8 to 10 do not depend on slices 4 to 7 and can move earlier if the owner wants R4 sooner; slice
11 waits on the APNs key (§6).

## 5 · What each gate needs

**R5's gate**, and where each check comes from:

- **The same session from both clients**: slice 5's owner run, the iPhone and the desktop on this
  computer, both on one agent's terminal on the remote box, through staging. Recorded with the box's
  distribution, the commit and the harness's version.
- **The host without a GUI**: slice 2's test runs it with no display variables; slice 3's with an empty
  environment; the owner's run is on a box with no desktop installed at all. That last one is the check.
- **Persistent data across a service restart**: slice 6's test, and the owner restarting the service
  on the box during slice 6's run.

**R4's gate:**

- **Lock/resume and network changes**: slice 8 on the iPhone (31 §17's N and W). On Android, only once
  there is a phone; the emulator result is recorded as an emulator result.
- **Generic push**: slice 11 on the iPhone through staging. Android push needs a phone and an FCM
  project; until both exist, R4's push check is iPhone-only and the record says so.
- **Offline honesty and no leaks**: slice 10's tests and its staging and phone checks.
- **The moved items**: the two budgets (slice 8) and the external keyboard (slice 9), on the iPhone,
  and on Android once there is a phone.

**So R4 cannot close without a physical Android phone either**, as R3 cannot. Everything else in both
phases can be done with the iPhone, this computer and one remote box.

## 6 · What only the owner can provide

| What                                                                                                                                                                               | For                                        | When                                                       |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ | ---------------------------------------------------------- |
| **A Linux box with no display, and SSH to it.** Any VPS or spare machine, x86_64 or aarch64, with `git`, and Claude Code (or another harness) installed and signed in there by you | Milestone A's run, slices 2 to 8           | Before slice 2's owner run                                 |
| The phone and the Mac, as in R3                                                                                                                                                    | Every phone step                           | As now                                                     |
| An **APNs auth key** (`.p8`, its key id, the team id), made in the Apple Developer account, put into staging's secrets by you                                                      | Slice 11                                   | Before slice 11; a staging deploy then needs your go-ahead |
| A new **TestFlight build**, archived on the Mac                                                                                                                                    | Slices 9 and 11 (native changes)           | At those slices                                            |
| A **Bluetooth keyboard** for the iPhone                                                                                                                                            | Slice 9                                    | At slice 9                                                 |
| An **FCM project** in Firebase, in a Yardsort-owned Google account, and an **Android phone**                                                                                       | Android push; R3's and R4's Android checks | When you choose; neither blocks anything else              |
| Go-ahead for each **staging deploy**                                                                                                                                               | Slices 10 and 11                           | At those slices                                            |
| Go-ahead to **cut a release** carrying the host tarball                                                                                                                            | After slice 3, if wanted                   | Your choice                                                |

**Not needed:** an Expo account (push goes to APNs and FCM directly), a new domain (staging's is
enough, and self-hosted servers bring their own), or any change to ZITADEL.

**The owner's run for milestone A** (slice 2), in outline; the slice's record will give the exact
commands. Build `yardsort-host` on the box (or copy the tarball from slice 3), `yardsort-host sign-in
https://remote-staging.yardsort.sh --name "Build box"`, open the printed address on this computer, paste
the address the browser lands on back into the SSH session, start `yardsort-host serve`, run
`yardsort-host pair` in a second SSH session, scan its code on the iPhone, approve at the box, add a
repository with `ys project add`, start Claude with `ys workspace new`, and watch and type from the phone.

### The test box

Milestone A's box is a Hetzner Cloud server in a project of its own, made and removed by
[`scripts/test-box.sh`](../../scripts/test-box.sh) through an `hcloud` context of the same name, so it
never lands in another project. Its first boot is
[`scripts/test-box/cloud-init.yaml`](../../scripts/test-box/cloud-init.yaml): updates, the build tools, an
ordinary `yardsort` user that has the login key, password logins off, a firewall that lets in SSH only
(and a Hetzner firewall in front of it), lingering so `serve` and the agents outlive a logout, and Rust.
Nothing that box holds is in the repository: no address, no key, no account. How to use it, for anyone:
[A test box for Remote](../test-box.md).

| Step                     | What does it                                                                                                                                                                                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `scripts/test-box.sh up` | Creates the firewall and the server, waits for its setup, installs `gh` and Claude Code, makes the box an SSH key on the owner's GitHub account, clones the demo repository, copies this checkout over and builds both programs |
| By hand, about 5 minutes | Signing in Claude Code, `gh auth login`, `yardsort-host sign-in`, and pairing the phone: each is the owner's own                                                                                                                |
| `sync`                   | Copies the checkout again and rebuilds, for each slice's run; a running `serve` keeps the old build until restarted                                                                                                             |
| `down`                   | Shows work there that is not pushed and asks; signs the host out at the server (or the phone keeps a computer that never comes back), signs `gh` out, removes the box's key from GitHub, then deletes the server                |

**Cost.** The CX33 (4 vCPU, 8 GB, 80 GB) is €0.0136 an hour, at most €8.49 a month, plus its IPv4
address, as of 2026-11-07. Hetzner bills a server while it exists, powered off or not; only `down` stops
it. **When:** made on 2026-11-07 for slice 2, kept for the rest of milestone A, since each slice's run is
then a `sync` and a restart, and taken down when milestone A is done or the slices pause for more than
a couple of weeks. No snapshots or backups: either would hold the owner's sign-ins, and `up` rebuilds
it in about fifteen minutes. `sync` and `status` were run against the box; `up` and `down` as a whole
were not run again since it was made by hand with the same steps.

## 7 · Risks

- **The guard on approval is not a boundary** (§3). A same-user process on a headless box holds the
  host's credential file. The guide says so plainly, and running agents under a separate user is the
  operator's option for a stronger line; Yardsort does not do it for them in R5.
- **A restored credential is a spent one** (§3, backup). The guide says to sign in again after a
  restore, which keeps the host and its devices.
- **A container restart ends the agents.** The daemon lives inside the container. Service restarts
  under systemd do not. The guide puts the two side by side.
- **The tarball and glibc.** Built on the oldest runner the release workflow has; a distribution older
  than that needs a build from source, which the guide shows.
- **Two device credentials on one desktop** (host and client). Each is a separate entry and a separate
  device at the server, so neither's rotation can spend the other's.
- **The Rust channel layer is a second implementation** of what the TypeScript client does. The shared
  fixtures pin the wire; behaviour (resume, renewal, uncertain delivery) is pinned by tests against the
  real server on both sides.
- **Push needs credentials that are the owner's**, and Android push needs hardware that does not exist
  yet. R4's push check may close on the iPhone alone, and the record will say so.
- **No Android phone.** R3 and R4 both stay open on it. Nothing in this plan pretends otherwise.

## 8 · The device run reminder

R3's iPhone device run ([31 §17](31-remote-r3.md#17--the-device-run-on-the-iphone)) is still to do. It is
about twenty minutes, and its parts N and W are also R4's lock and network checks, so running it during
slice 8 covers both. A reminder is due around 2026-11-11.

## 9 · Later, not in these slices

- Driving workflows, smoke tests and schedules on a headless host (the core's drivers with a headless
  `Hands`). Until then `ys` refuses to queue them there (§3).
- Remote file browsing and Git writes, each with its own scope and API (proposal §11).
- A device authorization grant on the server, if pasting the address proves awkward.
- A Yardsort-operated push gateway for self-hosted servers (R7).
- Running agents as a user other than the host's.
- Pairing a device at a headless host on macOS: a guard that checks the caller without `/proc`.

### Later phases: where a task runs (proposed)

Set out by the owner on 2026-11-08 and only proposed here. None is an R4 or R5 requirement, and none
changes what slices 1 to 11 build. Each needs its own plan when it comes, with its slices and smoke
tests, as this one has. The order is the order of what each needs.

**Execution targets, after R5's gate and slice 7.** The desktop's composer offers where a new task
runs: **This computer**, or a host the desktop is let into with the control a start needs, each
with its state, an offline one shown and not offered. The task's project has to be the same
repository there, so it waits on R6's explicit linking of a project across machines, or on a
narrower mapping of its own decided then (a project's remote URL, confirmed once by the person). What
it starts uses slice 7's `workspace.start` and `session.start`, by ids. The workspace it makes is the
other host's, listed under that host, as slice 4 lists them. Nothing is queued for a host that is
offline. Whether it is a milestone of its own or R6's first slice is decided when R6 is planned.

**Handover between machines, after execution targets.** A task moves by Git and by a packet the
person reads, never by moving a process:

- Git carries the code. The person pushes the workspace's branch, or confirms a push Yardsort offers.
  The other host starts a workspace on that branch: `workspace.start` gains a branch to start from, by
  name, never a path.
- The packet carries the task's state: the handoff packet that already exists between two agents on
  one computer (`activity::handoff`, [18](18-agent-events-stage-4-handoff.md)), rendered on the host
  the work is leaving, shown in the composer to be read and edited, and sent as the new session's
  prompt. Not a transcript, not a harness's own session files, and nothing the person did not see.
- The old session stays on its host until it ends or is ended there. Both directions: this computer
  to a host, a host to this computer, and between two hosts, from a client let into both.
- It is a person's act from a client. No API an agent is given starts one. The packet crosses the
  server as typing does, carried and never kept, and is bounded in size.

**Managed sandboxes, after R7.** Proposal §3.4 and §11 put provider-hosted compute out of scope;
this would reverse that, so it needs the owner's decision before any plan, and a plan of its own. A
**registered VM** is the person's machine: they run it, pay for it, sign it in, approve every device,
and it holds their credentials and keeps their work. A **managed sandbox** would be Yardsort's
machine, made for one task, and gone after it. It needs everything a registered VM leaves to its
owner:

- Isolation fit for strangers' code side by side: a microVM per task, not a container (proposal §3.2).
- Provisioning from an image with `git` and the agents, and the repository's setup run as the
  project's automation already runs it.
- Credentials scoped to the task: a token for that one repository that expires with it, and an
  agent's sign-in the person gives for that task or that is metered through Yardsort, never their own
  account-wide keys.
- Results by Git and the handover packet back: a pushed branch, or a pull request the person opens.
- Cleanup that is checked: the sandbox and its disk are destroyed when the task ends, fails or times
  out, and the account sees that it was.
- Cost controls: a limit per task and per month, an idle timeout and a hard stop, enforced by the
  server, with R7's billing.
- Enrollment of its own: a sandbox would be a host made by the service, not one the person signed
  in. Who approves its grant, and what a device may do there, is that plan's first decision. The
  default stays: nothing is let in because it is on the account.

## 10 · What the first slice adds

Built on 2026-11-06, on Linux. A pure move: nothing a person sees changes, and the server and the phone
did not change, so staging needs no deploy and the phone no new build.

- **`remote_host::account`** (`crates/remote-host/src/account.rs`): signing in as a host (`prepare`, then
  the caller's own way of getting the browser's answer, then `enable`), the session and its refresh
  credential (`ensure`, `access`), the link (`connect`, `disconnect`, `link`, `recheck`), pairing
  (`pair`, which returns the code and the link a QR code holds), `requests`, `answer`, `revoke` and
  `disable`. A device asking reaches `Home::asked`, which the app turns into the window's event.
- **`Home`** is what each program provides: its records, its settings, its credential store, what a
  channel answers from, and how to tell a person of a device asking.
- **Errors** are `Failure`, a stable code and words. Signed out says why and not what to do: the app adds
  "Sign in again in Settings → Remote.", and the headless host will add its own command.
- **The app** (`src-tauri/src/remote.rs`) keeps its commands, their types and its words, the OS credential
  store, the QR code's SVG, the status it shows and replacing an earlier daemon. It no longer holds a
  session or a connector of its own.
- **Tests.** The two that were the app's, a host signed in again being the same host and a refusal of its
  credential reading as signed out, are now the module's; the app keeps one for its own words. Nothing new
  in behaviour, so nothing new to fail without it: the smoke tests below are what show the move kept it.
- **Found on the way:** `remote-earlier-daemon` waited for a daemon of version 0.21, so it failed once
  0.22.0 was released, whatever the code did. It now waits for any daemon but the 0.20.2 one.

**Its smoke test.** Locally, in the real window, against the stand-in server and a pretend phone:
`remote-sign-in-again` passed, 29 steps of 29, and `remote-earlier-daemon` passed, 13 of 13, after the fix
above. `just check` and `just bindings-check` pass. Not run here: Windows and macOS, which CI covers.

**On the iPhone against staging** (passed, 2026-11-06, run by the owner): a development build of the app from this branch, signed in to
staging, with the phone already paired, still lists this computer, watches a shell, takes control, types,
and lets go; and a new pairing is approved at the window.

## 11 · What the second slice adds

Built on 2026-11-07, on Linux. Nothing changed on the server or the phone, so staging needs no deploy
and the phone no new build.

- **The runtime in the core** (`yardsort_core::runtime`): settling the records at start
  (`settle_at_start`), on each exit (`exited`), and the inbox watcher that drains hooks' reports as they
  land (`watch_inbox`, `drain`), with its tests. The app calls them; it keeps only telling its window.
- **`yardsort-host`** (`crates/host`, GPL-3.0-only, Linux; it builds on macOS and is a stub on Windows).
  It handles `--yardsort-daemon`, `--yardsort-hook` and `--yardsort-print-env` first, so its daemon is
  its own build. Commands: `sign-in`, `serve`, `pair`, `devices`, `revoke`, `status`, `sign-out`. Its
  `Home` keeps the credential in `<data dir>/credentials` unless `YARDSORT_CREDENTIALS_DIR` says
  otherwise. Since #180 it also answers a device's `workspace.start` and `session.start`, with the same
  `Starter` the app uses, so the phone can start work on a computer with no screen.
- **Signing in by pasting.** `remote_client::SignIn` now splits into `returned` (the browser came back),
  `pasted` (the address copied out of a browser elsewhere; only this sign-in's own loopback address, port
  and path are taken) and `redeem`. `sign-in` waits for either. `account::enable` no longer starts the
  link itself: the app does, and `sign-in` does not, since `serve` will.
- **One program per profile.** The lock now says which program holds it (`presence::Holder`, in
  `app.holder` beside `app.lock`, since Windows will not let a locked file be read). `serve`, `sign-in`
  and a `sign-out` with nothing serving all take it. `ys workflow run`, `ys test run` and
  `ys schedule run` refuse on a host's profile, and the schedule list says schedules do not fire there.
  The app on such a profile says that `yardsort-host serve` has it, at once, instead of waiting ten
  seconds for a window that will never answer.
- **The control socket and the guard.** `serve` listens on a Unix socket made `0600` under a `0177`
  umask. `pair` is refused unless the peer (`SO_PEERCRED`) is this user, carries none of the session
  markers, has a terminal on standard input that is not one of the daemon's children's, and does not
  descend from the daemon. A guard, not a boundary, as §3 says and the guide repeats.
- **`ys project add <path>`**, with `Projects::open`'s checks, and no `git init`.
- **Found on the way.** `<data dir>/host.sock` is too long for a Unix socket in deep data directories
  (a smoke run's is): a Unix socket's path
  may be at most 108 bytes. The socket follows the daemon's rule instead,
  `$XDG_RUNTIME_DIR/yardsort/<tag>.host.sock`, then `<data dir>/host.sock`, then the temporary
  directory (`pty_ipc::socket_candidates`), and a command tries each, since `serve` under systemd and an
  SSH login may not see the same `XDG_RUNTIME_DIR`. A smoke harness that is `sh` given the agent's first
  message runs the project-memory briefing as shell input; the test's plain shell takes no prompt.
- **Tests that fail without it.** The pasted address (`remote-client`); the holder (`presence`); `ys`
  refusing on a host's profile and `ys project add` (`crates/cli/tests`); the app naming a headless
  holder (`one_copy`); the socket's mode, the protocol's shape and the guard's parsing (`crates/host`).
- **Not in it**, as planned: packaging (slice 3), workflows, smoke tests and schedules on a headless host
  (§9), and resume and fork (slice 6).

**Its smoke test.** `headless-host.yaml`, locally, with no display variables at all: passed on
2026-11-07, 20 of 20 steps in 15 s.

**On the box against staging**: passed on 2026-11-07, by the owner (§6). A Hetzner CX33 with Ubuntu
24.04 reached over SSH, with `git`, `gh` and Claude Code installed and signed in as an ordinary user.
Both programs built there from the branch, signed in to `https://remote-staging.yardsort.sh` by
pasting, `serve` under `tmux`, a repository added with `ys project add`, Claude started with
`ys workspace new`, the iPhone paired with `yardsort-host pair`; it watched, took control and typed.
`serve` stopped and started again: the agent still there, and so was the pairing.

It found one bug: `Ctrl-C` did nothing in the terminal `serve` ran in. The probe of the login
shell's environment starts an interactive shell, which takes the terminal and on leaving hands it
back to whoever held it when it began; two probes at once left it with the first, already gone.
The probe now runs in a session of its own, with no terminal, as it does when the app is opened
from the desktop (`env.rs`, with a test that its shell's session is not ours).

## 12 · What the third slice adds

Built on 2026-11-08, on Linux. Nothing changed on the server or the phone, so staging needs no deploy
and the phone no new build.

- **The tarball** (`scripts/host-package.sh`): `yardsort-host-<version>-linux-<arch>.tar.gz`, holding
  `bin/yardsort-host`, `bin/ys`, `systemd/yardsort-host.service`, a README and the licence; the same
  bytes for the same build. The release workflow's **host** job builds it for x86_64 on `ubuntu-22.04`
  and for aarch64 on `ubuntu-22.04-arm`, runs both programs with an empty environment, and attaches it;
  the release is published only once both are attached. CI's **Host package** job builds both on every
  pull request and prints the newest glibc symbol they need.
- **The systemd user unit** (`crates/host/packaging/yardsort-host.service`): `Type=exec`,
  `ExecStart=%h/.local/bin/yardsort-host serve`, `KillMode=process`, `Restart=on-failure`. No network
  dependency: a user unit cannot wait for the system's network, and `serve` retries the link itself.
  `loginctl enable-linger` is the guide's step, not the unit's.
- **The image and the compose example** (`crates/host/Dockerfile`, `crates/host/compose.yaml`). The
  server's pinned Rust build stage; `debian:trixie-slim` pinned by digest, with `git`, `openssh-client`,
  `ca-certificates` and `tini`; a `yardsort` user whose ids are build arguments; `/data`, `/credentials`
  and `/worktrees` set by `YARDSORT_DATA_DIR`, `YARDSORT_CREDENTIALS_DIR` and `YARDSORT_WORKTREE_ROOT`;
  `tini -- yardsort-host` as the entry point and `serve` as the command. No `EXPOSE`, no published port,
  no Docker socket. CI builds it, runs `--version` for both programs, signs in to the stand-in by
  pasting (`headless.sh` with `YS_INPUT_BUILD=image` runs the image), runs `serve` until the link is
  connected, and checks that `docker stop` ends it with status 0. No image is published.
- **`yardsort-host status` names a daemon from an earlier release**, kept after an upgrade for the
  agents in it, and says phones cannot watch its terminals until it is replaced, once they end and
  `serve` restarts. §3 promised it; slice 2's `status` did not say it.
- **The guide**: installing from the tarball, the service with linger, the container (the UID, the
  volumes, no agents in the image, no port, no Docker socket, a restart ending the agents), upgrading,
  backing up (the profile and `settings.toml`; not the sign-in, with signing in again after a restore),
  and the sockets' places.
- **Found on the way.** Under systemd, `serve` has `XDG_RUNTIME_DIR`; a shell from `sudo -iu` or `su`
  does not. Each computed its own daemon address, so `ys` in such a shell started a second daemon, and
  `serve`, after a restart, took the first one's running agents for ended. Tried on this computer with a
  transient user unit and an `env -i` shell. `pty_ipc` now falls back to `/run/user/<uid>` when
  `XDG_RUNTIME_DIR` is unset, if that folder is this user's alone, for the daemon's address and the
  host's socket alike. A process that has the variable is unaffected. Tried again the same way: one
  daemon, and the agent still running after a restart and a stop.
- **The smoke test** gained an input. `headless-host.yaml` with `--input build=package` builds the
  tarball, unpacks it in the run's folder, and runs every step's `yardsort-host` and `ys` under
  `env -i` with only `HOME`, a `PATH` of the tarball's `bin` and the system's, and the run's profile.
  `build=debug`, the default, is slice 2's test as it was.
- **Tests that fail without it.** The unit stops `serve` alone and installs where the README says; the
  image runs as `yardsort` under `tini`, pinned, with no port, no privilege and no Docker socket
  (`crates/host/tests/packaging.rs`); the runtime folder counts only when it is this user's alone
  (`pty-ipc`).
- **Not in it**: a published image, packages for distributions, and macOS. The earlier-daemon line has
  no test of its own: making an earlier daemon needs the fake one in `pty-ipc`'s tests, which the host
  cannot reach.

**Its smoke test.** `headless-host.yaml` with `--input build=package`, locally: passed on 2026-11-08,
20 of 20 steps in 14 s, every `yardsort-host` and `ys` the unpacked tarball's under `env -i`. The image's
check runs in CI's **Host package** job.

**On the box against staging**: passed on 2026-11-08, by the owner. The tarball built here, copied to
the test box (Ubuntu 24.04, linger on), installed in `~/.local/bin` with the unit in
`~/.config/systemd/user`, in place of slice 2's `serve` under `tmux`, with Claude left running.
`systemctl --user enable --now yardsort-host` linked to `https://remote-staging.yardsort.sh`, the iPhone
watched and typed, and after `systemctl --user restart yardsort-host` the phone reconnected to the same
agent.
