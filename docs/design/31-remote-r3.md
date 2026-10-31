# Remote, R3: the phone app

**Status:** planned on 2026-10-30. The first slice, the TypeScript protocol client, was built on
2026-10-31 (§9); there is no app yet. This is R3's plan: what it has to show, what it starts
from, the decisions taken before any app code exists, the slices, and what each slice's smoke
test needs. The phone app's license was decided with it: MPL-2.0 (§6). Like
[27](27-remote-r2.md), it becomes the record of the slices as they are built. The proposal is
[25](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md); R0 and R1 are in [26](26-remote-r0-r1.md),
R2 in [27](27-remote-r2.md).

## 1 · What R3 has to show

The proposal's gate (§13): **a physical iPhone and a physical Android phone interact with real
agent sessions on existing hosts, and keyboard and control handoff pass.** Mocked connectivity,
an emulator or a TypeScript build is not that, and nothing below is called done on their
strength (§14).

In R3, a person can:

- choose a server (the managed one once it exists, or their own) and sign in through the system
  browser;
- pair the phone with a computer by scanning its code or typing it, and see it approved at the
  computer;
- see their computers, online or not and when last seen, then each one's workspaces, sessions
  and recent activity;
- watch a terminal, take control of it, type, paste and use the keys a phone lacks, and hand
  control back to the person at the computer, who can take it at any time;
- lock the phone, come back, and find the same terminal where it was;
- see when a computer is offline, when they were signed out, and when the phone was revoked, and
  sign out themselves.

R3 does **not** include:

- notifications while the app is closed, or any push (R4);
- diagnostics export, privacy defaults beyond those below, or the self-host push story (R4);
- a headless host or a desktop client of another computer (R5);
- sync (R6);
- store listings, store builds or billing (R7). Every build in R3 is a development build on the
  owner's own devices;
- editing code, Git writes, launching a new agent, approving anything on an agent's behalf, or
  approving memory. The proposal excludes them from the first mobile scope (§3.4). Reading
  approved memory is in scope, as `memory.list` already serves it.

## 2 · Where it starts

Checked against `main` at `eeb2df5` on 2026-10-30.

**What is there.** Everything a phone has to reach exists and is tested.

- `yardsort-server` serves sign-in, the server's own credentials, pairing, relay tickets and the
  relay (27 §2–§15). Its routes are in `crates/server/src/http.rs`. A device uses `GET /v1/server`,
  `GET /v1/login`, `POST /v1/token`, `GET /v1/devices`, `POST /v1/devices/{id}/revoke`,
  `GET /v1/hosts`, `POST /v1/pairings`, `GET /v1/grants` and `GET /v1/grants/{id}`,
  `POST /v1/grants/{id}/revoke`, `POST /v1/relay/tickets` and the `/v1/relay` socket.
- **The remote protocol** is at major 1, minor 4 (`crates/remote-proto`). Over the relay, a device
  sends `Hello`, is welcomed with the host's description, and then sends requests and receives
  replies and events as JSON text. Terminal output comes as binary frames with a 20-byte header
  (kind, version, stream, epoch, sequence). The operations a phone needs are all there:
  workspaces, sessions, activity with a cursor, approved memory, and terminal attach, resume,
  lease, input and resize (27 §8–§11).
- **Sign-in already allows the app's scheme.** The server's default device redirects are
  `yardsort://signed-in` and a loopback address (`config.rs`). Staging uses the defaults.
- **The desktop already makes the pairing code a phone scans:** `yardsort://pair?server=…&secret=…`
  (`src-tauri/src/remote.rs`), beside an eight-character code for typing.
- **A reference device in Rust**, `crates/remote-client` and `crates/server/examples/pair.rs`
  (`just server-pair`). It signs in, pairs, watches and types. The phone's client follows what it
  does.
- **The wire is pinned by 39 JSON fixtures** in `crates/remote-proto/fixtures/v1/`. Each one
  must parse and serialise back to exactly itself (`src/tests.rs`).

**What is not.**

- No TypeScript speaks the protocol. `src/lib/bindings.ts` holds only the desktop's own IPC types.
- No `apps/` or `packages/` directory. The root `package.json` has no workspaces, and `website/`
  is its own Bun project.
- No fixture for a binary frame. Their layout is pinned only inside Rust unit tests.
- Nothing has ever signed in through `yardsort://signed-in`. Both reference devices come back to
  a loopback listener, which a phone cannot run.
