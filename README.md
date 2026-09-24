<div align="center">
  <img src="assets/icon.svg" width="96" alt="" />
  <h1>Yardsort</h1>
  <p><strong>Run AI coding agents in parallel — each on its own track.</strong></p>
  <p>
    A desktop app for Linux, macOS and Windows that gives every task its own git worktree and its
    own terminal, running the coding agent of your choice.
  </p>
  <p>
    <a href="https://yardsort.sh">yardsort.sh</a> ·
    <a href="https://github.com/joaoh82/yardsort/releases/latest">Download</a> ·
    <a href="docs/quick-start.md">Quick start</a> ·
    <a href="docs/README.md">Documentation</a> ·
    <a href="CONTRIBUTING.md">Contributing</a> ·
    <a href="mailto:hello@yardsort.sh">Contact</a>
  </p>
  <p>
    <a href="https://github.com/joaoh82/yardsort/actions/workflows/ci.yml"><img src="https://github.com/joaoh82/yardsort/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue.svg" alt="License: GPL-3.0" /></a>
    <a href="https://github.com/joaoh82/yardsort/releases/latest"><img src="https://img.shields.io/github/v/release/joaoh82/yardsort?include_prereleases&label=release" alt="Latest release" /></a>
  </p>
</div>

![Yardsort: projects and workspaces on the left, an agent's terminal in the middle, its changes and a diff on the right](docs/images/overview.png)

## What it is

A sorting yard is where rail cars are sorted onto parallel tracks and later joined back into one
train. That is the job: fan work out onto parallel branches, then merge it back.

You describe a task, pick an agent, and press Enter. Yardsort creates a branch and a **git
worktree** for it — a separate folder — and starts the agent there in a real terminal. Start
another, and another. They cannot disturb each other, or your own checkout. On the right you
watch the files each one touches, live, with diffs.

It is modeled on tools like Conductor and Superset, with the requirement they do not meet:
**Linux, macOS and Windows are all first-class**, built and tested together on every change.

## Highlights

- **Any terminal agent.** Claude Code, Codex, Grok and OpenCode out of the box; add any other
  with a few lines of configuration — no plugin, no release to wait for.
- **The terminal is the truth.** Agents run in a real PTY with their own interface. Whatever they
  can do in your terminal, they can do here — and Yardsort never parses their output.
- **Plain git, no lock-in.** Workspaces are ordinary worktrees and branches. Inspect or undo
  anything with `git`. Worktrees made elsewhere can be imported, and never appear uninvited.
- **Closing the window doesn't stop them.** Terminals live in a small background process, so
  agents keep working while Yardsort is closed — or after it crashes. Open it again and every
  screen is repainted where it got to. Closing with work in flight asks first.
- **Pick up where you left off.** For conversations that really did end, press **Resume** — it is
  intact. **Fork** one to try a different approach without losing the first.
- **See what happened.** Live list of changed files, character-level diffs, a file tree, and
  one click into your editor.
- **Know who needs you.** Status dots show which agents are working and which are waiting; a
  desktop notification tells you when one finishes while you are elsewhere.
- **Careful with your work.** Deleting or archiving a workspace always keeps the branch, and
  never discards uncommitted changes without a second, explicit confirmation.
- **A second pair of eyes, if you want one.** [Assist](docs/guide/assist.md) asks
  [Jev](https://docs.typesafe.ai), TypeSafe's judgment model, small typed questions about each
  changed diff, and badges the files that look unrelated to the task, or that add a secret, weaken
  a test or switch a check off. It can also suggest a harness and an effort for the message you
  are typing. Off unless you bring your own TypeSafe API key — see
  [below](#assist-judgment-from-jev-if-you-want-it).
- **Private by construction.** No account, no telemetry, no keys of ours. Agents use their own
  logins; Yardsort just starts them. It talks to the network to check for new versions, which you
  can switch off — and, only if you switch Assist on and add your own key, to ask Jev about your
  diffs.
- **Keeps itself current.** Signed in-app updates on macOS, Windows and the Linux AppImage — one
  click, and your agents' conversations resume afterwards.
- **Scriptable.** [`ys`](docs/guide/cli.md), a small command-line client, starts a workspace and
  an agent without opening the window: `ys workspace new <project> "<prompt>"`. The agent belongs
  to the background process, so it carries on after the command returns — and `ys attach` puts it
  back on your terminal, `ys logs` prints what a session ended up with. Every command takes
  `--json`.
- **Light.** Built with [Tauri](https://tauri.app) and Rust: a few megabytes, not a bundled browser.

<table>
  <tr>
    <td width="50%"><img src="docs/images/composer.png" alt="The composer: describe a task, pick an agent, model, effort and branch" /></td>
    <td width="50%"><img src="docs/images/sessions.png" alt="Previous sessions with Resume and Fork" /></td>
  </tr>
  <tr>
    <td align="center"><sub>Describe the task, pick the agent, press Enter</sub></td>
    <td align="center"><sub>Come back later: Resume or Fork any conversation</sub></td>
  </tr>
</table>

## Install

Download the latest build from the [**Releases page**](https://github.com/joaoh82/yardsort/releases/latest):

| System      | File                                                                                                                                               |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Linux**   | `.AppImage` (portable), `.deb`, or `.rpm` (an AUR package, `yardsort-bin`, is on its way)                                                          |
| **macOS**   | `.dmg` — universal (Apple Silicon and Intel), signed and notarized — or `brew install --cask joaoh82/yardsort/yardsort`                            |
| **Windows** | `-setup.exe` or `.msi` — not code-signed yet: choose _More info → Run anyway_. (`winget install joaoh82.Yardsort` is awaiting Microsoft's review.) |

On Linux, the quick start shows how to
[add the AppImage to your app menu](docs/quick-start.md#linux-add-the-appimage-to-your-app-menu).

You also need **git** and at least one agent CLI that already works in your terminal (for
example [Claude Code](https://claude.com/claude-code)). Yardsort does not bundle agents and
never sees their credentials.

## Quick start

1. Open Yardsort and press **+** next to _Projects_ → **Open a folder** → choose a git repository.
2. Press **+** on the project (or `Ctrl+Shift+N` / `⌘N`), type what you want done, press **Enter**.
3. Watch the agent in the middle, and its changes on the right. Start more workspaces in parallel.
4. The result is an ordinary git branch — review it, push it, open a PR.

The [quick start guide](docs/quick-start.md) walks through it with pictures, and the
[documentation](docs/README.md) covers every part of the app. Both are also on the website, at
[yardsort.sh/docs](https://yardsort.sh/docs/).

## Assist: judgment from Jev, if you want it

Yardsort never parses what an agent prints — status, readiness and notifications come from
terminal activity alone, and that does not change. A diff, though, is text a model can be asked
about. **Assist** is the optional feature that does so, using [Jev](https://docs.typesafe.ai),
TypeSafe's judgment model.

Jev never generates text. It answers a _typed_ question about a piece of state — a yes/no
probability, a choice among named options, or a position on ordered levels — and Yardsort decides
what the number means. That keeps the interesting part in code: the questions are narrow (one
property each), the thresholds live in your settings, and the model is pinned (`jev-1.13.0`) so a
new version cannot quietly move under them.

![Settings → Assist, with the API key, the two features and the three thresholds](docs/images/assist.png)

**On the changes list**, shortly after an agent stops writing, each changed file is checked
against what the workspace was asked to do, and flagged files get a badge:

| Badge           | What it means                                                                       |
| --------------- | ----------------------------------------------------------------------------------- |
| **off-task**    | The change looks unrelated to what this workspace was asked to do.                  |
| **secret**      | The change looks like it adds a literal key, token or password.                     |
| **tests**       | The change looks like it deletes, skips or weakens a test.                          |
| **checks**      | The change looks like it switches a lint, type check or CI step off.                |
| **credentials** | The file's _name_ says it holds credentials — decided locally, contents never sent. |

**In the composer**, while you type the first message, Assist can offer a harness and an effort
level, built on **your** own "Good at" descriptions of your harnesses rather than on any opinion
of ours. Press **Use** to apply it; ignore it and nothing happens.

**You decide how sure is sure enough.** Three thresholds — flag a risky change at 70%, call a file
off-task at 60%, offer a suggestion at 50% — are settings with **Restore defaults**. Yardsort
caches Jev's answers rather than the badges, so moving a threshold re-reads what has already been
said: no new requests, no waiting.

**What it costs you:** Assist is off until you enter your own TypeSafe API key and tick a feature.
The key goes to your system credential store (Keychain, Credential Manager, Secret Service), never
into `settings.toml`, and requests are billed to your account. What leaves the machine is the diff
and path of a changed file plus the task, or the message you are typing plus your "Good at" texts
— nothing else, and never a terminal. Files whose names say they hold credentials are badged
without their contents being read. Nothing here is load-bearing: with no key, switched off,
offline or rate-limited, Yardsort behaves exactly as it does otherwise, minus a few badges.

The [Assist guide](docs/guide/assist.md) covers all of it, including what to do when TypeSafe
says no.

## Documentation

|                                                                                                  |                                                      |
| ------------------------------------------------------------------------------------------------ | ---------------------------------------------------- |
| [Quick start](docs/quick-start.md)                                                               | Download to first agent in five minutes              |
| [Projects](docs/guide/projects.md) · [Workspaces](docs/guide/workspaces.md)                      | Repositories, branches, worktrees, archiving         |
| [Terminals & sessions](docs/guide/terminals-and-sessions.md)                                     | Tabs, status dots, notifications, resume and fork    |
| [Changes & files](docs/guide/changes-and-files.md)                                               | Reviewing what an agent did                          |
| [Updates](docs/guide/updates.md)                                                                 | How new versions reach you                           |
| [Settings & harnesses](docs/guide/settings.md)                                                   | Configure agents, add your own                       |
| [Assist](docs/guide/assist.md)                                                                   | Optional Jev checks on changes and composer hints    |
| [The `ys` command line](docs/guide/cli.md)                                                       | Workspaces, agents, `attach` and `logs` from a shell |
| [Keyboard shortcuts](docs/guide/shortcuts.md) · [Troubleshooting](docs/guide/troubleshooting.md) |                                                      |
| [Design docs](docs/design/README.md)                                                             | Architecture, harness model, roadmap, open questions |

## Build from source

You need [Rust](https://rustup.rs) (stable), [Bun](https://bun.sh), git,
[`just`](https://just.systems), and Tauri's
[system dependencies](https://tauri.app/start/prerequisites/) for your OS.

```sh
git clone https://github.com/joaoh82/yardsort && cd yardsort
just setup     # install dependencies
just dev       # run with hot reload
just check     # formatting, lints, types, all tests
just build     # installers for this OS, in target/release/bundle/
```

`just` alone lists every recipe. See [CONTRIBUTING.md](CONTRIBUTING.md) for how the code is laid
out and how to send a change.

## Status

Early, and moving fast. The core loop — projects, parallel workspaces, configurable agents, live
review, resumable sessions — works on all three platforms and is covered by CI on each. Expect
rough edges, and please [report them](https://github.com/joaoh82/yardsort/issues/new/choose).
The [changelog](CHANGELOG.md) says what changed, and the [roadmap](docs/design/05-roadmap.md)
what is next.

## Contributing

Issues, ideas and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and the
[Code of Conduct](CODE_OF_CONDUCT.md). To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Get in touch

Bugs and feature requests belong in the
[issue tracker](https://github.com/joaoh82/yardsort/issues/new/choose), where everyone can find
them. For anything else — a question, help getting started, or just to say what you are building
with it — write to **[hello@yardsort.sh](mailto:hello@yardsort.sh)**.

## Team

[![João on X](https://img.shields.io/badge/Jo%C3%A3o-@codepolyglot-555?logo=x)](https://x.com/codepolyglot)

## License

[GPL-3.0](LICENSE). Yardsort is free software: you may use, study, share and change it, and
versions you distribute must stay free under the same terms.

Yardsort was called **Switchyard** until v0.2; if you used that, your projects and settings come
along automatically the first time you start Yardsort.

Yardsort is an independent project, not affiliated with Anthropic, OpenAI, xAI or any other
maker of the agents it can launch. Product names belong to their owners.
