# 06 — Open questions

Decisions still to make. Each has a current lean so work isn't blocked; move items out of here once
settled.

## Product

1. **What exactly does `local` run?** Lean: same as a workspace (harness or shell tabs), rooted at
   the repo's own checkout, on whatever branch is checked out. Should its row offer the composer too,
   or open straight to a shell?
2. ~~**Workspace naming.**~~ **Settled in M3:** a slug of the first message (filler words dropped,
   four words, 32 characters), numbered when taken; a railway station when the message yields
   nothing. Renaming arrives in M6.
3. ~~**Multiple sessions per workspace?**~~ **Settled by what shipped:** yes, as tabs, which was
   the lean. The tab bar's harness buttons start another conversation whatever is already running
   in that workspace, and Fork exists precisely to put a copy beside the original.
4. **Do we ever want our own chat UI?** The brief says the centre is a terminal, and that is v1.
   OpenCode (`opencode acp`) and others expose agent protocols that would allow a native UI later.
   Lean: not before v1; keep the core free of terminal-only assumptions where that's cheap.
5. **What happens to the branch when a workspace is deleted?** It is **always kept** — deleting
   never throws commits away, and the only `branch_delete` on a live path rolls back a creation
   that failed, on a branch we had just made. Offering to delete a fully merged branch was meant
   to arrive in M6 and did not, so the question is still open: what the offer should say, and how
   "fully merged" is judged when the base branch has itself moved on.

## Technical

6. ~~**When to ship the terminal daemon.**~~ **Settled in M9: shipped**, right after v0.4. The
   PTY host moved out of process into `yardsortd` — which is the app's own binary re-run with
   `--yardsort-daemon`, so there is nothing extra to bundle or sign. The lifecycle questions
   resolved as: **one daemon per data directory**; the app starts it when nobody answers, under a
   lock file; it stops itself once no client is connected and no session is running, or when the
   app's quit dialog says to; an **update** that changes the protocol replaces an idle daemon
   silently and leaves a busy one alone rather than kill work. "Background" on Windows is a
   `DETACHED_PROCESS` in its own process group behind a named pipe. See
   [03-architecture](03-architecture.md#the-daemon-yardsortd).
7. **Windows harness support.** Several harnesses officially target WSL rather than native Windows.
   Do we support launching harnesses _inside WSL_ (`wsl.exe -d <distro> -- claude …`, worktree on
   the WSL filesystem)? Lean: native first; treat WSL as a per-harness command prefix + path
   translation, designed in M4, built when someone needs it.
8. ~~**Worktree root default.**~~ **Settled in M3:** visible and short —
   `~/yardsort/<project>/<workspace>`; `YARDSORT_WORKTREE_ROOT` overrides it, and M4 makes it
   a setting.
9. ~~**Diff viewer.**~~ **Settled in M5:** CodeMirror 6 with its unified merge view — light, themable
   from our CSS variables, and one component serves both the diff and the read-only file viewer.
10. ~~**Frontend framework.**~~ **Settled 2026-09-17: React** (+ TypeScript, Vite, Tailwind,
    Zustand), for the component ecosystem.
11. **`stdin` transport readiness detection.** Quiet-period heuristic vs per-harness ready regex
    vs fixed delay. The quiet-period heuristic is what shipped — wait for output, then
    `stdin_ready_ms` of silence — and M9 moved it into `pty-host`, so delivery now outlives the
    window. What it still cannot tell is _what_ the program fell quiet waiting for. Seen while
    taking the 0.5.0 screenshots: Claude Code, on its first run in a new worktree, printed its
    banner, went quiet, and only then asked "Quick safety check: is this a project you trust?" —
    a paste would have landed in that dialog. The message survived only because Claude Code takes
    it on argv. A ready regex would not have helped either; what the heuristic is missing is that
    a prompt awaiting a _person_ looks exactly like a prompt awaiting a _message_.
12. **Discovering harness-chosen session ids** (Codex, OpenCode) by reading their session stores —
    worth the coupling, or is `latest-in-cwd` enough?

## Project

13. ~~**Licence.**~~ **Settled 2026-09-18: GPL-3.0**, and the project is open source from v0.1.
14. ~~**Name availability.**~~ **Settled 2026-09-19: renamed from Switchyard to Yardsort.** Every
    Switchyard domain worth having was taken, as were the GitHub, crates.io, npm and AUR names, and
    other products already use the name. `yardsort` was free everywhere checked (.com/.dev/.app/
    .net, GitHub, crates.io, npm, AUR, PyPI, RubyGems, Homebrew) and keeps the metaphor: a sorting
    yard is where cars are sorted onto parallel tracks. App id `dev.yardsort.app`, branch prefix
    `ys`. `legacy.rs` carries data over from the old app id and keeps `SWITCHYARD_*` working.
15. **Distribution.** Flatpak and/or Snap in addition to AppImage/deb/rpm/AUR? (Whether to be open
    source at all was question 13, and is settled.) Tracked in M7 as "on request", so this waits
    for someone to ask.

## Assist

16. **May Assist read an agent's screen?** Assist (2026-09-20) deliberately stays away from the
    terminal: it judges git diffs and the composer's text only. The tempting next step is to send
    the last screen from the headless VT when an agent goes quiet, and have Jev say _why_ —
    waiting for permission, asking a question, finished, or failed — so the notification can say
    it. That is worth real money to someone running five agents, and it is a deliberate exception
    to "the terminal is the truth, we never parse agent output", with a screen that may hold
    secrets. _Deferred on purpose; decide after living with the first two features._
17. **Assist threshold defaults.** The defaults (flag at 70%, off-task at 60%, suggest at 50%) were
    chosen by reading TypeSafe's guidance, not measured against real workspaces. They are settings
    now, so one machine can be tuned — but the defaults, and the wording of the questions
    themselves, still need a pass over real diffs. The right values may differ per repository,
    which the settings cannot express.