- Nothing receives `yardsort://pair`.
- No device approval is pushed: the reference device polls its grant every second.
- A JavaScript WebSocket never sees the relay's pings, so a phone cannot apply the server's
  60-second silence rule from them.
- While a device holds control, no viewer answers a program's terminal questions; the host
  answers only where the cursor is (27 §8). R2 left the question of making the controller's
  viewer the responder for R3.

## 3 · The stack

The proposal settled the framework: React Native, TypeScript and Expo, with Expo Router,
SecureStore, development builds, and a bundled xterm.js in an isolated WebView (§1, §7). It is
not reopened here. Versions were current on 2026-10-30; the slice that adds each package pins it
exactly and records it.

| Piece           | Choice                                                                                                                                            |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Expo SDK        | 58 (released 2026-10-15): React Native 0.88, React 19.3, iOS 16.4 and later, Android 7 and later. New Architecture only, as every SDK since 55    |
| Navigation      | Expo Router                                                                                                                                       |
| Credentials     | `expo-secure-store`, `WHEN_UNLOCKED_THIS_DEVICE_ONLY`, so a backup or a new phone does not carry them                                             |
| Sign-in         | `expo-web-browser`'s auth session: `ASWebAuthenticationSession` on iOS, Custom Tabs on Android. Not an embedded WebView, which RFC 8252 rules out |
| Pairing code    | `expo-camera`'s QR scanning, and the `yardsort://pair` link when the phone's own camera opens it                                                  |
| Terminal        | `react-native-webview` 14 with `@xterm/xterm` 6 and its fit and Unicode 11 addons                                                                 |
| Protocol client | `packages/remote-client`, plain TypeScript with no dependencies (§4)                                                                              |
| Package manager | Bun, as the rest of the repository. The app is its own Bun project under `apps/mobile`, like `website/` (below)                                   |

**Why the app is not a workspace of the root project.** Bun installs a workspace with its
isolated linker by default, and this repository's lockfile already selects that behaviour. Adding
`workspaces` to the root `package.json` would change how the desktop app's own dependencies are
laid out, for no gain to it. Expo also fails at run time when one app ends up with two copies of
React or React Native. So `apps/mobile` has its own `package.json` and lockfile, and takes the
protocol client from `packages/remote-client` as a local file dependency. That package has no
dependencies, so it brings nothing to resolve twice.

**Building.** Android development builds are made on Linux with the Android SDK, without EAS.
**iOS builds need macOS and Xcode.** The owner has both, an iPhone, and a paid Apple account, so
iOS development builds do not lapse after the seven days a free account allows, and associated
domains and push are available when they are wanted (§5, R4). EAS is not used. A run on Linux
does not show that the iOS app works, nor a simulator or an emulator that a phone does, and the
record will say where each result comes from.

## 4 · The protocol client

`packages/remote-client` is the TypeScript twin of `crates/remote-client`, and the only code that
talks to the server. Screens never build a request themselves. It runs anywhere with `fetch` and
`WebSocket`: on the phone, in Bun for tests, and later in a desktop client of another computer
(R5).

- **Types written by hand, checked by the fixtures.** The Rust types are the source. The
  TypeScript types are written to match, each with a decoder that enforces the same bounds:
  string lengths, list sizes, 64 KiB messages, 64 KiB frame payloads. Messages from the host
  come through the relay from a computer the person paired, but they are still parsed as
  untrusted. Generating the types from Rust was considered and set aside: it would give the
  shapes, but not the bounds or the decoding of values a newer peer might send, which are what
  the fixtures test.
- **The same fixtures as the Rust side.** A conformance run with `bun test` reads every file in
  `fixtures/v1/`, decodes it, encodes it again and requires the same JSON. It also repeats the
  per-file assertions that `remote-proto`'s tests make. A file that one side reads and the other
  does not fails the run. The first slice also adds **binary frame fixtures** (a hex string and
  the fields it decodes to, as JSON), pinned by both sides.
- **Credentials.** Sign-in, refresh and sign-out are as in the Rust client. One refresh at a time
  per profile: a second caller waits for the first one's answer instead of sending the same
  credential, since a spent one revokes the device outside the 30-second retry (27 §21). The new
  refresh credential is saved before the access credential is used. Credentials are never
  logged, and neither are pairing secrets, codes or tickets.
