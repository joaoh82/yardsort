# Changelog

Notable changes in each release. The [releases page](https://github.com/joaoh82/yardsort/releases)
has the downloads and the full commit lists.

## Unreleased

- **Worktrees removed with git are noticed.** Delete a workspace's worktree _and_ its branch
  yourself, and Yardsort marks the workspace **gone** and asks whether to delete it from the app
  too — a "keep" is remembered. See [Workspaces](docs/guide/workspaces.md#when-the-branch-goes-too).
- Install with Homebrew on macOS: `brew install --cask joaoh82/yardsort/yardsort`.
- Linux: the quick start shows how to give the AppImage a launcher entry and icon.
- Submitted to winget (`joaoh82.Yardsort`), pending Microsoft's review.

## 0.3.1

- A changelog (this file).
- First release delivered through the in-app updater: a 0.3.0 install offers it by itself.

## 0.3.0

- **In-app updates.** Yardsort looks for new versions shortly after starting and once a day, shows
  an **Update to x.y.z** button, and installs on request — signed and verified. The macOS app,
  Windows installers and the Linux AppImage update themselves; `.deb`, `.rpm` and AUR installs are
  told a new version exists. See [Updates](docs/guide/updates.md).
- **First-run check.** The welcome screen says whether git and at least one agent were found and,
  if not, how to install them — with **Check again**, no restart needed. The status bar's
  `env: …` indicator re-reads your environment from anywhere.
- Settings → General gains **Check for updates automatically** and **Check now**.

## 0.2.0

- **Renamed from Switchyard to Yardsort.** Projects, workspaces, session history and settings are
  carried over automatically on first launch. New branches are prefixed `ys/`; `sy/` branches
  keep working, as do the `SWITCHYARD_*` environment variables.
- An AUR package, `yardsort-bin`, is prepared and will be published by the release workflow.

## 0.1.0

First public release, as _Switchyard_.

- Projects and workspaces: every task gets its own git worktree and branch.
- Real terminals running the agent of your choice — Claude Code, Codex, Grok, OpenCode, or any
  terminal agent you configure.
- Live list of changed files with diffs, a file tree, and one click into your editor.
- Resume and fork agent conversations; status dots and desktop notifications.
- Rename, archive, restore and delete workspaces, never losing uncommitted work silently.
- Linux, macOS (signed and notarized) and Windows builds.
