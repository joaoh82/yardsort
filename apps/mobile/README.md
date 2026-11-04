# Yardsort for phones

The phone app: reach the agents on your computers from an iPhone or an Android phone, through a
Yardsort server you sign in to. It is an [Expo](https://expo.dev) app, SDK 58 (React Native
0.88, React 19.3), and it says everything it says to a server through
[`packages/remote-client`](../../packages/remote-client). Its plan, and why each decision went
the way it did, is [docs/design/31](../../docs/design/31-remote-r3.md), and how it looks is
[docs/design/33](../../docs/design/33-phone-design.md).

**What it does so far (R3 slices 2 to 7):** connect to a server by its address and see what that
server is and can see, sign in through the system browser, list the account's devices with this
phone among them, sign out, and keep a profile per server with one in use. Pair with a computer by
scanning its code, typing it or following a link, see each computer online, offline or
reconnecting, and unpair. See each computer's workspaces, a workspace's conversations and other
terminals, its activity as it is recorded, and a project's approved memory, with what was last seen
shown as such when a computer cannot be reached. Watch a terminal at the computer's size, resuming
after a drop or the background. Take control of it, type, paste and use the keys a phone lacks, drawn
to fit the phone, and hand it back, or have the person at the computer take it at any time. Tell a
computer that took this phone back or left the account from this phone being signed out, and remove
a server from the phone.

## What is in it