- **The relay channel.** A ticket, the socket, `{ticket}` as the first message, the greeting, then
  `Hello`. It matches each reply to its request, routes events and frames, keeps attachments by
  stream, and turns close codes into states the screens show (4004 offline, 4008 fell behind,
  and so on). 4003 is not one state: the relay sends it when one computer took back this phone's
  grant, when the computer itself was removed or signed out, and when the phone was revoked
  (`relay.rs`, `recheck`). The client asks the server which it was before anything acts on it
  (§5).
- **Liveness without pings.** While the app is in the foreground, a channel with nothing sent for
  20 seconds sends `host.describe`. One that gets no answer within 10 seconds is treated as dead,
  closed, and opened again. A lease's renewals every 10 seconds count as traffic.

## 5 · Decisions

**Sign-in comes back through `yardsort://signed-in`.** It is the server's default, so every
server already allows it. An https callback through universal links and App Links is stronger
against another app claiming the scheme, but the domains it works for are fixed in the app when
it is built, with a file on each of them. The managed service's domain could be one; a
self-hosted server's cannot. So every server keeps the scheme, and a universal link for the
managed service is an addition for later, not a replacement. The second PKCE
step already makes a stolen code worthless: the code that reaches the redirect needs the verifier
the app kept (27 §2). A development build uses the same scheme; it has its own bundle identifier,
so the two are told apart on the phone, not at the server.

**One server at a time, never switched by a link.** The app keeps a profile per server: its URL,
the server's id from `GET /v1/server`, this phone's device id and its refresh credential. Each
profile's credentials and cache are its own, and one profile is active. A pairing link names its
server; if that is not the active one, the app says so and asks before signing in there. It never
reuses another server's credentials (§5.1).

**Pairing.** Scan the computer's code, or type its eight characters. The app redeems it, then
asks for its grant every two seconds until the person at the computer decides, or the offer's five
minutes run out. Polling is enough for something that happens once per phone, and push is R4.
The app asks for every scope a phone uses. The computer's person sees them and chooses (27 §7).

**Online is what the socket says.** The server has no presence, and `lastSeenAt` is history. A
computer is shown online while the app holds a channel to it, offline with when it was last seen
when the relay answers `host_offline`, and reconnecting in between. Offline never means the
agents stopped (§3.1).

**The terminal is drawn in a WebView, typed into natively.**

- The WebView renders. It loads one self-contained HTML document, built from pinned xterm.js and
  a small bridge script by a build step, inlined into the app's bundle, and checked to be current
  the way `bindings.ts` is. Nothing is loaded over the network. Navigation away from it is
  refused, links open only when the person taps one and only for `https`, and clipboard writes
  from escape sequences are dropped.
- The bridge carries typed messages, each bounded and checked on both sides: output frames in,
  size and selection out. The page never receives a credential, a ticket or a server address.
- **Typing goes through a native composer line and a row of keys** (Esc, Tab, Ctrl, the arrows,
  and Enter), not into xterm.js's own hidden textarea. xterm.js on Android still loses and repeats
  keystrokes under some keyboards (its issue 3600 is open), and a native field gives the system
  keyboard, composition, dictation, screen readers and dynamic text for free. The device run
  (§7) tries direct typing into the terminal as well. If it works on both phones, it can be
  offered beside the composer; it does not replace it within R3.
- Paste goes as one write, bracketed when the program asked for bracketed paste.

**Who answers a program's questions.** R3 keeps R2's rule: while a phone holds control, no viewer
answers, and the host answers where the cursor is. The phone's xterm.js never answers, since the
host already strips the questions from what a phone is sent. The device run checks this with
Claude and Codex, which ask their terminal things when they start. If either is held up by a
question nobody answers, the controller's viewer becomes the responder: a minor version that
lets the phone send answers marked as answers, which the host writes without taking control, as
it does for a terminal shown a question (27 §8). That is a slice of its own if needed, not
assumed.

**View-only is drawn at the computer's size.** A phone that is only watching does not resize
anything. It draws at the terminal's own columns and rows, and scrolls sideways when that is
wider than the screen. A phone that takes control may resize the terminal to fit. Its size stands
while it holds control, and the computer's own size comes back when it lets go or the person
there types (27 §8).

