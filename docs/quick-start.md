# Quick start

From nothing to an agent working on your code, in about five minutes.

## 1. Before you start

Yardsort runs coding agents **you already have installed**. It does not bundle or replace them,
and it never sees your API keys — each agent uses its own login. You need:

- **git**
- at least one agent CLI that works in your terminal. Out of the box Yardsort knows
  [Claude Code](https://claude.com/claude-code) (`claude`), [Codex](https://github.com/openai/codex)
  (`codex`), Grok (`grok`), [OpenCode](https://opencode.ai) (`opencode`),
  [OMP](https://omp.sh/) (`omp`), [Cursor](https://cursor.com/cli) (`cursor-agent`) and
  [Pi](https://pi.dev/) (`pi`). Anything else that
  runs in a terminal can be [added in settings](guide/settings.md#adding-your-own-harness).

Check that it works where Yardsort will look for it — a fresh terminal:

```sh
claude --version
```

## 2. Install

Download the latest build for your system from the
[**Releases page**](https://github.com/joaoh82/yardsort/releases/latest).

| System      | File                             | Notes                                                                                                                                                                                                              |
| ----------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Linux**   | `.AppImage`, `.deb` or `.rpm`    | AppImage: `chmod +x Yardsort_*.AppImage` and run it, or [add it to your app menu](#linux-add-the-appimage-to-your-app-menu). An AUR package (`yardsort-bin`) is on its way; until then the AppImage works on Arch. |
| **macOS**   | `.dmg` (Apple Silicon and Intel) | Open it and drag Yardsort to Applications. Or with Homebrew: `brew install --cask joaoh82/yardsort/yardsort`.                                                                                                      |
| **Windows** | `-setup.exe` or `.msi`           | Builds are not code-signed yet, so SmartScreen warns: choose **More info → Run anyway**. Needs [Git for Windows](https://git-scm.com/download/win).                                                                |

Prefer to build it yourself? See [CONTRIBUTING.md](../CONTRIBUTING.md) — it is `just setup && just build`.

### Linux: add the AppImage to your app menu

An AppImage is a single file that runs from wherever it is, so it does not appear in your launcher
by itself. Give it a permanent home and a menu entry — run this in the folder you downloaded it to:

```sh
mkdir -p ~/Applications ~/.local/share/applications ~/.local/share/icons/hicolor/scalable/apps
mv Yardsort_*.AppImage ~/Applications/Yardsort.AppImage
chmod +x ~/Applications/Yardsort.AppImage
curl -fsSL https://raw.githubusercontent.com/joaoh82/yardsort/main/assets/icon.svg \
  -o ~/.local/share/icons/hicolor/scalable/apps/yardsort.svg
cat > ~/.local/share/applications/yardsort.desktop <<EOF
[Desktop Entry]
Type=Application
Name=Yardsort
Comment=Run AI coding agents in parallel, each in its own git worktree
Exec=$HOME/Applications/Yardsort.AppImage
Icon=yardsort
Terminal=false
Categories=Development;
StartupWMClass=Yardsort
EOF
```

Yardsort now shows up in your launcher (GNOME, KDE, Walker on Omarchy, rofi, …); some need a
moment, or a new session, to notice. Two details matter:

- **Keep the version out of the file name.** [Updates](guide/updates.md) replace the AppImage in
  place, so a fixed name keeps the menu entry working after every update.
- **Keep it somewhere you can write to**, such as your home folder — otherwise it cannot update
  itself.

To remove it again, delete those three files. Your projects and settings live elsewhere
(`~/.local/share/dev.yardsort.app`) and are not touched.

## 3. First launch

Yardsort looks for git and for the agents it knows, and the welcome screen tells you what it
found. If something is missing it shows how to install it, with a command you can copy; install
it, press **Check again** — no restart needed — and carry on. When everything is in place the
screen points you at the next step: adding a project.

## 4. Add a project

Open Yardsort and press **+** next to _Projects_ (or `Ctrl+Shift+O` / `⌘O`).

- **Open a folder** — pick any git repository on your machine.
- **Create a new project** — Yardsort makes the folder, runs `git init` and adds a first commit.

Your project appears on the left with one entry under it, **local**: your repository exactly as it
is on disk. Click it and you get a shell there.

## 5. Start a workspace

Press the **+** on the project row (or `Ctrl+Shift+N` / `⌘N`) and say what you want done:

![The composer](images/composer.png)

Pick the agent, optionally a model and effort level, and press **Enter**. Yardsort then:

1. creates a new branch and a **git worktree** for it — a separate folder, so the agent cannot
   disturb your own checkout or any other agent;
2. starts the agent in that folder, in a real terminal, with your message as its first prompt.

The first time an agent sees a new folder it may ask whether you trust it; answer in the terminal
as you normally would.

## 6. Watch, review, repeat

![Yardsort at work](images/overview.png)

- The **middle** is the agent's own terminal UI. Type to it exactly as you would anywhere else.
- The **right** lists every file the agent has touched, updating live. Click one for the diff.
- The **left** shows all your workspaces. Start another — in the same project or a different
  one — and they run side by side. A pulsing dot means an agent is working; a number at the end of
  the row is how many agents there are waiting for you, and a **✓** means they have all finished.

When the work is done, it is an ordinary git branch: review it, push it, open a pull request —
from the agent, from a shell tab (`Ctrl+Shift+T`), or from your usual tools.

## 7. Come back later

Quit whenever you like. When you return, pick the workspace and press **Resume** — the agent
reopens with the whole conversation intact.

![Previous sessions](images/sessions.png)

## Next

- [Workspaces](guide/workspaces.md) — branches, archiving, cleaning up
- [Terminals & sessions](guide/terminals-and-sessions.md) — resume, fork, notifications
- [Settings & harnesses](guide/settings.md) — make an agent start the way you like
- [Keyboard shortcuts](guide/shortcuts.md)
