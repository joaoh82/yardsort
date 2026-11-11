# Remote, R3: the phone app

**Status:** planned on 2026-10-30. The first slice, the TypeScript protocol client, was built on
2026-10-31 (§9). The second, the app signed in, was built the same day (§10), and its smoke
test passed on the owner's iPhone against staging on 2026-11-01. The third, pairing and computers,
was built on 2026-11-01 (§11), and its smoke test passed on the same iPhone that day. The fourth,
workspaces, sessions and activity, was built the same day (§12), and its smoke test passed on the
same iPhone on 2026-11-02. The fifth, watching a terminal, was built that day too (§13), and its smoke
test passed on the same iPhone that day. The sixth, taking control, was built the same day (§14), and
its smoke test passed on the same iPhone that day, after it found a tab at the computer taking control.
The seventh, signed out, revoked and switching, was built on 2026-11-03 (§15), and its smoke test passed on
the same iPhone that day. That is the last slice, and R3 is not done: the device run below is its gate, and
needs an Android phone. Android was built and run in an emulator on 2026-11-03, which found three bugs, now fixed
(§16). That is not the device run. The run's iPhone half is written down (§17), and has not been run.
This is R3's plan: what it has to show, what it starts from, the decisions taken before any app code exists, the
slices, and what each slice's smoke test needs. The phone app's license was decided with it: MPL-2.0 (§6). Like
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
- store listings, public store builds or billing (R7). Every build in R3 is a development build on
  the owner's own devices, except that on 2026-11-04 the owner moved one thing earlier so as to start
  using the app: **internal TestFlight builds**, archived in Xcode with no EAS
  ([the README](../../apps/mobile/README.md#on-testflight)), or, since 2026-11-06, by the TestFlight
  workflow on GitHub Actions, signed by hand from a distribution certificate and an App Store profile
  held as secrets. It first ran on 2026-11-07, with Xcode 26.6, and its build reached the owner's
  iPhone. Its version, kept in `mobile-v` tags, is raised by each run by the part picked; the
  builds before said 0.1.0. External testers and listings stay in R7.
  The release identity was first built that day, for Android, and run on the emulator: its code is
  embedded, it shows no developer tools, and it looked up staging. The first TestFlight build followed
  the same day: archived in Xcode 27, uploaded, installed on the owner's iPhone, signed in and paired.
  Android's generated permissions (§16) still want blocking before any build goes to a store;
- editing code, Git writes, launching a new agent, approving anything on an agent's behalf, or
  approving memory. The proposal excludes them from the first mobile scope (§3.4). Reading
  approved memory is in scope, as `memory.list` already serves it. Launching an agent, in a new
  workspace or an existing one, and giving one a picture, came in after first real use, at the
  owner's request (§19).

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
| Terminal        | `react-native-webview` 14 with `@xterm/xterm` 6 and its Unicode 11 and web links addons                                                           |
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
- **Liveness without pings.** While the app is in the foreground, a channel that has heard
  nothing for 20 seconds sends `host.describe`. One that still hears nothing within 10 seconds is
  treated as dead, closed, and opened again. Anything that arrives counts: a reply, an event,
  output. What is sent does not, so a lease's renewals going unanswered end the channel too.
  (Planned first as "nothing sent", with renewals as traffic; review of #150 found that
  unanswered renewals then kept a dead channel open forever.)

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
assumed. Tried on 2026-11-02 with both on Linux, started from a shell under a lease, up to their
first screen: neither was held up (§14). The device run still has the last word.

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
something before the network answers. It is a file in the phone's cache folder, not SecureStore,
which is meant for small secrets: names and branches are not credentials, the folder is never
backed up, and the system clearing it costs only what was last seen. No terminal output is kept, and no activity, memory or
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

The owner changed this on 2026-11-04. The iPhone repeats only what its smoke tests never ran (§17), the budgets and an external keyboard
move to R4 on both phones (§18), and Android, with no smoke tests of its own on a phone, still does the rest of the list above.

## 8 · Risks

- **Terminal input on Android.** xterm.js's own input misbehaves under some Android keyboards.
  Typing through the composer avoids it, at the cost of typing into the terminal directly; the
  device run says whether that can be offered too.
- **Programs that wait for an answer.** If Claude or Codex waits on a terminal question that no
  viewer answers while a phone controls, the controller's viewer has to become the responder: a
  protocol change, and a slice (§5). Neither did on the way to its first screen (§14). A question
  asked later in a session is for the device run to find.
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
    `drawn` is the last position drawn whole, for resuming on another channel. Every frame of a
    split piece but its last is full (`remote_proto::terminal` now says so), so a piece is whole
    once a shorter frame or the next item comes. While the last piece may have more to come,
    `drawn` is `null` and the next attachment starts from a snapshot: resuming after a piece cut
    short would lose its end.
  - **Control** renews its lease every `renewEveryMs`, numbers input from 1, and never sends
    input again. Input in flight when the channel closes rejects with `ChannelClosed`: whether
    it arrived is not known. Input refused before it is sent, too large or not a request a host
    would take, takes no number, since the host takes only the next one.
  - **Liveness is as §4,** but a dead channel is only closed, as `silent`, never reopened by the
    client itself. Opening a new channel, and attaching again from `drawn`, belongs to the screens
    that know what was open (slice 5).
  - **`Session`:** one refresh in flight at a time, and `keep` finishes before the new access
    is handed out. When `keep` fails, the `NotKept` error carries the session, since by then
    the session holds the only copy of the new credential. A refresh that never got through is
    retried on the next call, inside the server's 30 seconds if the phone is still online
    (27 §21). `beforeRefresh` runs before every refresh credential is sent, and when it throws,
    nothing is sent and the credential is not spent. `accessAfterRefusal` replaces access the
    server refused, once for everyone refused it. Its clock is monotonic unless given one.
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
- **Tests: 110 under `bun test`**:
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

- **What review of #150 found, and fixed:** unanswered renewals counted as traffic and kept a
  dead channel open; input refused here still used up a number, so the host refused everything
  typed after it; and `drawn` named a piece whose later frames never came, so resuming lost the
  rest of it. Each has a test that fails without the fix, and the host now has one for how it
  splits a piece.

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

## 10 · What the second slice adds

Built on 2026-10-31, on Linux. Its smoke test, below, passed on 2026-11-01: a development build on
the owner's iPhone, built on the Mac, against staging. That is the first time the client ran under
Hermes, SecureStore kept a credential, and a sign-in came back through `yardsort://signed-in`.
Android is still untried. The review after it changed how refreshes and refused access are handled
(below). The smoke test was run again on the phone with those changes the same day, with one step
added: the phone locked for at least 16 minutes, past the 15 minutes access lasts, and a pull to
refresh on waking stays signed in. Another server taking over an address, and an answer arriving
after a switch, are shown against the real server through the stand-in, not on the phone.

- **`apps/mobile`**, its own Bun project (§3): `LICENSE` with the MPL-2.0 text, `"license":
"MPL-2.0"`, the MPL's short notice in each file, and a README on running it. Expo SDK 58, every
  package pinned exactly as Expo's installer chose it: `expo` 58.0.22, `expo-router` 58.0.29,
  `react` 19.3.0, `react-native` 0.88.1, `expo-secure-store` 58.0.2, `expo-web-browser`
  58.0.7, `expo-crypto` 58.0.7, `expo-dev-client` 58.0.18, `expo-device` 58.0.5,
  `react-native-safe-area-context` 5.10.1 and `react-native-screens` 4.30.1, among others in its
  `package.json`. `ios/` and `android/` are generated when a build needs them, never committed.
  The development build is `Yardsort Dev`, `dev.yardsort.mobile.development`; a release build
  would be `dev.yardsort.mobile`, and nothing in R3 makes one.
- **The screens:** Connect, the account (its devices, with this phone marked, and signing out),
  and the servers on the phone (one in use, another added through Connect). Native components
  only; text follows the system's size, and every control has a label for screen readers.
- **Connect takes a server's address.** It does not offer the managed service, since production
  is not set up (28), as the desktop's Remote settings do not. Looking a server up sends only
  `GET /v1/server`. It shows the server's name, its address, who it signs in with, that the
  account there is that server's own, and that whoever runs it can see what passes through, not
  end to end (proposal §9). Signing in names the phone; iOS tells an app only "iPhone", so the
  field starts with that.
- **What it decides beyond §5:**
  - **A profile is a server's address and the id it gave.** Before every refresh credential goes
    to an address, on opening a profile or later in a session already open, the app asks the
    address who it is. If it now answers as another server, nothing is sent; the app says so, and
    the person can remove the profile. A server's id is what the server says, so this alone is no
    defence against a server that lies; it keeps a credential from going to a server set up
    again, or moved, at the same address. It covers the refresh credential, which is what lasts.
    An access credential, which lasts 15 minutes, goes with each request without the question, so
    until a refresh is due a replacement can be sent one.
  - Signing in where the phone is signed in already opens that profile instead.
  - A sign-in whose credential cannot be kept is revoked at once, with the access that came with
    it, so no device is left at the server that nothing holds.
  - A refresh credential the server refuses means this phone was signed out there: the profile
    is deleted and the phone says so. A server out of reach deletes nothing, and is tried again.
  - A refused access credential means less, because the server answers the same for access that
    has lapsed as for a device signed out. So it is refreshed and the request tried once more;
    only a refusal after that deletes anything. Deleting on the first refusal would lose the only
    credential able to sign the phone out, with the device still active at the server.
  - Sessions run on the wall clock, since the steady clock may stop while the phone sleeps. One
    that is wrong either way costs a refresh, through the retry above.
  - An answer that arrives after switching to another server changes nothing about the profile
    now in use. The account screen starts afresh for each profile, so one server's devices never
    show under another's name.
  - Signing out revokes the phone at the server. When the server is out of reach, the phone
    stays signed in unless the person removes it from this phone only, and is told the server
    still counts it until another device revokes it.
  - Opening a profile is one at a time, since it spends the refresh credential: a second caller
    waits for the first.
  - Switching back to a profile opened since the app started reuses its session.
- **Tests: 40.** The logic, 30 under `bun test`, against the real server and its sign-in through
  the stand-in (two of them at once, for two servers): signing in, a closed browser, a sign-in
  that cannot be kept, opening after a restart, opening twice at once, an address that answers as
  another server, a server out of reach, signed out from another device on opening and while open,
  signing out, two servers, access refused and access past its time, another server at the address
  of a session already open, and answers that come after a switch. The screens, 10 under Jest with
  `jest-expo` and Expo Router's in-memory router, against a small fake server whose answers the
  real client decodes: Connect's lookup and what it says, a bad address, signing in, a closed
  sign-in, a server signed in to already, signing out with a no respected, signing out out of
  reach, signed out at the server, out of reach on opening, and one server's devices under
  another's name. Breaking the address check, the one-at-a-time opening, the order
  credential-then-list, the question before signing out, the wall clock, the check on a resume or
  on a later refresh, the retry of refused access, or leaving another profile's standing alone
  each failed a test.
  Two runners, because the screens need React Native's transforms and the logic needs the
  server, which Bun starts as the client's tests do.
- **Lint is Expo's** (`eslint-config-expo`), on ESLint 9.39.5: its React plugin fails to load
  under the ESLint 10 the desktop uses. The desktop's lint skips `apps/mobile`, and Vitest
  skips it too.
- `just check` runs the app's typecheck, lint and tests (`test-mobile`); CI runs them on all three
  systems after the client's. `just mobile-ios` builds and installs the development app on an
  iPhone from a Mac, and `just mobile-start` serves its code to one already installed.
- **What was shown without a phone:** Metro bundles the app for iOS and Android, and Hermes
  compiles both bundles. `expo-doctor` passes its 20 checks. `expo prebuild` generates both
  native projects with the `yardsort` scheme and the development identifier. None of that runs
  the app: whether the client works under Hermes, SecureStore keeps what it is given, and the
  sign-in session comes back through `yardsort://signed-in` is what the smoke test shows.
- **Not built here:** iOS needs macOS and Xcode, and this machine had no Android SDK, so neither
  app was built. Android stays untried until it is, in an emulator first (§7). It was, on 2026-11-03 (§16).

**Its smoke test, on the iPhone against staging:**

1. On the Mac, from this branch, with the iPhone plugged in and unlocked: `just mobile-ios`.
   Choose the iPhone, and the paid team. `Yardsort Dev` opens on the phone.
2. **Connect:** `https://remote-staging.yardsort.sh`, **Look up**. It shows staging's name, its
   address, the ZITADEL host it signs in with, and that whoever runs it can see what passes
   through.
3. Name the phone and **Sign in**. iOS asks to let the app sign in with the website; continue,
   and sign in with your staging account. Back in the app: "Signed in to … as …", and the
   account's devices with this one marked **This phone**.
4. Close the app from the app switcher and open it again: it is still signed in, with no browser.
5. Lock the phone for at least 16 minutes, past the 15 minutes access lasts, then pull to refresh
   on the devices: still signed in.
6. **Sign out**, then **Cancel**: still signed in. **Sign out** again, and confirm: Connect.
   Confirming is what revokes the phone at the server.
7. From the repository, on any computer with a browser:
   `just client-pair devices https://remote-staging.yardsort.sh`, signing in in the browser with
   the same account. It lists the phone as revoked, because step 6 revoked it.

The Mac needs Bun, Node.js and CocoaPods besides Xcode, not Rust. A fresh one also needs Xcode's
first-launch components, an Apple Development certificate, Apple's WWDR G3 intermediate and
`codesign` allowed to use the key; `apps/mobile/README.md` has each step and the error it ends.
On the first run, the list showed the phone still signed in: the second **Sign out** had not been
confirmed. Confirmed, it showed it revoked.

## 11 · What the third slice adds

Built on 2026-11-01, on Linux. Its smoke test, below, passed on 2026-11-01: a development build on
the owner's iPhone against staging, with the desktop app as the host, once the two things its first
run found were fixed (below). That was the first time a channel opened under React Native. A link
naming another server is shown only by tests, since the smoke test has no second server to name,
and Android stays unbuilt. Step 10 came from review, after that run, and passed on the same phone
the same day. The server needed no change, so staging needs no deploy; the stand-in gained `hostOffline` and
`hostOnline`, for its host to drop its link to the relay and make it again.

- **Pairing** (`src/app/pair.tsx`, `src/lib/pairing.ts`): scan the computer's code with
  `expo-camera`, which opens only when asked for and asks for no microphone, or type its eight
  characters. The app redeems the offer, then asks for its grant every two seconds until the
  person at the computer answers. It asks for every scope a phone uses, and says what it was given.
  A no, a request that ran out, and a wrong or expired code each say so.
- **A link keeps its secret out of the router.** `yardsort://pair` is taken in `+native-intent.ts`
  before the router reads it, and held in memory in `Incoming`: routes, history and params only
  ever see `/pair`, and nothing is written to disk. A link lost to a restart costs a new scan.
- **A link is never used on its own say.**
  - Scanned in the app, the person chose it, so it goes at once.
  - From outside the app, whoever sent it is unknown, so the app asks first.
  - Naming a server this phone is signed in to, not the one in use, it asks before switching.
  - Naming one it is not signed in to, it asks, then goes through Connect, with what that server
    can see, before anything is sent there. Nothing of one server goes to another.
  - Naming an address the app will not use, plain `http` to a remote host for one, it goes nowhere.
- **An offer goes once**, so `Incoming` hands a link out once, however many ask. A camera reports
  the same code many times a second, and an effect may run again. The pair screen is
  `dangerouslySingular`, and signing in returns to the one already open. The first version opened
  a second, and two screens could each have spent the offer; a test found that.
- **A link arriving while the screen is busy** is asked about first, whatever the screen was doing:
  sending another, waiting for a computer, or showing an answer. A no goes back to that. A yes goes
  on, and an earlier request stays open at the server, where the account screen can cancel it until
  it runs out. Letting go of a sent link leaves a newer one that replaced it, and only the newest
  request moves the screen on. Review found the first version losing or hiding the newer link.
- **Computers** (`src/ui/computers.tsx`) come first on the account screen: approved grants whose
  computer is still on the account, requests still waiting, which can be cancelled, and **Pair a
  computer**. **Unpair** asks first, and the phone stays signed in.
  (**Unpair** moved to the computer's own screen on 2026-11-04, [33](33-phone-design.md). Later steps
  in this document that find it in the list were written before that.)
- **Online is what the socket says** (`src/lib/presence.ts`, §5). A computer is online while a
  channel to it is open, offline with when it was last seen when the relay says so, and
  reconnecting in between, waiting 2 seconds, doubling up to 30. An offline computer is asked
  about again every 15 seconds. A computer that no longer lets this phone in (4003, 4010,
  `forbidden`, `not_found`) is not asked again until the next start, and the list is read again.
  Channels are held only while the list is on screen and the app is in the foreground.
- **Left for later:** the small cache of computers (§5) comes with slice 4's workspaces, and telling
  apart everything 4003 can mean comes with slice 7 (§15). For now a 4003 reads the list again, which
  drops a computer that took back its grant and signs out a phone that was revoked.
- **Tests: 84.** The logic, 54 under `bun test`, 24 of them new, against the real server, relay and
  host: pairing by code and by link, a no, a wrong code, an offer going once, a request taken back,
  unpairing, waiting through a server out of reach, where a link belongs, online, offline before
  and after opening, coming back, a grant taken back at the computer, and channels closed on
  stopping. The screens, 30 under Jest, 20 of them new. Breaking any of these failed a test:
  closing channels on stopping, offline read from the close or from opening, not asking again
  about a computer gone, where a link belongs, a link handed out once, the question before using
  a link, keeping the secret from the router, returning to the pair screen, and holding channels
  only on screen.
- **What was shown without a phone:** Metro bundles the app for iOS and Android, Hermes compiles
  both, `expo-doctor` passes its 20 checks, and `expo prebuild` writes the camera's reason into
  `Info.plist` with no microphone. None of that runs the app: the scanner, a link from the system
  camera, and `WebSocket` under React Native were shown by the smoke test.
- **Not built here:** as in §10. This adds a native module, so a phone needs a new build, not only
  new code from Metro.
- **Found on the first run of the smoke test, on the iPhone:** two things no check here could see.
  `expo run:ios` does not apply `app.config.ts` to an `ios/` that already exists, so a build over
  slice 2's project linked the camera without its `Info.plist` reason, and the camera stopped the
  app with a red error. `just mobile-ios` now runs `expo prebuild` first. With a project generated
  here and its reason removed, that put the reason back. It also clears the signing team, which
  `expo run:ios` sets again: by itself with one signing identity on the Mac, and by asking, last
  choice first, with more. And from iOS 26 the header is glass laid over the screen, so every
  screen began underneath it, since a React Native scroll view does not leave room for a header
  unless told to. It now does.

**Its smoke test, on the iPhone against staging,** with the desktop app on a computer as the host:

1. On the computer, from this branch, with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote: server `https://remote-staging.yardsort.sh`, **Turn on and sign in** with the
   staging account.
2. On the Mac, from this branch: `just mobile-ios`. Sign in on the phone with the same account. The
   account screen says no computer lets this phone in yet.
3. **By typing.** On the computer, **Pair a device**. On the phone, **Pair a computer**, type the
   code. It says to say yes at the computer. Let it watch and type. The phone says it was let in and
   what it may do; **Done**, and the computer is listed as **Online**.
4. **Unpair**, on the computer's own screen since [33](33-phone-design.md). **Unpair**, then **Cancel**: still listed. **Unpair** and confirm: gone from the
   phone and from the computer's list, and the phone is still signed in.
5. **By scanning.** **Pair a device**, **Pair a computer**, **Scan the code**, allow the camera, and
   point it at the code. Deny it at the computer: the phone says the computer said no.
6. **From the system camera.** **Pair a device**. Scan the code with the iPhone's Camera app, and
   open the link. The app asks first. **Ask the computer**, and approve it. **Online.**
7. **Offline.** Quit the desktop app. Within seconds the phone shows **Offline, last seen …**, and
   that its agents did not stop. Start it again: **Online** within about 15 seconds.
8. **Taken back at the computer.** **Revoke** it in the desktop app. The computer goes from the
   phone's list, and the phone is still signed in.
9. **A wrong or expired code.** Type `ABCD-2345`: it says the code is wrong or has expired. Show a
   code at the computer, wait out its five minutes, and type it: the same.
10. **A second link while waiting.** **Pair a device**, then type the code on the phone, which says
    to say yes at the computer. Don't answer. On the computer, **Done**, then **Pair a device**
    again, and open that code's link with the iPhone's Camera app. The phone asks first, and says
    the first request stays open. **Cancel**: it is back to waiting for the first. Open the link
    again, then **Ask the computer**. The computer shows two requests; answer the newest and the
    phone follows it. The first is on the account screen, and **Cancel the request** removes it.

## 12 · What the fourth slice adds

Built on 2026-11-01, on Linux. Its smoke test, below, passed on the owner's iPhone against staging
on 2026-11-02, the first use of the real cache file on a phone. Step 5 first failed as written, and
the fault was the step's: it did not turn capture on, so the agent's prompts and tools were never
recorded and the phone was right to show only its start. The step is corrected, and with capture on
the rows came in. The screen's line saying an agent reports only its start and end was written after
that failure and was not on the phone for the test, so it has only run under tests. Android is still
unbuilt. The server and the host needed no change, so staging needs no deploy; the stand-in
gained `note`, for a test to read an approved note through the real host. Everything here is read
through a channel, and a phone changes nothing on the computer.

- **A computer** (`src/app/computer.tsx`): its workspaces by project, with the branch and how many
  conversations are running. A page at a time, and more when asked for.
- **A workspace** (`src/app/workspace.tsx`): its conversations, ended ones when asked for, its other
  terminals, and its activity. A terminal's state comes from when it last printed, so a quiet one
  says "Nothing printed for 3 min" and never that it is waiting for anything. Opening a terminal is
  slice 5.
- **Activity is followed live** (`src/lib/activity.ts`): the newest page, then a subscription after
  it. A channel lost carries on from the last batch, so nothing still kept is missed and nothing
  repeats. Older pages come when asked for. It says what the desktop says of each event, and a kind
  this build does not know by its name. At most 300 events are held in memory. An agent reports its
  prompts and tools only if its capture was on at the computer when it started, which by default it
  is not. Then only its start and end are recorded, and the screen says so and where to turn it on,
  so that a short list does not read as an agent doing nothing.
- **Approved memory** (`src/app/memory.tsx`): a project's approved notes, to read, with no way to
  change one from here. The host serves only approved notes (§1).
- **What was last seen** (`src/lib/observed.ts`): the computers and each one's first page of
  workspaces, in a file per profile in the cache folder (§5). The account shows the computers before
  the server answers or when it cannot; a computer's screen shows its workspaces, marked "As of",
  until it answers or while it cannot. It holds names and branches, and never a session title, a
  task's title, activity, memory or a grant's scopes. A failed read or write costs only that.
- **What was last seen goes** with a computer that took this phone back, which the next list from
  the server settles, and with the profile. A server out of reach deletes nothing. A computer that
  refuses a channel takes its screen with it at once; telling apart everything a 4003 can mean stays
  with slice 7 (§15).
- **One channel at a time:** each screen holds its own while it is the one shown (`src/ui/live.tsx`),
  with the same reach, retries and foreground rule as the account's list. Coming back to a screen, or
  pulling it, reads again.
- **What a grant does not reach** says so in the computer's own words, whichever list it stops.
- **Not built:** a workspace's linked tasks are not shown. No conversation is kept, so a list of
  them shows only when the computer can be reached, marked "As of" if the channel drops while it is
  open.
- **What was tested:** the logic against the real server, relay and host: reading and following,
  carrying on after a dropped channel, stopping, a grant that does not reach activity or memory,
  and what is kept going with a sign-out, a computer's refusal and nothing with a server out of
  reach. The screens against a fake relay: the lists, paging, live events, the same event told
  twice, memory with nothing that changes it, a cold start with the server down, and nothing but
  names and branches on the shelf. Breaking each of those made a test fail. Doing that found a test
  whose program exited after the phone had reconnected, so it proved nothing, and a real double
  subscription when a pull to refresh came during the first read; both are fixed.
- **What was shown without a phone:** Metro bundles the app for iOS and Android, and `expo-doctor`
  passes its 20 checks. Tests use a map for the cache file, so only the smoke test used a real one.
- **Not built here:** as in §10.

**Its smoke test, on the iPhone against staging,** with the desktop app on a computer as the host:

1. On the computer, from this branch, with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote: server `https://remote-staging.yardsort.sh`, **Turn on and sign in**. Add a
   project and a workspace with a branch. Settings → General → Activity: turn on **Capture what
   Claude Code reports** and **Save activity settings**, since an agent reports its prompts and tools
   only if this was on when it started. Start a Claude agent in the workspace, open a shell in it,
   and write a note in the project's Memory view.
2. On the Mac, from this branch: `just mobile-ios`. Pair the phone if it is not already, letting it
   watch and read everything.
3. **A computer.** **Open** the computer. It says **Online**, with the workspace under its project,
   its branch and **1 running**.
4. **A workspace.** Tap it. The conversation shows with its title and agent, and the shell under
   other terminals. **Show ended conversations** shows any that ended.
5. **Live.** With the phone on that screen, send the agent a prompt that makes it use a tool. Rows
   such as **prompt submitted** and the tool's come in under Activity without touching the phone.
6. **Away and back.** Lock the phone for a minute while the agent does something else. Unlock it.
   What happened meanwhile is there, once each.
7. **Memory.** **Approved memory** shows the note, with nothing to change it. A proposal waiting for
   review does not show.
8. **Offline.** Back on the computer's screen, quit the desktop app. It says **Offline, last seen
   …**, and that its agents did not stop. Leave and reopen the screen: the workspace shows, marked
   **As of**. Start the app again: **Online** within about 15 seconds.
9. **No network.** Turn on airplane mode and force-quit the app. Open it: it says the server could
   not be reached, and the computer shows marked **As of**. Open it: the workspace shows, marked **As
   of**, and tapping it says conversations show when the computer can be reached. Turn airplane mode
   off and **Try again**.
10. **Taken back.** With the computer's screen open, **Revoke** the phone in the desktop app. The
    screen says the computer no longer lets this phone in. **Back to the account**: the computer is
    gone, and the phone is still signed in.
11. **Signed out.** Pair again, open the computer, then **Sign out** and confirm. Sign in again: no
    computer shows until the phone is paired again.

## 13 · What the fifth slice adds

Built on 2026-11-02, on Linux. Its smoke test, below, passed on the owner's iPhone against staging
that day, the first time WebKit drew the page. Android is still unbuilt. A new native module means
`just mobile-ios` again, not `just mobile-start`. The server and the host needed no change, so
staging needs no deploy. Nothing on this screen reaches the program: taking control is slice 6.

- **Opening one** (`src/app/workspace.tsx`): a conversation with a terminal, or another terminal, is
  now a button. The route carries the host, the terminal and the agent's name, never a conversation's
  title, which is the person's own words (§5).
- **The page** (`terminal/page.ts`, built by `scripts/terminal-page.ts` into
  `src/ui/terminal-page.generated.ts`): one document with xterm.js inlined. A test fails when it is
  out of date, and `just mobile-terminal-page` builds it. The build is the same on the Bun CI pins and
  on a later one. Its Content-Security-Policy allows its own script by hash and connects to nothing.
  xterm.js's input is read-only, with stdin disabled and nothing listening for what it would answer.
  OSC 52 goes nowhere, as the addon that would carry it out is not there.
- **The bridge** (`src/lib/bridge.ts`): the only thing the two sides share. Each message is bounded
  and read by a checker on the side receiving it: `reset`, `output` and `resize` in, `ready`, `drawn`,
  `link` and `trouble` out. `trouble` says which, never what the terminal printed.
- **The WebView** (`src/ui/terminal-view.tsx`): it goes nowhere but its own document and keeps
  nothing. A link must be `https` and tapped, and then the person sees where it leads and says yes,
  since a link's text in a terminal can say anything. If the system ends the WebView's process, a new
  one starts and gets a new snapshot.
- **Watching** (`src/lib/watch.ts`), through the screen's channel, whose retries and foreground rule
  are the same as every other screen's:
  - A dropped channel, or the app in the background, attaches again from the last position drawn,
    and the computer says whether that goes on or a snapshot replaces the screen.
  - 4008 and `resnapshot_required` start from a snapshot, and so does a gap. A position refused on
    resuming falls back to one too.
  - Five fresh starts stop and say so, unless the page caught up on output past the snapshot in
    between. A snapshot's own frames do not count, since one always comes, and neither does new output
    alone, which a program outrunning the page has plenty of.
  - Nothing is sent to the program by any of this.
- **Holding output back** (`PageFeed`): at most 256 KB is with the page, undrawn. Up to 4 MB waits in
  the app, a size change keeping its place among it, and past that the page starts over from a new
  snapshot, which counts as a fresh start. A WebView's queue of messages would otherwise have no
  bottom, and a phone draws slower than a program can print.
- **Scrolling:** at the computer's size, the document moves sideways under a finger. xterm.js scrolls
  only by wheel, so one finger moving up or down goes through what scrolled off, once the page itself
  can move no further. Pinching zooms in, not out: allowing out made Chromium lay the page out nearly
  3,000 pixels tall, with empty space to pan into.
- **What differs from §3:** the web links addon is here, or a plain address printed by an agent could
  not be tapped.
- **What was tested:**
  - The logic against the real server, relay and host, with a real program in a real terminal: a
    snapshot at the computer's size, resuming with nothing cleared, lost or drawn twice, and nothing
    typed by coming back, a page started anew, a program's end, a terminal that is gone, and a grant
    that does not reach terminals.
  - What the host cannot be made to do on demand against a scripted channel: a new epoch, 4008,
    `resnapshot_required`, repeated gaps and output arriving before its reply.
  - The page, as built, in real headless Chromium at a phone's size, with real touch and mouse events:
    what it draws, order against a reset or a resize, sideways and back through what scrolled off, no
    question answered, no typing taken, links only when tapped and only `https`, and nothing reaching
    a real server beside it. Skipped, saying so, where no browser is found. It forces a frame on each
    look, because a headless tab need not paint on its own. It passed eight runs in a row, three of
    them with every core busy.
  - The screens against a fake relay and a stand-in WebView.
- **Breaking each behaviour made a test fail,** 38 ways, and 13 more after review. Doing that found four tests that proved
  nothing: a reset and a size change sent too slowly to race the output before them, a check for
  answers that a second layer already stopped, and a check that the page reaches nothing that passed
  because nothing was listening. All four are fixed. Writing the tests found a real bug too: a write
  larger than the page's allowance would have waited forever. Review then found that giving up never
  happened against the real host: every snapshot comes as a frame, which cleared the count, and the
  scripted channel sent none. It now sends one as the host does, and starting over counts too. That
  came after the smoke test, so giving up has only run under tests.
- **What was shown without a phone:** Metro bundles the app for iOS and Android, and `expo-doctor`
  passes its 20 checks. Chromium is not a phone's WebView, so only the smoke test tried WebKit's
  drawing, touch and scrolling.
- **Not built here:** as in §10.

**Its smoke test, on the iPhone against staging,** with the desktop app on a computer as the host:

1. On the computer, from this branch, with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote: server `https://remote-staging.yardsort.sh`, **Turn on and sign in**. Add a
   project and a workspace with a branch. Start a Claude agent in it, open a shell in it, and make the
   window wide.
2. On the Mac, from this branch: `just mobile-ios`, since this slice adds a native module. Pair the
   phone if it is not already, letting it watch and read everything.
3. **Opening one.** Open the computer, then the workspace, then tap the conversation. Claude's screen
   shows as it is on the computer, in colour, its top line clear of the header, with **Watching** below.
4. **Moving around.** Drag sideways: the rest of the wide screen comes into view. Drag down: what
   scrolled off the top comes back. Drag up again to return. Pinching zooms in.
5. **Live.** Send the agent a prompt at the computer. The phone follows within a second or so, with no
   corrupt screen.
6. **Nothing reaches the program.** Tap the terminal. No keyboard comes up, and nothing changes at the
   computer.
7. **A full-screen program.** Go back and open the shell, then run `vim` or `htop` at the computer. It
   draws whole on the phone, and leaving it brings the shell back.
8. **A new size.** Make the desktop window narrower. The phone redraws at the new size.
9. **A short drop.** Turn on airplane mode for a few seconds, while the agent or the shell prints
   something. **Reconnecting**, then **Watching** within about 30 seconds of turning it off. The screen
   is intact, what was printed meanwhile is there, and nothing was typed at the computer.
10. **Away and back.** Send the app to the background for a minute while something prints, then come
    back. Lock the phone and unlock it the same way. It resumes both times.
11. **A link.** Have something print `https://example.com`, then tap it. The phone asks first, showing
    the address. Cancel, and nothing opens.
12. **A program's end.** In the shell, run `exit`. The phone says **The program ended**.
13. **Taken back.** With a terminal open, click **Revoke** at the computer. The phone says the computer
    no longer lets it in, and **Back to the account** works, still signed in.

## 14 · What the sixth slice adds

Built on 2026-11-02, on Linux. Its smoke test, below, passed on the owner's iPhone against staging that
day, once it had found one bug, fixed before it passed; Android stays unbuilt. It adds no
native module, so `just mobile-start` serves it to the app already installed. The server needed no
change, so staging needs no deploy; the stand-in learned to type as the person at the computer does,
for the tests. The host changed once, for what the smoke test found (below).

- **Taking it** (`src/lib/control.ts`): only ever a person pressing **Take control**. Another device's
  control is named on the button, **Take over from …**, and pressing that is what asks to take over.
  The person at the computer cannot be overridden, as the host holds to: a refusal says they are
  typing, and to try again once they stop. A grant that only watches is told so. A computer may not
  have read the name of a device let in since, which is then called another device.
- **Holding it:** the client renews the lease. The moment the computer reports the person there or
  another device, the phone lets go, without waiting for the next renewal. Only a new report counts,
  since taking over starts from an old one naming the other device, and nobody is no loss by itself.
- **Letting go:** **Hand back**, leaving the screen, the app going to the background, or the program
  ending. The release goes before the channel closes, which is why the screen's hook for it comes
  before the channel's. Nothing takes control again by itself: a phone coming back is watching.
- **Typing** (`src/ui/composer.tsx`): a native field, with autocorrection off, since it turns `--` and
  quotes into something else in a command. It is multiline so that a paste keeps its lines, and Return
  sends instead of adding one.
  - A line goes as keystrokes, not as a paste, or vim in normal mode would take a command as text. Then
    Enter, after the same 150 ms the computer leaves for a first prompt, since a burst reads as a paste.
  - Several lines go as one paste, for the host to bracket if the program asked, then Enter.
  - An empty field sends Enter. More than one write's 32 KB stays in the field, and says so.
  - **Type** sends the field without Enter. A menu that takes one key, vim's normal mode, or a line to
    finish with Tab all need that, and an Enter nobody meant could answer the next question.
- **The keys** (`src/lib/keys.ts`): Esc, Tab, Ctrl, the arrows, Backspace and Enter. They go at once and
  leave the field alone.
  - Backspace is beyond §5's list: nothing else could delete what the program already has.
  - Ctrl is a toggle for the next thing pressed. A letter typed then is not put in the field: it sends
    the control character. An arrow becomes Ctrl and that arrow. Something with no control character is
    typed as it is.
  - Arrows follow what the program asked for. The page reads application cursor keys off xterm.js and
    says when they change, and a snapshot brings them back, as it does every input mode.
- **What may not have arrived:** one thing goes at a time, each after the last was answered. If the
  channel closes on one, it is shown, and never sent again by itself: **Put it back** puts text in the
  field, and **Dismiss** lets go of it. Text may also have arrived and be waiting for Enter, so it asks
  the person to look first.
- **What was waiting:** losing control drops whatever had not gone, Enter included, and says so. A
  refused lease is known not to have been typed, so it is not called uncertain.
- **What is in the field is not what was waiting.** A person's half-written text stays when control is
  lost, read-only, and goes only if they take control again and press Send. It is the person's, nothing
  sends it, and §5 keeps it across a reconnect, so the proposal's "clear unsent input on lease loss"
  is held to for what had been sent on its way, not for this.
- **Fitting** (`fitted`): with control, the terminal is resized to what fits the room above the field,
  and again when the keyboard or a rotation changes that, once it settles. Below 20 columns or 5 rows
  the size stays. Watching never resizes, and the computer's own size comes back with control (27 §8).
- **What differs from §3:** no fit addon. Only the app knows how much of the WebView can be seen, since
  from iOS 26 it starts under a see-through header. So the page says how large a character is, and the
  app works it out, taking off what the header covers.
- **The terminal-question check (§5):** Claude Code 2.1.302 and Codex 0.163.0, on Linux, started from a
  shell in a terminal held by a lease with no viewer shown any question, against tmux, which answers
  them. Both reached their first screen with nothing waiting on an answer. Four runs each, with
  throwaway config, so signed out:
  - Claude's screen settled at 424 to 961 ms under the lease, and 436 to 1,083 ms in tmux.
  - Codex's settled at 471 to 1,955 ms under the lease, and 549 to 760 ms in tmux. The 1,955 was its
    first run, cold, and the next three were all faster than tmux's.
  - So no change is needed for now. It shows nothing about signed-in sessions or questions asked later.
    The smoke test's step 9 tried signed-in agents on the phone and found nothing stuck; the device run
    still has questions asked later.
- **What was tested:**
  - The logic against the real server, relay and host, with a real program in a real terminal: a line
    with its Enter, lines pasted, keys in their place, the size fitted and given back, a take-over and
    being told of it, a dropped channel letting go and typing nothing, a grant that only watches, and
    the person at the computer typing.
  - What the host cannot be made to do on demand against a scripted lease: answers in order, closing on
    an input, a lease lost before Enter, and letting go while control is being asked for.
  - The page in real headless Chromium: the character size, what fits leaving nothing to scroll to, and
    application cursor keys.
  - The screens against a fake relay and a stand-in WebView.
- **Writing the tests found** that a terminal stood 5 pixels taller than its rows, from how its box sat
  on the line, which would have left a fitted page able to move. The real host found that a device can
  be nameless, which the fake had not shown.
- **Breaking each behaviour made a test fail,** 77 ways. Four first survived, each on a test too thin:
  a wait for the size to settle that nothing raced, a notice said twice, the field outliving a program's
  end with text in it, and a character's size said again unchanged. All four are fixed.
- **What was shown without a phone:** nothing about WebKit, the system keyboard, the keyboard moving
  the screen, or a real header: the fit's numbers come from a stand-in layout. The smoke test is the
  first time any of that runs.
- **Not built here:** direct typing into the terminal (§5 leaves that to the device run), a button to
  paste from the clipboard (the system's own menu pastes into the field, where it can be read first,
  and needs no new module), and selecting text to copy.

**What the smoke test found.** Selecting Claude's or Codex's tab at the computer took control from the
phone, and a shell's did not. Signed in, both agents ask for every pointer movement (`?1003h` with
`?1006h`), and a snapshot restores mouse modes, so a tab just selected reports the pointer as it moves.
Those reports came to the host as writes from this computer, and any such write took control. Focus
reports (`?1004h`) did the same in a terminal that saw the program ask for them; a snapshot does not
restore that mode. Now `pty_host::control::presence` tells these from typing, by the bytes real xterm.js
sends. Neither takes control or extends the person's hold. While a lease holds, the program still
hears about focus, and nothing of a pointer over a screen drawn at another device's size. A press, a
drag or the wheel goes to the program as something the person did, and takes control as typing does.
The fix runs in the terminal daemon, and a build of the same protocol keeps the daemon already running,
so trying it means stopping that daemon first (step 1).

**Found after it, in review and CI, and fixed without running on a phone again.** Two sizes went wrong
under particular timing and fonts. An answer to where the room starts could come after a newer layout
and bring back the size from before a keyboard settled, so it now says only where. And xterm.js rounds
the drawn screen to a whole pixel, so a character read from it came out differently at another column
count. The page now says it again only if rounding cannot explain the difference (`sameCell`). Every
monospace font tried on this machine is exactly 7.2 pixels wide, which hid that on Linux here; CI's Linux
and macOS failed. Its test now passes here with wider fonts substituted through fontconfig too.

**Its smoke test, on the iPhone against staging,** with the desktop app on a computer as the host:

1. On the computer, from this branch, with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote: server `https://remote-staging.yardsort.sh`, **Turn on and sign in**. In a
   workspace with a branch, start a Claude agent and a Codex agent, and open a shell. Make the window
   wide. A daemon from an earlier build of this slice keeps running through a rebuild: with `just dev`
   stopped, `pkill -f 'yardsort-daemon .*/tmp/ys31/'` ends it and that profile's agents first.
2. On the Mac, from this branch: `just mobile-start`, and open Yardsort Dev on the phone. Nothing native
   changed. If the phone is not paired, pair it and allow it to control terminals at the computer.
3. **Taking it.** Open the computer, then the workspace, then Claude's conversation. It says **Watching.
   Nobody has control.** and nothing can be typed. Press **Take control**. It says **You have control**,
   and the screen redraws to fit the phone, its top line clear of the header and its bottom line above
   the field, with nothing left to scroll sideways. The window on the computer shows the phone's size.
4. **Typing.** Tap the field. The keyboard comes up and the screen redraws to fit what is left, with
   Claude's prompt still in view. Type a short request and press return. Claude takes it and answers.
   Do the same with **Send** rather than return.
5. **The keys.** While Claude is working, press **Esc**: it stops. Ask it for something it has to ask
   permission for. Answer with **↑**, **↓** and **Enter**, or type the option's number in the field and
   press **Type**, which sends it without Enter. Then go back, open the shell and **Take control**:
   - Send `ls`, then press **↑**: the line comes back at the prompt. **⌫** deletes its last letter. Type
     that letter in the field, press **Type**, then **Enter** runs it.
   - Type `ech` in the field, press **Type**, then **Tab**: the shell completes it to `echo`. **Enter**
     runs it.
   - Send `sleep 100`, press **Ctrl**, then type `c` on the keyboard: it stops, and no `c` is in the field.
   - Send `vim /etc/hosts`. **↑**, **↓**, **←** and **→** move the cursor, with no letters typed. Then
     send `:q!`.
6. **Pasting.** Copy three lines from Notes. In the shell, run `cat`, long-press the field, **Paste**, and
   press **Send**: the lines come back whole. **Ctrl** then `d` ends `cat`. Open Claude's conversation,
   take control and do the same: Claude takes all three as one message, not three.
7. **Unicode.** Send `héllo 日本語 👍` to the shell's `cat`. It comes back unbroken. Try dictation.
8. **Codex.** Open its conversation and take control. Type to it, use the arrows in its prompt and press
   **Esc**.
9. **The terminal-question check.** With control of the shell, send `claude`, and answer what it shows
   until it is at its prompt. Press **Ctrl** then `c` twice to leave, then do the same with `codex`. Each
   comes up about as quickly as it does at the computer, with nothing stuck waiting.
10. **Taken back mid-typing.** With control of the shell, type something in the field without sending it.
    At the computer, type one key in that terminal. Within a second or so the phone says **The person at
    the computer took control.**, the keys go, and the text stays in the field, read-only. The window
    goes back to its own size. **Take control** straight away says they are typing. About 30 seconds
    after the last key at the computer it works, and **Send** sends the text then, once.
11. **Only being at the computer.** With the shell's tab showing at the computer, take control of Claude
    on the phone. At the computer, select Claude's tab, move the pointer over the terminal, and switch to
    another window and back. The phone keeps control throughout, and **Send** still works. Do the same with
    Codex. Then click once inside the terminal: **The person at the computer took control.**
12. **Handing back.** Wait about 30 seconds, take control again, and press **Hand back**. It says nobody
    has control, and the computer's size comes back.
13. **Away and back.** Take control, then send the app to the background. At the computer, the terminal is
    back to its own size at once, and typing there works. Come back: it is watching, with **Take control**
    offered and nothing typed. Lock the phone and unlock it the same way.
14. **A drop.** Take control of the shell, turn on airplane mode and press **Send** with something in the
    field. Within about 30 seconds it says the connection dropped, and that the text may not have reached
    the computer. Turn airplane mode off. **Watching** comes back, nothing was typed at the computer, and
    control is not taken. **Put it back** returns the text.
15. **Sideways.** Take control and rotate the phone, with and without the keyboard up. The screen redraws
    to fit each, and with too little room for 5 rows it stays as it was.
16. **A program's end.** With control of the shell, run `exit`. The phone says **The program ended**, and
    the field goes.

## 15 · What the seventh slice adds

Built on 2026-11-03, on Linux. Its smoke test, below, passed on the owner's iPhone against staging that day.
Android stays unbuilt. It adds no native module, so `just mobile-start` serves it to the app already installed. The server and
the host did not change, so staging needs no deploy.

- **Telling apart what a refusal meant** (`Accounts.explain`, §5). A channel closed with 4003 or 4010, or a
  ticket refused with `forbidden` or `not_found`, used to read as the computer no longer letting the
  phone in. Now the server is asked first, by reading the list of computers:
  - A computer the list no longer has, but the account does, **took the phone back**. One the account no
    longer has **is no longer on this account**. Each says so in those words, on the screen that was
    looking and on the account, and what was kept of it goes. The phone stays signed in, and every other
    computer's channel stays as it was.
  - **The phone refused** means signed out, as anywhere else: its credential, its profile and what was kept
    go, and Connect says so.
  - **Anything else**, a server out of reach or one that still lists the computer, deletes nothing. The
    computer reads **Cannot be reached**, the terminal or what was last seen stays, and the channel is tried
    again, waiting 2 seconds and doubling up to 30.
- **Why not `explainRevoked`:** the client's function answers the same three questions, and the pairing tool
  still uses it. But it takes refused access for being signed out, and the app refreshes first, since a
  phone that slept can hand out access that has lapsed. And it does not tell the two kinds of parting apart.
- **Told once, and only when it is news** (`Snapshot.lost`). The list says which computers it dropped from what
  was kept, so a computer that left while the app was closed is told on opening too. It is in memory, per
  profile, until **OK**. A computer unpaired here tells nobody anything: the relay's close can come before
  the request's answer, so unpairing marks it before asking.
- **Nothing is left on screen** (`OfProfile`, `src/ui/live.tsx`). A computer, workspace, memory or terminal screen
  belongs to the profile that opened it. Once that profile is gone or not the one in use, or its address
  answers as another server, the screen shows nothing and goes to the account, which says what happened. It
  is per screen, not a guard in the layout, because a pairing link can switch servers from the top of a stack
  with another server's terminal under it: going back to that terminal shows nothing of it.
- **Removing a server** (`src/app/servers.tsx`): **Remove** on any profile, in use or not, asks first, then
  signs out at that server and deletes the credential and what was kept. One not opened since the app started
  is opened for this and no more, with its address asked who it is first. With the server out of reach, or
  another server at its address, nothing is sent and nothing goes unless the person chooses **Remove from this
  phone**, as with signing out. Removing the one in use leaves no profile in use until one is picked.
- **A credential that comes back too late** is no longer kept: a refresh answered after its profile was deleted
  would have left a credential in the store that nothing could ever delete.
- **Tests: 287 in the app.** The logic, 180 under `bun test`, 19 of them new, against the real server, relay and host:
  a grant taken back with someone looking and with nobody looking, unpairing here, the phone revoked, the server
  out of reach, lapsed access, a computer taken off the account, and removing a server in each state it can be in.
  The screens, 107 under Jest, 17 of them new. Breaking any of 29 lines on purpose failed a test.
- **What has not been tried on a phone:** Android at all. The **Cannot be reached** state: it needs the relay's
  close to arrive while the server does not answer, which a phone cannot be made to do on purpose, so tests against
  the real server cover it, and step 7 covers the part a phone can show. Two different servers: the smoke test's
  second profile is the same server at another address, and tests cover two real ones. Two computers are tried on
  the screens against a fake and in the smoke test: the stand-in has one host.

**Its smoke test, on the iPhone against staging.** The plan's second computer was the Mac, and its second server
a local one. Neither is needed to show this. The pairing tool is a real second computer through the relay, with
no Rust on the Mac. A phone cannot reach a local server (§7), but staging also answers at its Fly address, under
the same id, so a second profile needs no setup. It is the same account, so the devices list is the same on both.
What it shows is that computers and what was kept are per profile; tests against two real servers cover
the rest. A sign-in does come back at that address, so step 9 needs no Simulator.

1. On this computer, from the worktree, with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote on against `https://remote-staging.yardsort.sh`, with a shell open in a workspace.
2. On the Mac, from your clone, on this branch: `just mobile-start`, and open Yardsort Dev on the phone. If it is
   not paired with this computer, pair it.
3. **A second computer.** On this computer, in another terminal, from the worktree:
   `just server-pair host https://remote-staging.yardsort.sh --control --serve --revoke-after 1800`. Open the address
   it prints and sign in. It prints a code. On the phone, **Pair a computer**, type the code. **Pairing tool host**
   joins the list, and both computers read **Online**. It stays for 30 minutes, then takes the phone back and
   removes itself.
4. **One computer takes the phone back.** On the phone, open this computer's shell, so it is watching. At the
   computer, Settings → Remote → **Revoke** beside the phone. Within a second or so the phone says **This computer no
   longer lets this phone in.**, and the terminal is gone. **Back to the account**: a card says the same by name, the
   computer is off the list, and **Pairing tool host** still reads **Online**. Open its terminal, take control and
   send `hi`: `got: hi` comes back. **OK** clears the card. Pair with this computer again.
5. **While the app was closed.** Force-quit the app. Revoke the phone at the computer again. Open the app: the same
   card is on the account. **OK**, and pair again.
6. **Unpairing here.** On the phone, open this computer, **Unpair** it and confirm. It leaves the list, and no card says anything.
   Pair again.
7. **Out of reach deletes nothing.** Open this computer on the phone, so its workspaces show. Turn on airplane mode: within
   about 30 seconds it reads **Reconnecting**, and the workspaces stay, marked **As of**. Revoke the phone at the computer. Nothing changes
   on the phone. Turn airplane mode off: within about 30 seconds it says **This computer no longer lets this phone
   in.**, and the workspaces are gone. Pair again.
8. **A computer gone.** Open this computer's shell on the phone. At the computer, Settings → Remote → **Turn off**. Within
   about 15 seconds the phone says **This computer is no longer on this account.**, not that it took the phone back.
   **Back to the account**: a card says the same. Turn Remote on again at the computer, and pair again.
9. **A second server.** Account → **Servers on this phone** → **Add a server**: `https://yardsort-remote-staging.fly.dev`.
   Look it up and sign in. Its account screen says no computer lets this phone in yet, with none of the computers from
   before. **Servers on this phone** lists both, with their addresses. Press the first: its computers are back, with no
   sign-in. Press the second again: none.
10. **Removing a server.** With the second in use, **Remove** the first and press **Cancel**: nothing changes. Then
    **Remove** the second and confirm. It leaves the list, and none is in use: **Choose one to use.** Press the first:
    its computers are there. Its devices list shows the second's device as signed out.
11. **The phone revoked while watching.** Open this computer's shell on the phone. On this computer, from the worktree:
    `just client-pair devices https://remote-staging.yardsort.sh`, sign in, and note the id beside the phone. Then
    `just client-pair devices https://remote-staging.yardsort.sh --revoke ID`, and sign in again. The phone leaves the
    terminal for Connect, saying **This phone was signed out of Yardsort staging. Nothing from it is kept here.**, not
    that a computer took it back. **Servers on this phone** is gone if it was the last. Force-quit and open: Connect, with
    nothing shown.

## 16 · Android in an emulator

Run on 2026-11-03, on Linux, after the seven slices: §7 asks for Android to be built and tried in an emulator as the
work goes, and until now this machine had no Android SDK. **It shows that the app builds and runs on Android, not that
it works on a phone,** and it is not the device run. The emulator was Android 16 (API 36) with Google APIs, a Pixel 7's
screen, x86_64, WebView and Chrome 133, with no window, driven through `adb`. The server, relay, host and sign-in
were the stand-in (`crates/server/examples/stand-in.rs`), reached through `adb reverse`: no real account, no real agent.

**It found three things no test had, each fixed here and with a test that fails without the fix:**

- **Opening any terminal crashed the app.** `dataDetectorTypes="none"` is a word, which the WebView's iOS side turns into
  the list its native prop is, and its Android side passes on as it is. React Native's native code then aborts the
  process. It is `["none"]` now.
- **A dark phone had a white header**, with the status bar's white icons lost against it. Android's header is drawn
  from the navigation theme, and nothing set one. The root layout now gives it the app's own colors. On iOS 26 the glass
  bar and the title stay the system's, since nothing sets a header background or tint. What changes there is that the
  bar is told a dark phone is dark, and the back button takes the dark theme's blue, so **iOS's dark mode wants a look**
  on the next run there.
- **A sign-in's return reached the router, code and all.** On Android it comes to the app like any link, as well as to
  the browser session waiting for it. A sign-in that went on to the account was left looking fine. Backing out of the
  browser just as it came back left **Unmatched Route** on the screen, with the code in it, and the phone not signed
  in. It now stops in `+native-intent`, as a pairing link's secret does, and a session still waiting hears it through its
  own listener. Six sign-ins in a row afterwards came back to the account. A code is useless without the verifier the session holds, but it had no
  business in a route.

**What worked, seen on the emulator:**

| Area       | What                                                                                                                                                                           |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Signing in | A Custom Tab through to `yardsort://signed-in`, after Chrome's first-run screen. Signed in again after a crash and a force-stop. Signing out, and Connect after it             |
| Pairing    | Typing the code, a link from outside the app (asked about first), and the camera permission denied, asked again and allowed                                                    |
| Watching   | The WebView's page, messages both ways, Unicode, CJK and emoji, sideways scrolling, a link tapped and asked about, no keyboard from tapping the terminal                       |
| Control    | Refused while the computer types, taken, typed, handed back, let go on leaving and in the background. Type, Esc, ↑ and Enter reached the program as the right bytes            |
| Keyboard   | The composer stays above it with the screen fitted, and Back closes it before leaving the screen. The status-bar and keyboard measures §14 left untested work on this emulator |
| Lifecycle  | Back from the background with what was missed, a connection dropped and regained (15 seconds after it returned), and the WebView's process killed                              |
| Losing it  | Taken back while watching: says so there and on the account, with **OK**                                                                                                       |
| Looks      | Dark and light, followed while running, landscape, and the account's screen with text at 1.6 times its size                                                                    |
| Kept       | SecureStore's backup rules are in the manifest, leaving credentials out of cloud backup and device transfer                                                                    |

**Left as they are, for the device run or later:**

- **Landscape with the keyboard up** gets Android's full-screen text entry, which hides the terminal. Its Send works.
  There is too little room beside a keyboard to show both, so this is left alone until it has been tried on a phone.
- **Changing the system's text size** recreates the Android activity, which goes back to the first screen and lets go
  of control. `fontScale` is not among the config changes the generated project handles. Nothing is lost, and it is rare.
- **Backing out of the browser just as a sign-in returns** can still read as not signed in, since Expo races the app
  coming back against the address arriving. The person presses **Sign in** again.
- **Permissions.** The generated project also asks for external storage, vibration and drawing over other apps, which
  the app does not use. They want blocking before any store build (R7). `RECORD_AUDIO` is not asked for.
- **SecureStore's `WHEN_UNLOCKED_THIS_DEVICE_ONLY` is iOS's.** On Android the values are encrypted with a key in the
  Keystore, and the backup rules above do the rest.

**Not tried at all:** a real phone and its keyboards, a real scan (the camera's view opened, black without a window, and
nothing was scanned), the other keys on the row, an external keyboard, the phone locked and woken, Wi-Fi to cellular, or a release build. Real agents and a real sign-in are
for the smoke test below.

**Setting it up.** `apps/mobile/README.md` has the packages. This machine's Java is a runtime with no compiler, which
stops the build, so JDK 17 sits beside the SDK in `~/Android/jdk17`. `just mobile-android` then built, installed and opened
the app from nothing.

**Its smoke test, on this computer, against staging.** A person signing in for real, with real agents, which the run
above had neither of. It needs no deploy.

1. In one terminal, start the emulator: `ANDROID_AVD_HOME=~/.config/.android/avd ~/Android/Sdk/emulator/emulator -avd yardsort`.
   A window with a phone in it should open: the run above had none, so that part is untried here. If it does not, put
   `QT_QPA_PLATFORM=xcb` in front.
2. In another, from the worktree: `~/Android/Sdk/platform-tools/adb reverse tcp:8081 tcp:8081`, then `just mobile-start`. This
   serves the app's code to the emulator, as the Mac does for the iPhone.
3. In a third, from the worktree, the desktop app with a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev`.
   Settings → Remote on against `https://remote-staging.yardsort.sh`, with Claude running in a workspace.
4. **Signing in.** Open **Yardsort Dev** in the emulator. Connect to `https://remote-staging.yardsort.sh`, and **Sign in**. Chrome
   opens: sign in there. It comes back to the account, with no page saying **Unmatched Route** on the way.
5. **Pairing.** At the computer, Settings → Remote → **Pair a device**. Type the code in the emulator, and say yes at the computer. The
   computer reads **Online**.
6. **A terminal.** Open it, and open Claude's conversation. The terminal draws, where it used to crash the app.
7. **Control.** **Take control**, tap the field, and send `hi`. Claude answers, with the keyboard below the field and
   nothing hidden. Press the emulator's Back: the keyboard goes. Press it again: the screen goes, and the phone lets go of control.
8. **Dark.** `~/Android/Sdk/platform-tools/adb shell cmd uimode night yes`: the header goes dark, and the clock and icons
   above it can be read. Then the same with `no`.
9. **Background.** Open the terminal again, press the emulator's Home, type something at the computer, and open the app: what
   was typed is there.
10. **Taken back.** At the computer, **Revoke** beside the phone: **This computer no longer lets this phone in.**

## 17 · The device run, on the iPhone

Cut on 2026-11-04, by the owner's decision, to what the smoke tests never ran on a phone, and **not yet run**.
It takes about 20 minutes, with the phone also left locked for 20 minutes at some point. Android's half stays
open, so this does not close §7's gate.

**How to read it.** **Expect** is what the app was built to do, so anything else is a failure. **Find out** is
something nobody has decided or seen, so there is nothing to fail: write down what happened. Report a step by its
name, such as `C2`.

**Already passed on this iPhone, against staging, and not repeated here:**

| Passed     | Smoke test                                                                                                                                         |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-11-02 | §12: workspaces and activity. §13: watching a terminal, airplane mode and the background                                                           |
| 2026-11-02 | §14: Claude, Codex and a shell under control, the keys, pasting, Unicode, dictation, vim, handing over, locking, rotating, a drop, a program's end |
| 2026-11-03 | §15: signed out, revoked and switching                                                                                                             |

**Moved to R4 by the same decision:** the proposal's two time budgets, input to echo and back from the foreground,
and an external keyboard. Neither has been tried. Nothing in the app reads a physical key, so most of them may do nothing
there.

**Left for the Android phone:** direct typing into the terminal, which §5 said this run would try. The page has it turned off
(`disableStdin`), so it needs a build of its own, and it could only be offered once it had worked on both phones.

### Setting up

The phone needs data of its own, with Settings → Apps → Yardsort Dev → Cellular Data on and any VPN off. Nothing native has
changed since the last iPhone run, so the app already installed will do.

1. **The host.** On this computer, stop any `just dev` for this profile. Then, from
   `/home/josh/yardsort/yardsort/implement-remote-workspace`, end its old daemon and agents, since a daemon keeps running through a rebuild, and start again:

   ```sh
   pkill -f 'yardsort-daemon .*/tmp/ys31/'
   YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys31 YARDSORT_WORKTREE_ROOT=/tmp/ys31-wt just dev
   ```

   Settings → Remote: server `https://remote-staging.yardsort.sh`, **Turn on and sign in**.

2. **The terminals.** In a workspace on a throwaway project, start a Claude agent and open two shells. In the first, run
   `bash --norc`: this is **the shell**. In the second, run `while :; do date +%T; sleep 1; done`: this is **the clock**, which
   shows at a glance whether a screen is live and current. Make the window wide.

3. **The phone.** On the Mac, from your clone: `git fetch origin && git checkout --detach origin/main`, then `just mobile-start`, and leave it
   running. Open Yardsort Dev, and sign in and pair if it asks, allowing it to control terminals. If iOS ends the app while it is away, it
   needs the Mac to start again. On cellular, a notice about Metro is the development build's, not a failure.

### N · Wi-Fi to cellular and back

Airplane mode is a clean drop, and this can leave a connection half dead. Toggle Wi-Fi in Settings, since Control Center's button only
disconnects it for a while. Write down how long each return takes, by the clock.

- `N1` Watch the clock on Wi-Fi, and turn Wi-Fi off. **Expect:** **Reconnecting. What is on the computer keeps running.**, then the clock live over
  cellular, with an unbroken screen.
- `N2` Turn Wi-Fi back on, and keep watching. **Find out** whether it moves over by itself, how long that takes, and whether the screen notices.
- `N3` Open the shell, **Take control**, and send `cat`. Send `1`, turn Wi-Fi off, come straight back, send `2`, then `3` as soon as you can. **Expect:** at
  the computer each line at most once and in order. A line that may not have arrived is shown as such with **Put it back** and **Dismiss**, nothing is
  sent by itself, and control is not taken again by itself.
- `N4` Turn Wi-Fi back on, and do `N3` again. Then press **Ctrl** and `d` to end `cat`.

### W · Away past the credential's 15 minutes

- `W1` Ask Claude for a task that takes a while, open its conversation on the phone, and lock the phone for over 20 minutes. Unlock. **Expect:** no
  sign-in asked for, **Watching**, and the agent's screen current. Then open the clock. **Expect:** it is current and ticking, and at the computer it has not missed a beat.

### C · Composing text

Add a keyboard: Settings → General → Keyboard → Keyboards → Add New Keyboard → Japanese → Romaji. Take control of the shell, and send `cat`.

- `C1` Type `nihongo`, choose `日本語` from the suggestions, and press **Send**. **Expect:** `日本語`, once, with no `nihongo`.
- `C2` Type `nihongo`, and press return **while it is still underlined**. **Find out** whether return only accepts the word or sends it too, and whether a
  half-made word ever reaches the computer. Then do the same with **Send**.
- `C3` Type `nihongo`, and while it is underlined press **Ctrl**, then type `c`. **Find out** what happens. Ctrl works out what was added to the field, and
  composing changes what is there. Then press **Ctrl** and `d` to end `cat`.

### Q · Programs that ask the terminal things

While a phone has control, the host answers only where the cursor is, and nothing else is answered.

- `Q1` With control of the shell, send `tmux`. Press **Ctrl** then type `b`, then type `%` and press **Type**. **Expect:** a split, with no stray
  characters. **Find out** whether it starts slower than at the computer. Send `exit` in each pane.
- `Q2` Paste each of these into the field, copied on the Mac or from Notes, and press **Send**:

  ```sh
  printf '\e[6n'; IFS= read -rsd R -t 2 a; printf '%q\n' "$a"
  printf '\e[c'; IFS= read -rsd c -t 2 a; printf '%q\n' "$a"
  ```

  **Expect:** the first prints a position, such as `$'\E[12;1'`, once. The second prints `''` after two seconds. Neither leaves stray characters at the next
  prompt. Run in tmux and with nothing answering on 2026-11-04, these print those shapes. Through a phone's lease, they have not been tried.

- `Q3` With control of Claude's conversation, type `/` in the field and press **Type**, since Enter would choose from the menu. Use its arrows, leave it with
  **Esc**, then send it two requests in a row. **Expect:** nothing sits stuck. If anything hangs until a key is pressed, that is §5's slice, so write
  down what it was doing.

### L · What changed after the last run

- `L1` **The sizing fixes** (§14). With control of the shell, quickly raise and dismiss the keyboard five times, rotate twice quickly, and leave it. Send
  `stty size`. **Expect:** it settles with no half row and nothing to scroll, its top line clear of the header and its bottom line above the field, and the
  rows and columns are what fits. Do the same in Claude's conversation.
- `L2` **Dark mode** (Settings → Display & Brightness, with the app's own choice in its Settings on Automatic, [33](33-phone-design.md)). Go through the account, a computer, a workspace and a terminal, in dark and light. **Find out** how the header's glass, its title and the back button look. They changed on 2026-11-03 (§16), and nobody has looked at them on iOS.

### The record

To be filled in from the run, with the iPhone's model and iOS version, `claude --version` and the commit:

| Part                     | Result  | Notes |
| ------------------------ | ------- | ----- |
| N · Wi-Fi and cellular   | Not run |       |
| W · Away past 15 minutes | Not run |       |
| C · Composing text       | Not run |       |
| Q · Programs that ask    | Not run |       |
| L · After the last run   | Not run |       |

## 18 · Open after this plan

- **For R4, moved from the device run on 2026-11-04 (§7):** input to echo at p95 under 300 ms at 100 ms of round trip, a usable screen within 5 seconds
  of coming back to the foreground, and an external keyboard. The first two have their tools since
  R4's slice 8 ([35 §17](35-remote-r4-r5.md#17--what-the-eighth-slice-adds)): the echo measured
  locally through 100 ms at p95 105 ms. The keyboard came with slice 9
  ([35 §18](35-remote-r4-r5.md#18--what-the-ninth-slice-adds)), tried on the Android emulator. On
  the iPhone, none of the three has been tried.
- **A physical Android phone** for the smoke tests and the device run (§7). The owner's to
  provide; R3 cannot close without one.

## 19 · After first real use: scrolling, starting work, pictures

The owner's first days of real use, on 2026-11-06, found one bug and asked for two things.

**A terminal's history could not be reached.** Watching a Claude Code session, the phone went up as
far as the top of the computer's screen and no further, though the computer kept the lines above it
(`ys logs` printed them). The page moved by itself while it could, and handed the rest of a drag to
the terminal's history; but a terminal taller than the phone is a page that moves, and once the
WebView's own scroll has a drag, the page hears no more of it. The page's test had only a terminal
shorter than the phone. Now the page moves everything upright itself, the page and then the
history, carrying on after the finger lifts, and sideways is left to the WebView; a test drags a
terminal taller than the phone from its bottom into its history and back. The page also keeps
10,000 lines, as the computer does, where it kept 5,000.

**Starting work, and pictures (protocol minor 5).** `harness.list`, `workspace.start`,
`session.start`, `upload.begin` and `upload.chunk` (`crates/remote-proto/src/start.rs`). Resuming, and
the desktop starting work, followed in minor 6 ([35 §16](35-remote-r4-r5.md#16--what-the-seventh-slice-adds)).
Decisions:

- **Under `terminal:control`.** Starting one of the computer's own agents is no more than a device
  that may type could do in a shell, so it needs no new scope, and no phone paired before has to
  pair again. A device names a harness, a project or workspace, and a message; never a program, an
  argument or a folder. The computer offers only harnesses turned on and found, and starts them as
  its settings say, through the same `Workspaces::create` and `Launcher::in_workspace` as its window.
- **A start answers from a thread of its own.** A project's setup can take minutes, and a channel
  that stops answering for 30 seconds is taken for dead by the phone. A start whose answer is lost
  is not tried again: the phone says it may have started, and to look before starting again.
- **A picture goes outside every worktree**, in the profile's `uploads/<workspace>/`, under a name
  made of the time and the letters and digits of its own, never replacing a file. Pieces of at most
  32 KB go in order, a few ahead of their answers; a piece out of order, or more than was said,
  ends the upload and deletes it, as a channel closing does. Files older than seven days go when
  another comes. The phone sends a JPEG no larger than 2048 pixels, since a photo off the camera is
  several megabytes of HEIC.
- **The last piece's answer carries the file's path**, the only answer that carries one: the phone
  pastes it into the terminal, quoted for the computer's shell, as a file dropped on the desktop's
  terminal is. Only a device that could type `pwd` is given it.
- **The window shows what a phone started at once**, through the `SessionStarted` event a workflow's
  start already sent: its tab, and the sidebar reads the workspaces again.

The picture needs two native modules, `expo-image-picker` and `expo-image-manipulator`, so it reaches
the iPhone with a new TestFlight build. Not yet seen on a real phone.