**Reconnecting never types.** On a short drop, the app attaches again with the last sequence it
drew and resumes. A new epoch, or a gap the host can no longer fill, gives a fresh snapshot that
replaces the screen. After 4008 (fell behind), it attaches again for a snapshot. Unsent text in
the composer stays there to be sent again by hand. Text whose delivery is uncertain is shown as
such and never sent again by itself. Losing the lease clears what was waiting to be sent (§6.2).

**The app's life on the phone.** When the app goes to the background, it releases any lease and
closes its channels. Nothing keeps a socket open in the background, which is what the phone
expects. Coming back to the foreground opens them again, resumes the terminals, and refreshes
the lists. Locking the phone changes nothing on the computer, apart from control coming back to
it.

**What the phone keeps.** Its own profile in SecureStore. A small cache of computers and
workspaces, each entry with when it was observed and shown as such, so a cold start shows
something before the network answers. No terminal output is kept, and no activity, memory or
session title (below). Signing out, or removing a profile, deletes that profile's credentials and
cache. What a closed channel takes away depends on what was revoked, and the relay's 4003 does
not say, so the app asks with its own credential (refreshing it first if it has lapsed):

- **One computer took back its grant** (`GET /v1/grants/{id}` says `revoked`), or the computer
  is gone from `GET /v1/hosts`: that computer is shown as no longer paired, and its cached
  entries are deleted. The phone stays signed in, and every other computer it is paired with
  goes on working.
- **The phone itself was revoked**, or its account signed out at the server: the server refuses
  the phone's own credential, or the refresh comes back signed out. Only then are the profile's
  credentials and cache deleted, the phone says it was signed out, and it keeps nothing it
  showed.
- **The server cannot be asked** (offline, an error): nothing is deleted. The computer is shown
  as unreachable, and the app asks again when it can.