| Path                  | What                                                                                                                                                                                                                                                                                                                                                        |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/app/`            | The screens, as Expo Router routes: `index` (where to start), `connect`, `account` (computers, devices, signing out), `servers` (one profile in use), `pair`, `computer` (a computer's workspaces), `workspace` (conversations, terminals and activity), `memory`, `terminal`, and `+native-intent`, which takes a pairing link before the router reads it. |
| `src/lib/accounts.ts` | Signing in, opening a profile again, signing out. Screens read its snapshot and call it; none builds a request or holds a credential.                                                                                                                                                                                                                       |
| `src/lib/profiles.ts` | The profiles, kept in the phone's credential store: each server's address, its id, this phone's device there and its refresh credential.                                                                                                                                                                                                                    |
| `src/lib/pairing.ts`  | A pairing link held in memory until used, where it belongs among the profiles, and waiting for the person at the computer.                                                                                                                                                                                                                                  |
| `src/lib/presence.ts` | Whether each computer can be reached, from a channel held to it while someone is looking.                                                                                                                                                                                                                                                                   |
| `src/lib/activity.ts` | A workspace's activity followed through a channel, and what each event says. In memory only.                                                                                                                                                                                                                                                                |
| `src/lib/control.ts`  | Having control of a terminal: asking for it, what is typed going one at a time, what may not have arrived, and letting go.                                                                                                                                                                                                                                  |
| `src/lib/keys.ts`     | The keys a phone lacks, as the bytes a terminal would send for them.                                                                                                                                                                                                                                                                                        |
| `src/lib/watch.ts`    | Watching a terminal through whichever channel is open, and holding output back from a page that has not caught up.                                                                                                                                                                                                                                          |
| `src/lib/bridge.ts`   | The only thing the app and the terminal's page share: typed, bounded messages, each checked by the side reading it.                                                                                                                                                                                                                                         |
| `terminal/`           | The page a terminal is drawn in. `scripts/terminal-page.ts` builds it into `src/ui/terminal-page.generated.ts`: `just mobile-terminal-page` after changing it. A test says when it is out of date.                                                                                                                                                          |
| `src/lib/observed.ts` | What was last seen of a profile's computers and workspaces, kept in the cache folder.                                                                                                                                                                                                                                                                       |
| `src/lib/native.ts`   | What only the phone has: SecureStore, the cache folder, the system browser's sign-in session, randomness and SHA-256, the phone's name.                                                                                                                                                                                                                     |
| `src/ui/parts.tsx`    | The few native components the screens are built from, with the colours and fonts of [the design](../../docs/design/33-phone-design.md).                                                                                                                                                                                                                     |
| `src/ui/composer.tsx` | The field and the row of keys for typing into a terminal.                                                                                                                                                                                                                                                                                                   |
| `src/ui/live.tsx`     | What screens share about being live: a channel held only while on screen, how far a computer can be reached, what was last seen meanwhile, and showing a profile's things only while it is in use.                                                                                                                                                          |
| `src/ui/leaving.ts`   | What is asked when signing out cannot reach the server.                                                                                                                                                                                                                                                                                                     |
| `assets/`             | The icon and splash screen, as SVG and the PNGs built from them, and the embedded fonts with their licenses.                                                                                                                                                                                                                                                |
| `test/lib/`           | The logic, under `bun test`, against the real server and its sign-in (`crates/server/examples/stand-in.rs`, as the protocol client's tests use), and the terminal's page in real headless Chromium, skipped where no browser is found.                                                                                                                      |
| `test/screens/`       | The screens, under Jest with `jest-expo`, through Expo Router's in-memory router and Testing Library, against a small fake server.                                                                                                                                                                                                                          |

Things it holds to:

- **Credentials** live only in SecureStore: on iOS `WHEN_UNLOCKED_THIS_DEVICE_ONLY`, and on Android
  under a Keystore key with its backup rules leaving them out. Never in a backup, never on a new
  phone, never in a log. A new refresh credential is kept before the access that
  came with it is used.
- **A profile is a server's address and the id it gave.** Before every refresh credential goes to
  an address, on opening or in a session already open, the app asks the address who it is. If it
  now answers as another server, nothing is sent, and the app says so.
- **Signed out at the server** (the refresh credential refused): the profile is deleted and the
  phone says so. A refused access credential is refreshed and tried once more first, since the
  server says the same for access that has lapsed. **A server out of reach** deletes nothing.
- **A pairing link's secret** stays out of the router and off the disk, and a link is never used
  on its own say: one from outside the app, or naming another server, is asked about first. **A
  sign-in's code** stays out of the router too, though Android hands it to the app as a link.
- **Online is what the socket says.** Channels are held only while the computers are on screen and
  the app is in the foreground. Offline never means a computer's agents stopped.
- **A computer refusing this phone** does not say why, so the server is asked before anything goes.
  Taken back or off the account: that computer's kept entries go, and every other computer goes on
  working. The phone's own credential refused: signed out, as below. Not known: nothing goes, it
  reads as unreachable, and it is tried again.
- **What a profile reaches stays on screen only while that profile is in use.**
- **What is kept of a computer** is names and branches, each marked with when it was seen: never
  a session title, activity, memory or terminal output. It goes with a computer that took this phone
  back and with the profile. Taking control of a terminal is the only thing here that changes anything
  on the computer.
- **A terminal is drawn, and nothing more.** Its page connects to nothing, goes nowhere, and keeps
  nothing, and no terminal output is written anywhere. A link is `https`, tapped, and asked about
  first. Typing has a native field of its own.
- **Control is only taken by a person pressing something**, and never taken back by itself. Nothing is
  sent twice: what may not have arrived is shown as such, and what was waiting when control was lost
  is dropped. Reconnecting never types. Leaving the screen or the app lets go.
- **Signing out or removing a server** revokes this phone at that server, which takes back every
  computer's grant, then deletes the profile. With the server out of reach, or another server at its
  address, it sends nothing and asks before deleting anything.

## Running it

`ios/` and `android/` are generated by `expo prebuild` when a build needs them, and never
committed. The app is its own Bun project (31 §3), and takes the protocol client as a local
file dependency: after changing `packages/remote-client`, run `bun install` here again.

### On an iPhone, from a Mac

You need macOS with Xcode, signed in to your Apple account (Xcode → Settings → Accounts), and the
iPhone plugged in, unlocked and trusting the Mac. On the iPhone, turn on Developer Mode (Settings
→ Privacy & Security → Developer Mode) when iOS asks. Besides Xcode, the Mac needs Bun, Node.js
(Expo's command line runs on it) and CocoaPods; with Homebrew,
`brew install oven-sh/bun/bun node cocoapods watchman`. Rust is not needed for the app.

A fresh Mac needs a little more before it can build onto a phone:

- **Xcode's extra components.** Open Xcode once, or run `sudo xcodebuild -runFirstLaunch` in a
  terminal. Without them `expo run:ios` stops with "xcrun is not configured correctly".
- **A signing certificate.** Signing in to Xcode does not create one. In Xcode → Settings →
  Accounts, select the team, then Manage Certificates → + → Apple Development. Without one,
  `expo run:ios` stops with "No code signing certificates are available to use".
- **Apple's G3 intermediate certificate.** Expo says the same thing when the certificate is there
  but macOS cannot trust it: `security find-identity -v -p codesigning` reports 0 valid
  identities. Import the intermediate from the command line (opening the file in Keychain Access
  can fail with error -25294):

  ```sh
  curl -fsSLO https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer
  security import AppleWWDRCAG3.cer -k ~/Library/Keychains/login.keychain-db
  ```

- **Access to the certificate's key.** During the first build, macOS asks whether `codesign` may
  use the key: enter the login password and choose Always Allow, or it asks again for every
  framework. If that dialog stops responding, quit it with `killall -9 SecurityAgent`, stop the
  build, and grant the access once from a terminal:

  ```sh
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s ~/Library/Keychains/login.keychain-db
  ```

```sh
just mobile-ios      # or, here: bun install && bun run ios
```

That builds the development app (`Yardsort Dev`, `dev.yardsort.mobile.development`), installs it
on the phone, and serves its code from the Mac. The first time, it asks which phone and which
signing team: choose the paid team, so the build does not lapse after seven days. The phone and
the Mac must be on the same network; let the app find devices on the local network when iOS
asks. Afterwards, `just mobile-start` serves the code to the app already installed. A new native
module or a change to `app.config.ts`, as slice 3's camera was, needs `just mobile-ios` again: it
runs `expo prebuild` first, because `expo run:ios` alone builds an `ios/` that already exists
without applying the config to it, and the app then lacks what the config adds. Prebuild clears
the signing team, so with more than one signing identity on the Mac it asks for the team again.

The phone cannot reach a development server on the Mac's loopback, so on a phone use staging,
`https://remote-staging.yardsort.sh` (31 §7). The iOS Simulator shares the Mac's loopback, so it
can use `just server-up` and `http://127.0.0.1:8787`.

