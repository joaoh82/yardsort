# The phone app's design

_Built on 2026-11-04, ahead of the owner's first real use of the app. On an Android emulator so far;
not yet seen on a real phone._

The phone app worked, but looked plain: off-white and green, unlike the desktop app, with a default
icon. This gives it the desktop app's colours and a more developer-like look, with a proper icon. It
changes how the app looks, and moves one button. The app's states, its wording and what it says to a
server are unchanged, apart from the two fixes under [What changed besides the look](#what-changed-besides-the-look).

The design is in Figma, in the Yardsort project:
[Yardsort phone app](https://www.figma.com/design/ezlPCcxVNBFKP0bkGODMHZ). Its pages are Foundations,
Components, Screens and App icon. Names in it are made up.

## The look

- **Colours.** The desktop's canvas, surface and amber accent (`src/styles.css`), with a light set
  beside the dark one, following the phone's setting. They are in `apps/mobile/src/ui/parts.tsx` and
  in the Figma file's variables under the same names. Change them in both.
- **Light, dark or automatic** is a choice in the app's Settings, reached from the account screen and
  from Connect. Automatic, the default, follows the phone. It changes the screens at once and tells the
  system too, for alerts and the keyboard. It is kept in a small file in the app's documents folder, read
  before the first screen is drawn so the other mode never flashes, and is never sent to a server. It may
  go along in a phone's backup, since it says nothing about anyone.
- **The terminal stays dark in all of them**, at `#0b0d10`, with an amber cursor.
- **Type.** Inter for text and JetBrains Mono for what a developer would read as data: addresses,
  branches, reach, times and section headings. Both are embedded under the SIL Open Font License,
  with the licenses beside them in `apps/mobile/assets/fonts/`.
- **Each weight is its own font family**, so nothing sets `fontWeight`, which would make the system
  substitute a font. Each file is named by its PostScript name, because iOS finds a font by that name
  and Android by its file's. `test/screens/look.test.ts` holds the names and the files together.
- **Rows in groups.** Lists are rows inside one rounded group with hairlines between them, under a
  small mono heading with a count, in place of a card for each thing.
- **Status is a dot and words.** Online is filled and offline hollow, so it does not rest on colour
  alone. The words stay either way.

## The icon

Icon A of the three in the Figma file: the logo, one line forking into three, with the middle line in
amber, on `#14171c`. The sources are SVG, in `apps/mobile/assets/`, and the PNGs are built from them:

```sh
cd apps/mobile/assets
rsvg-convert -w 1024 -h 1024 icon.svg -o icon.png
magick icon.png -background '#14171c' -alpha remove -alpha off icon.png   # the App Store refuses alpha
rsvg-convert -w 1024 -h 1024 icon-foreground.svg -o icon-foreground.png   # Android's adaptive icon
rsvg-convert -w 512 -h 512 splash.svg -o splash.png
```

## What changed besides the look

- **Unpair moved** from the list of computers to the computer's own screen, at the bottom. In the
  list, a destructive button sat beside every computer, next to the row you tap to open it. It asks
  the same question. If the server cannot be reached the phone stays on that screen and says so.
- **Done after pairing** returns to the account already underneath. It used to put a second account
  screen on top, with a back arrow leading to the first. This was there before the redesign.

## Where it differs from the Figma file

- **The terminal's text is the system's monospace font**, not JetBrains Mono. The terminal's page
  measures its cells from the font, and it has a strict content policy that an embedded font would
  have to pass. That is a risk to the part of the app that matters most, for a small gain.
- **Android's header titles are on the left**, as Android's are. Figma draws the iOS header.
- **Settings is not in the Figma file.** It was added after the design, from the same parts.
- **The row of keys scrolls.** The eight keys, at a size a thumb can hit, are wider than a phone, so
  the last one sits partly off the edge until the row is scrolled.

## What was checked, and on what

Checked on the Android emulator, against the stand-in server, on 2026-11-04: the fonts, launcher icon
and splash screen; Connect, Account, Pair (typing a code, waiting, approved), the computer, a
workspace, approved memory, Servers and a computer offline; the terminal watching, refused while the
person at the computer types, in control, sending a line, and handing back. Dark and light were both
looked at, apart from a few of those in one mode only. The screen tests cover the states.

The appearance choice was checked there too: Dark on a light phone, Automatic following the phone as it
changed, and Dark still chosen after quitting and reopening.

An emulator is not a phone. Not checked: anything on an iPhone, which includes whether iOS finds the
embedded fonts, how its header looks, whether its alerts and keyboard follow the choice, scanning a code, the largest text sizes, and anything on a
physical Android phone.

## Smoke test on the iPhone

This needs a new build, because fonts, the splash screen and SVG drawing are native additions: an
older build of the app cannot run this code. Use staging and your real computer, as before.

1. **Build it.** From your clone on the Mac: `git fetch origin && git checkout ys/remote-phone-redesign && just mobile-ios`.
2. **The icon and the splash.** The home screen shows the forked line on near-black, not a default
   icon. Force-quit and open the app: the same logo on a dark screen, with no white flash.
3. **The fonts.** This is the check most likely to fail on iOS. On the account screen, the server's
   address and the small section headings should be visibly monospaced, and everything else Inter. If
   the headings look like ordinary system text, the fonts were not found.
4. **Dark and light.** On the account screen, open **Settings** and choose **Dark**: the app changes
   at once, header and status bar included, and stays dark after force-quitting it. Choose **Light**
   too, then **Automatic**, which follows iOS Settings, Display & Brightness. Everything stays readable
   and the terminal stays dark throughout. Sign out's question should match the mode, as should the
   keyboard (**Cancel** the question).
5. **A real terminal.** Open your computer, a workspace, then a terminal. Its text should line up in
   columns as on the computer. **Take control**, type a line, **Send**, then **Hand back**.
6. **Unpair moved.** On the computer's screen, **Unpair** is at the bottom and no longer in the
   list. Tap it, then **Cancel**: nothing changes.
7. **Compare with the Figma file.** Note anything you dislike. Taste is the point of this slice.