**Session titles (#36): they cross, as now.** Without them, a person cannot tell two
conversations apart. A title may quote the person's first message, so the app treats it as the
person's content: shown, never logged, never written to the cache, and never sent anywhere but
the screen. A mode that hides titles can come later if someone asks for one.

## 6 · The license (#43)

The phone app is published through Apple's App Store and Google Play. Apple's terms add usage
rules to every app the store distributes, such as personal use, Apple devices only and no
redistribution. The Free Software Foundation's view is that these are further restrictions the
GPL forbids. Over that, it had GNU Go removed in 2010, and a contributor's complaint had VLC
removed in 2011. VLC came back in 2013 under MPL-2.0. Google Play's terms are milder, but they
also forbid sublicensing what the store distributes. Every commit in this repository so far is
the owner's, so the owner can publish the app under any license today. Once someone else
contributes under a license that forbids store distribution, the app could no longer be
published without their consent. So the choice is made before the first line of app code, and
the app's own `LICENSE` file says it from that first slice.

| Option                                   | What it means                                                                                                                                                                                                                                                                                                                                                                                                     | Used by                                                              |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| **GPL-3.0 with a store permission (§7)** | Copyleft like the rest of Yardsort. A written additional permission lets anyone distribute it through an app store despite the store's terms. Contributions come in under the same terms, so the permission carries, and forks keep it. The wording is ours to write and has no ruling behind it; it should be read by counsel before the first store submission. GPL-only code from elsewhere cannot be taken in | Nextcloud's iOS app (a promise not to enforce the conflict), Ptt-iOS |
| **MPL-2.0**                              | Copyleft per file: anyone who ships a changed file must publish that file. The app can be combined with closed code. It explicitly allows distributing the built app under other terms as long as its source stays available under the MPL, so a store's terms do not conflict. No exception to write                                                                                                             | VLC for iOS since 2013, Firefox for iOS                              |
| **MIT or Apache-2.0**                    | No copyleft. Anyone may ship a closed version of the app, for example one pointed at their own server. The server stays AGPL whatever the app's license                                                                                                                                                                                                                                                           | Most React Native apps                                               |

A fourth way, a contributor agreement that lets the owner relicense contributions (as Signal and
Mastodon require), keeps the GPL without an exception. It puts a form in front of every
contributor and still needs the store question answered for the owner's own builds, so it is
listed only for completeness.

**Recommendation: MPL-2.0** for `apps/mobile` and `packages/remote-client`. It keeps the app's
own code open, as the GPL does for the desktop. The store question is answered by the license's
own text, not by an exception Yardsort writes and hopes holds. It is the license VLC moved to for
exactly this reason. And it lets the protocol client be used by someone building their own client
against a self-hosted server without taking on the GPL. What it gives up next to the GPL with a
permission: a fork can add closed files beside the open ones. The managed service's value is the
server's operation (30), and the server's AGPL is unchanged, so that loss is small.

The desktop app and the CLI stay GPL-3.0-only and the server AGPL-3.0. No code moves from them
into the app. Shared logic is written new, or relicensed by the owner, who holds it all.

**Decided: MPL-2.0**, by the owner on 2026-10-30, for `apps/mobile` and
`packages/remote-client`. Each gets the license's text in its own `LICENSE` file, and a
`license` field saying `MPL-2.0`, in the slice that creates it.

## 7 · Slices

Each slice ends with a smoke test the owner runs, as in R2. Every slice from the second on needs
a physical phone. **The owner has an iPhone and a Mac, and no Android phone yet** (2026-10-30).
So the smoke tests run on the iPhone. Android is built and tried in an emulator on Linux as the
work goes, which shows that it builds and runs, not that it works on a phone. Where a row below
says "the phone", that is the iPhone now, and the Android phone again once there is one. R3's
gate needs both, so **R3 cannot close until a physical Android phone has been through the device
run.**

**A phone and a development server.** The server accepts plain HTTP only on a loopback address
(27 §3), and a phone's loopback is not the computer's. So:

- **The iPhone uses staging** (`https://remote-staging.yardsort.sh`), which is real HTTPS.
  Staging gets each slice's server changes, if a slice has any, with the owner's go-ahead for
  that deploy.
- **The iOS Simulator** on the Mac shares the Mac's loopback, so it can use a local environment
  run on that Mac (`just server-up`), for development only.
- **An Android phone over USB**, once there is one: `adb reverse tcp:8787 tcp:8787` and
  `adb reverse tcp:8080 tcp:8080` carry the phone's `127.0.0.1:8787` (the server) and
  `127.0.0.1:8080` (the local ZITADEL) to the computer, and the local environment works
  unchanged. The emulator can do the same.

| #   | Slice                             | What it adds                                                                                                                                                                                                                                                                                                                                  | Its smoke test                                                                                                                                                                                                                                                                                                                                                                                          |
| --- | --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | The protocol client               | `packages/remote-client` (§4) and its license. The conformance run over `fixtures/v1/`, and binary frame fixtures pinned by both sides. Tests against a real `yardsort-server`. A TypeScript twin of `server-pair` that signs in, pairs, watches and types from Bun, and lists and revokes the account's devices. `just check` runs its tests | The twin against the local server and the desktop app: signs in, is approved, watches a shell, types into it, is revoked                                                                                                                                                                                                                                                                                |
| 2   | The app, signed in                | `apps/mobile` on Expo SDK 58: Expo Router, the Connect screen (server address, what the server can see, §9 of the proposal), sign-in through the system browser to `yardsort://signed-in`, SecureStore, this account's devices, sign out. Profiles, one active. The app's typecheck, lint and unit tests in `just check`                      | A development build on the iPhone signs in against staging, lists its devices and signs out                                                                                                                                                                                                                                                                                                             |
| 3   | Pairing and computers             | Scanning the computer's code, `yardsort://pair` links, typing the code, the question when a link names another server, waiting for approval, the computers with online, offline and last seen, removing a pairing                                                                                                                             | The phone pairs with the desktop app by code and by typing, and are approved at the computer. A denied request and an expired code say so. Unpairing removes it                                                                                                                                                                                                                                         |
| 4   | Workspaces, sessions and activity | A computer's workspaces, a workspace's sessions, recent activity followed live, approved memory read-only. Cached observations shown as such                                                                                                                                                                                                  | With Claude and Codex running in two workspaces, the phone shows them and their activity as it happens. Offline shows the last-known state with its time                                                                                                                                                                                                                                                |
| 5   | Watching a terminal               | The WebView and its bridge (§5), attaching, the snapshot, resuming after a drop, a new snapshot on a new epoch or after falling behind, view-only at the computer's size with sideways scrolling, a program's end                                                                                                                             | The phone watches a running Claude session and a full-screen program. Airplane mode for a few seconds resumes without a corrupt screen. The app in the background and back resumes                                                                                                                                                                                                                      |
| 6   | Taking control                    | Acquire, renew and release, taking over from another device, the composer and the key row, paste, resize while in control, control taken back at the computer, uncertain delivery shown, the lease lost clearing what was waiting. The terminal-question check (§5)                                                                           | On the phone: type to Claude and Codex, use Esc, Ctrl-C and the arrows, paste several lines, and have the person at the computer take control back mid-typing                                                                                                                                                                                                                                           |
| 7   | Signed out, revoked and switching | The states a phone can be left in, told apart as §5 says: one computer's grant taken back, a computer gone, the phone revoked, signed out at the server. Removing a profile, a second server, and nothing from the first shown under the second                                                                                               | With the phone paired to two computers (this one and the Mac), take its grant back at one while it watches: that one shows unpaired, the other still works, and the phone stays signed in. Then revoke the phone from another of the account's devices (slice 1's TypeScript twin) and see it signed out with nothing kept. Sign in to a second server (staging and local) and see nothing of the first |

Then the **device run**, R3's gate, recorded in this doc: on both phones, with the software
keyboard and an external one, in portrait and landscape:

- real Claude and Codex sessions, and a shell with a full-screen program;
- Unicode and composed input;
- control handed over and back with the person at the computer;
- the phone locked and woken;
- Wi-Fi to cellular and back.

The proposal's budgets are measured there and recorded with how they were measured (§14): input
to its echo at p95 under 300 ms on a link with 100 ms of round trip, and a usable screen within 5
seconds of coming back to the foreground.

## 8 · Risks

- **Terminal input on Android.** xterm.js's own input misbehaves under some Android keyboards.
  Typing through the composer avoids it, at the cost of typing into the terminal directly; the
  device run says whether that can be offered too.
- **Programs that wait for an answer.** If Claude or Codex waits on a terminal question that no
  viewer answers while a phone controls, the controller's viewer has to become the responder: a
  protocol change, and a slice (§5).
- **No Android phone yet.** Every Android result until there is one comes from an emulator, and
  R3 cannot close without a physical one (§7). Android-only trouble, such as xterm.js's input
  under some Android keyboards, may surface late.
- **iOS is built only on the Mac.** Each iOS smoke test needs it at hand.
- **Another app claiming `yardsort://`.** It could catch the sign-in redirect, but not use what it
  carries without the app's verifier (§5). It could catch a pairing link, whose secret it could
  redeem only if signed in to the same account, and then the person at the computer still has to
  approve it. A universal link would close even that for the managed service, though not for a
  self-hosted server (§5).
- **Expo's pace.** A new SDK every few months, each supported for about a year. R3 stays on the
  SDK it starts on, and moving up is a slice of its own.

## 9 · What the first slice adds

Built on 2026-10-31, on Linux.

- **`packages/remote-client`**, its own Bun project like `website/`: `LICENSE` with the MPL-2.0
  text, `"license": "MPL-2.0"` in `package.json`, and the MPL's short notice at the top of each
  file. No dependencies. Its only development tools are TypeScript 6.0.3, which the desktop
  already uses, and `@types/bun` 1.4.2, for the Bun that CI runs, both pinned exactly. It is new
  TypeScript, written after `remote-proto` and `remote-client`, and follows their documentation
  closely. All of that is the owner's, so the MPL here is the owner's choice for their own work
  (§6). Its README says what each file holds.
- **What it decides beyond §4:**
  - A decoded value writes back as Rust writes it: fields Rust always writes are there, `null`
    when empty, and fields it leaves out when empty are left out. That is what lets the fixtures
    pin both sides.
  - The bounds held to a computer's replies are the ones `remote-proto` publishes: ids,
    cursors, `MAX_TEXT`, `MAX_LABEL`, `MAX_NOTE`, `MAX_LINK`, `MAX_TASKS`, `MAX_TERMINALS` and
    `MAX_PAGE`, with characters counted as Rust counts them. Strings the protocol does not bound,
    such as an activity event's details or an error's message, are held only to the message's
    own 64 KiB. Secrets are at most 256 bytes.
  - **A bound only this side has:** numbers past 2^53 − 1 are refused rather than rounded, since
    a JavaScript number cannot hold them exactly. That covers sequence numbers, times and
    counts, and a frame whose sequence is past it fails as `seqTooLarge`. A terminal would have
    to print for longer than anything runs to get there.
  - What a newer peer adds is passed over: unknown fields, an event this build does not know, a
    frame of another kind or version. The enums that read an unknown value as `other` or
    `unknown` in Rust do the same here. Anything else that does not read closes the channel as
    `invalid`, rather than drawing a terminal wrong.
  - **An attachment checks every number.** A snapshot's frames carry the reply's position. A
    resumed stream goes on after it. A piece split over several frames shares one number. An
    item out of that order ends the attachment as `gap`, and the computer is told to detach it.
    `drawn` is the last position drawn, for resuming on another channel.
  - **Control** renews its lease every `renewEveryMs`, numbers input from 1, and never sends
    input again. Input in flight when the channel closes rejects with `ChannelClosed`: whether
    it arrived is not known.
  - **Liveness is as §4,** but a dead channel is only closed, as `silent`, never reopened by the
    client itself. Opening a new channel, and attaching again from `drawn`, belongs to the screens
    that know what was open (slice 5).
  - **`Session`:** one refresh in flight at a time, and `keep` finishes before the new access
    is handed out. When `keep` fails, the `NotKept` error carries the session, since by then
    the session holds the only copy of the new credential. A refresh that never got through is
    retried on the next call, inside the server's 30 seconds if the phone is still online
    (27 §21).
  - **`explainRevoked`** asks with the phone's own credential. A grant revoked, denied, expired
    or not found, or a computer missing from `GET /v1/hosts`, is `unpaired`. A refused credential
    is `signedOut`. Anything else is `undetermined`.
  - **Sign-in** is `beginSignIn(redirect)` and `codeFromRedirect`. The client never runs a
    listener; the app's redirect is the app's own scheme.
- **Fixtures.** `frame-output.json` and `frame-refused.json` pin binary frames on both sides:
  41 fixtures in all. Each side now fails on a fixture it does not read. In Rust,
  `every_fixture_is_read_here` looks for each file's name in the tests. In TypeScript, the
  conformance table has to list exactly the directory.
- **`crates/server/examples/stand-in.rs`**, AGPL like the server. It runs the server with its
  records in memory, the identity provider the server's tests use (it gains an `/authorize` that
  signs a chosen subject in at once), and a real host: the app's connector, the core and a
  program in a real terminal. It is controlled by a line of JSON at a time on stdin. Every
  client of it comes from one address, so its limits per address are raised. The server's own
  tests hold it to the real limits.
- **The pairing tool's twin,** `tools/pair.ts` (`just client-pair`), as listed in CONTRIBUTING.
  It never prints a credential, a ticket, the link's secret or a conversation's title.
- **Tests: 106 under `bun test`**:
  - the conformance run (45);
  - decoding bounds;
  - the HTTP client and the session against small Bun servers;
  - the channel against a computer that misbehaves on purpose, over a real WebSocket;
  - 13 against the stand-in, covering sign-in through `yardsort://signed-in`, pairing by code
    and by link, a denial, another account's code, typing and resuming, following activity, a
    viewer refused control, and each of §5's answers to a 4003 (a grant taken back, a computer
    removed, the phone revoked, a server that cannot be asked);
  - the twin, run whole.

  `just check` runs them (`test-client`, and `typecheck` now covers the package), and CI runs
  them on all three systems after the Rust tests, which build the stand-in. Mutating the
  decoders, the numbering check and the probe each made a test fail.