### On Android

`just mobile-android` (here: `bun run android`) builds with the Android SDK, without EAS, onto an
emulator or a phone over USB. So far it has run on Linux, onto an emulator: **no physical Android phone
has run this app** (31 §16). It needs:

- **JDK 17**, which is what it has been built with. `JAVA_HOME` says which one to use. A Java with no
  compiler stops the build with `does not provide the required capabilities: [JAVA_COMPILER]`.
- **The Android SDK**, with `ANDROID_HOME` pointing at it, `sdkmanager` from Google's command-line
  tools, and these packages: `platform-tools`, `platforms/android-37.0`, `build-tools/37.0.0`,
  `ndk/27.1.12297006` and `cmake/3.22.1`. An emulator also takes `emulator` and
  `system-images/android-36/google_apis/x86_64`, which has Chrome for signing in.

```sh
export JAVA_HOME=~/Android/jdk17 ANDROID_HOME=~/Android/Sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$PATH"
just mobile-android
```

Afterwards, `adb reverse tcp:8081 tcp:8081` and `just mobile-start` serve the code to the app already
installed. A phone or emulator reaches a local server through `adb reverse tcp:8787 tcp:8787` and
`adb reverse tcp:8080 tcp:8080` (31 §7), or uses staging, which an emulator reaches directly.

## Checks

```sh
just test-mobile     # or, here: bun run test
```

`just check` runs them, and the app's typecheck (`bun run typecheck`) and lint (`bun run lint`,
Expo's rules) too. CI runs them on Linux, macOS and Windows. None of that runs the app on a
phone: what was tried on which phone is recorded in [31](../../docs/design/31-remote-r3.md).

## License

[MPL-2.0](LICENSE), as the protocol client is and unlike the rest of the repository: see
[docs/design/31 §6](../../docs/design/31-remote-r3.md#6--the-license-43).
