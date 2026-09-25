# Updates

Yardsort tells you when a newer version exists, and — where it can — installs it for you.

## How you find out

Shortly after starting, and once a day, Yardsort reads a small file from its GitHub releases to
see whether a newer version exists. Returning to the window looks again if the last check is a
day old, which is how a machine that spent the night asleep finds out. If a newer version exists,
an **update** pill appears beside **Settings** at the foot of the projects panel; press it to see
what the update is. Nothing is downloaded until you ask for it.

You can also look right now: **Settings → General → Check now**, which shows the version you are
running too. To stop the automatic check, untick **Check for updates automatically** there.

That file is the only thing Yardsort itself ever fetches from the network, and it sends nothing
about you or your work.

## Installing

Press the **update** pill. The dialog shows the release notes and, if terminals are running, how
many will be stopped by the restart. **Install and restart** downloads the update, verifies it,
installs it and starts the new version.

- **Your agents' conversations survive.** The restart stops running terminals, but afterwards you
  pick the workspace and press **Resume** — see [Terminals & sessions](terminals-and-sessions.md).
- **Updates are verified.** Each one is cryptographically signed by the project, and Yardsort
  refuses anything whose signature does not match the key it was built with.
- **Later** closes the dialog; the pill stays beside **Settings** until you update.

## Which copies update themselves

| How you installed it                                 | What happens                                                                                                                                                                                                                                |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS app (`.dmg`)                                   | Updates itself.                                                                                                                                                                                                                             |
| Windows installer (`-setup.exe`, `.msi`)             | Updates itself; Windows may show the installer's progress briefly.                                                                                                                                                                          |
| Linux **AppImage**                                   | Updates itself, replacing the AppImage file in place.                                                                                                                                                                                       |
| Linux `.deb` / `.rpm`, or the AUR package            | **Told, not updated.** Those files belong to your package manager, and an app must not change them behind its back. Update the way you installed — `yay -Syu`, or download the new `.deb`/`.rpm` from the release page the dialog links to. |
| A build you made yourself (`just dev`, `just build`) | Never updates itself.                                                                                                                                                                                                                       |

The [`ys` command](cli.md) comes with the app, so it is updated with it: through your package
manager, through the link into the macOS app, or — for an AppImage or on Windows — by Yardsort
replacing the copy it installed the next time it starts. See [Installing `ys`](cli.md#installing).

## If it goes wrong

- **"Could not check for updates"** — you are offline, or GitHub is unreachable. The automatic
  check stays silent about this; you only see it after pressing **Check now**.
- **The install fails** — nothing has been changed; your current version keeps working. Try again,
  or download the release by hand from the link in the dialog.
- **On Linux, the AppImage cannot be replaced** — it must be somewhere you can write to. If it
  lives in a system folder, move it to your home directory. The quick start's
  [app menu setup](../quick-start.md#linux-add-the-appimage-to-your-app-menu) puts it in
  `~/Applications` under a fixed name, which also keeps its menu entry working across updates.