- **Not shown yet.** Nothing has run on a phone. The package has not run under React Native's
  Hermes: injecting `crypto` and avoiding `URL` are meant for it, and slice 2 shows whether
  they were enough.

**Its smoke test, on this machine:**

1. `just server-up`, then `just server-run`.
2. The app, from this branch, with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote: server `http://127.0.0.1:8787`, **Turn on and sign in** as
   `developer@yardsort.test` with `Yardsort-dev-1`. Add a project and open a shell in a workspace.
3. **Pair a device**. In another terminal, in `packages/remote-client` (through `just`, `--type`
   takes one word), `bun tools/pair.ts <code> --type "echo hello-from-typescript" --wait-for-revoke`,
   and sign in again in the browser. **Let it watch and type.** The tool says it watches the
   shell, and types. The shell in the app shows `hello-from-typescript`. The tool says it resumed
   from where it left off, and that the items came numbered without a gap.
4. **Revoke** it in the app. The tool says the computer took back its grant and it is still
   signed in, then signs out.
5. `just client-pair devices`, signing in once more, lists the first run's device as revoked,
   the app's computer as signed in, and itself as this one.

## 10 · Open after this plan

- **A physical Android phone** for the smoke tests and the device run (§7). The owner's to
  provide; R3 cannot close without one.
