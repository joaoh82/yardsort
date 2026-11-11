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
to fit the phone, and hand it back, or have the person at the computer take it at any time. Give the
agent a picture. Start a new workspace with an agent, or another agent in a workspace
([31 §19](../../docs/design/31-remote-r3.md#19--after-first-real-use-scrolling-starting-work-pictures)). Tell a
computer that took this phone back or left the account from this phone being signed out, and remove
a server from the phone. Choose light, dark or automatic in Settings.

## What is in it

| Path                     | What                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/app/`               | The screens, as Expo Router routes: `index` (where to start), `connect`, `account` (computers, devices, signing out), `settings` (light, dark or automatic), `servers` (one profile in use), `pair`, `computer` (a computer's workspaces), `workspace` (conversations, terminals and activity), `memory`, `terminal`, `start` (a new workspace, or another agent in one), and `+native-intent`, which takes a pairing link before the router reads it. |
| `src/lib/accounts.ts`    | Signing in, opening a profile again, signing out. Screens read its snapshot and call it; none builds a request or holds a credential.                                                                                                                                                                                                                                                                                                                  |
| `src/lib/profiles.ts`    | The profiles, kept in the phone's credential store: each server's address, its id, this phone's device there and its refresh credential.                                                                                                                                                                                                                                                                                                               |
| `src/lib/pairing.ts`     | A pairing link held in memory until used, where it belongs among the profiles, and waiting for the person at the computer.                                                                                                                                                                                                                                                                                                                             |
| `src/lib/presence.ts`    | Whether each computer can be reached, from a channel held to it while someone is looking.                                                                                                                                                                                                                                                                                                                                                              |
| `src/lib/activity.ts`    | A workspace's activity followed through a channel, and what each event says. In memory only.                                                                                                                                                                                                                                                                                                                                                           |
| `src/lib/control.ts`     | Having control of a terminal: asking for it, what is typed going one at a time, what may not have arrived, and letting go.                                                                                                                                                                                                                                                                                                                             |
| `src/lib/appearance.ts`  | Light, dark or as the phone is: the choice, keeping it, and telling the system.                                                                                                                                                                                                                                                                                                                                                                        |
| `src/lib/attach.ts`      | A picture for an agent: what was chosen, and its path on the computer as a terminal pastes it.                                                                                                                                                                                                                                                                                                                                                         |
| `src/lib/keys.ts`        | The keys a phone lacks, as the bytes a terminal would send for them.                                                                                                                                                                                                                                                                                                                                                                                   |
| `src/lib/hardware.ts`    | A keyboard paired with the phone: which keys are taken from the field, and what each sends.                                                                                                                                                                                                                                                                                                                                                            |
| `modules/hardware-keys/` | The native view around the composer that hears them: key commands on iOS, the window's key events on Android.                                                                                                                                                                                                                                                                                                                                          |
| `src/lib/watch.ts`       | Watching a terminal through whichever channel is open, and holding output back from a page that has not caught up.                                                                                                                                                                                                                                                                                                                                     |
| `src/lib/bridge.ts`      | The only thing the app and the terminal's page share: typed, bounded messages, each checked by the side reading it.                                                                                                                                                                                                                                                                                                                                    |
| `terminal/`              | The page a terminal is drawn in. `scripts/terminal-page.ts` builds it into `src/ui/terminal-page.generated.ts`: `just mobile-terminal-page` after changing it. A test says when it is out of date.                                                                                                                                                                                                                                                     |
| `scripts/ios-signing.ts` | Signs the generated iOS project with an App Store profile and writes the export options, for the [TestFlight workflow](#from-github-actions). A test reads a made-up profile with it.                                                                                                                                                                                                                                                                  |
| `scripts/version.ts`     | The version to build, from the `mobile-v` tags raised by a part, for the [TestFlight workflow](#the-version). A test runs it in a made-up repository.                                                                                                                                                                                                                                                                                                  |
| `src/lib/observed.ts`    | What was last seen of a profile's computers and workspaces, kept in the cache folder.                                                                                                                                                                                                                                                                                                                                                                  |
| `src/lib/native.ts`      | What only the phone has: SecureStore, the cache folder, the system browser's sign-in session, randomness and SHA-256, the phone's name, and the system's picture picker.                                                                                                                                                                                                                                                                               |
| `src/ui/parts.tsx`       | The few native components the screens are built from, with the colours and fonts of [the design](../../docs/design/33-phone-design.md).                                                                                                                                                                                                                                                                                                                |
| `src/ui/composer.tsx`    | The field and the row of keys for typing into a terminal.                                                                                                                                                                                                                                                                                                                                                                                              |
| `src/ui/live.tsx`        | What screens share about being live: a channel held only while on screen, how far a computer can be reached, what was last seen meanwhile, and showing a profile's things only while it is in use.                                                                                                                                                                                                                                                     |
| `src/ui/leaving.ts`      | What is asked when signing out cannot reach the server.                                                                                                                                                                                                                                                                                                                                                                                                |
| `assets/`                | The icon and splash screen, as SVG and the PNGs built from them, and the embedded fonts with their licenses.                                                                                                                                                                                                                                                                                                                                           |
| `test/lib/`              | The logic, under `bun test`, against the real server and its sign-in (`crates/server/examples/stand-in.rs`, as the protocol client's tests use), and the terminal's page in real headless Chromium, skipped where no browser is found.                                                                                                                                                                                                                 |
| `test/screens/`          | The screens, under Jest with `jest-expo`, through Expo Router's in-memory router and Testing Library, against a small fake server.                                                                                                                                                                                                                                                                                                                     |

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
  back and with the profile. Taking control of a terminal, and what a device that may take control
  can do besides — start an agent, give one a picture — are the only things here that change
  anything on the computer.
- **Nothing is started twice.** A start whose answer was lost is said to be so, and never sent
  again by itself.
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

### On TestFlight

A build you can install and keep using, away from the Mac, for internal testers. It is the release
identity (`Yardsort`, `dev.yardsort.mobile`), so it sits beside the development app on the phone, with
its own sign-in: sign in and pair again. Each build lasts 90 days. It is archived in Xcode on a Mac, or
[by GitHub Actions](#from-github-actions), with no EAS. The first time only, register the app:

1. Run the recipe below once and choose your team in step 2, which should register
   `dev.yardsort.mobile`.
2. In [App Store Connect](https://appstoreconnect.apple.com), under **Apps**, **+**, **New App**: iOS,
   the name `Yardsort` (any other if it is taken: it is only a listing), the bundle ID
   `dev.yardsort.mobile`, and any SKU, such as `yardsort-mobile`. If the bundle ID is not offered, the
   form's **Register a new bundle ID** does the same, as an explicit ID with no capabilities: reload the
   form afterwards. Then in that app's **TestFlight** tab, add an internal testing group with yourself in
   it.
3. On the phone, install the **TestFlight** app and sign in with the same Apple ID.

Then, for each build:

1. `just mobile-testflight`, from your clone on the Mac. It gives the build a new number from the date
   and time in UTC, the version of the newest `mobile-v` tag (see [the version](#the-version); `git
fetch --tags` first), and opens it in Xcode. It replaces `ios/`, so `just mobile-ios` builds it again for
   development afterwards. With `APPLE_TEAM_ID` set, step 2 is done for you.
2. In Xcode, select the **Yardsort** target, then **Signing & Capabilities**, tick **Automatically manage
   signing**, and choose the paid team.
3. Set the destination at the top to **Any iOS Device (arm64)**, then **Product**, **Archive**.
4. When the Organizer opens, **Distribute App**, **TestFlight Internal Only**, **Distribute**.
5. Apple then processes it, and until that is done App Store Connect shows no build at all, though the
   Organizer says it was uploaded. A problem with the build comes by email. Once it is ready, the
   **TestFlight** app on the phone offers it.

#### From GitHub Actions

The **TestFlight** workflow (`.github/workflows/testflight.yml`) does the same with no Mac at hand:
**Actions**, **TestFlight**, **Run workflow**, from `main`. It generates the release identity,
signs it by hand with a distribution certificate and an App Store profile, archives it, and uploads it
for internal testers with an App Store Connect API key. The build number comes from the time in UTC, as
the recipe's does, so builds from either never go backwards. The run keeps the `.ipa` for a week, and
step 5 above follows as it does from Xcode. It first ran on 2026-11-07, with Xcode 26.6, and its build
reached the iPhone through TestFlight.

##### The version

TestFlight and the phone show a version beside the build number, such as **0.2.0 (261107.1412)**. It
is kept in git tags, `mobile-v0.2.0`, rather than in a file, since the workflow cannot push a commit to
`main` but can push a tag. **Run workflow** asks which part to raise:

| Pick    | From 0.2.3 | For                                    |
| ------- | ---------- | -------------------------------------- |
| `patch` | 0.2.4      | The default: fixes and small changes   |
| `minor` | 0.3.0      | Something new to use                   |
| `major` | 1.0.0      | A break, or the first release for real |
| `none`  | 0.2.3      | The same version again, as a new build |

It starts from the newest `mobile-v` tag in `main`'s history, or from `package.json`'s `0.1.0` before
there is one, and the run's summary shows what it built. Once Apple has the build, the run tags the
commit it built, so a run that fails takes no version, and the next one tries the same again.
`bun scripts/version.ts minor` prints what a run would build. The desktop app's versions are tags too,
`v0.22.2`, and the two are not related.

A run uses the picked branch's copy of the workflow, and that copy receives the secrets, so the
`testflight` environment admits `main` only (**Deployment branches and tags**): a run from any other
branch stops before it gets them. Merge first to build a branch.

It needs the app registered (steps 1 to 3 of the first time, above), and once, from a Mac:

1. **A distribution certificate you hold the key of.** In Xcode → Settings → Accounts, select the team,
   then Manage Certificates → **+** → **Apple Distribution**. Xcode's Organizer may have signed earlier
   builds with a certificate Apple keeps (cloud-managed), which cannot be exported, so make this one.
   In Keychain Access, under **My Certificates**, export **Apple Distribution: …** as a `.p12`, with a
   password.
2. **An App Store profile for it.** In the
   [Apple Developer site](https://developer.apple.com/account/resources/profiles/list), **Profiles**,
   **+**, **App Store Connect**, the App ID `dev.yardsort.mobile`, the certificate from step 1, and a
   name such as `Yardsort TestFlight`. Download it.
3. **An API key.** In App Store Connect, **Users and Access**, **Integrations**, **App Store Connect
   API**, **Team Keys**, **+**, with the **App Manager** role. Note its **Key ID** and the **Issuer ID**
   above the list, and download the `.p8`: Apple offers it once only.
4. **The secrets**, in a GitHub environment named `testflight` (the repository's **Settings**,
   **Environments**). The workflow says which are missing before it builds anything:

   | Secret                                  | What                                     |
   | --------------------------------------- | ---------------------------------------- |
   | `IOS_DISTRIBUTION_CERTIFICATE`          | The `.p12`, in base64                    |
   | `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD` | Its password                             |
   | `IOS_PROVISIONING_PROFILE`              | The `.mobileprovision`, in base64        |
   | `APP_STORE_CONNECT_KEY_ID`              | The key's ID                             |
   | `APP_STORE_CONNECT_ISSUER_ID`           | The issuer ID                            |
   | `APP_STORE_CONNECT_KEY`                 | The `.p8` as it is, `-----BEGIN` and all |

   With the GitHub CLI, from where the files are:

   ```sh
   base64 -i Certificates.p12 | gh secret set IOS_DISTRIBUTION_CERTIFICATE --env testflight
   gh secret set IOS_DISTRIBUTION_CERTIFICATE_PASSWORD --env testflight
   base64 -i Yardsort_TestFlight.mobileprovision | gh secret set IOS_PROVISIONING_PROFILE --env testflight
   gh secret set APP_STORE_CONNECT_KEY_ID --env testflight
   gh secret set APP_STORE_CONNECT_ISSUER_ID --env testflight
   gh secret set APP_STORE_CONNECT_KEY --env testflight < AuthKey_XXXXXXXXXX.p8
   ```

   The team and the profile's name are read from the profile. Delete the files afterwards.

The certificate lasts a year, and the profile ends with it: the run's **Sign with the profile** step
says until when. Then make both again and replace the first three secrets. A development or ad hoc
profile, or one for another app, is refused with what to make instead.

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
