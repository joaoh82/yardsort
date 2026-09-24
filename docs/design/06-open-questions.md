# 06 — Open questions

Decisions still to make. Each has a current lean so work isn't blocked; move items out of here once
settled.

## Product

1. ~~**What exactly does `local` run?**~~ **Settled 2026-09-21: it asks.** Clicking `local` offers
   **Open Terminal** or **Open Composer** — the lean was right that it should run the same things
   as a workspace, and the answer to "composer or straight to a shell" is neither by default. The
   composer for `local` starts the agent in the checkout on the branch it has out, creating no
   branch and no worktree; it is an ordinary `pty_spawn` into an existing workspace, so the
   conversation is recorded and can be resumed or forked like any other. It only asks when there
   is nothing running and nothing to resume, which is exactly when a worktree would have opened a
   shell by itself.
2. ~~**Workspace naming.**~~ **Settled in M3:** a slug of the first message (filler words dropped,
   four words, 32 characters), numbered when taken; a railway station when the message yields
   nothing. Task clauses after background prose are preferred; fenced code and recognized
   request lead-ins are skipped. Manual renaming changes the display label only.
3. ~~**Multiple sessions per workspace?**~~ **Settled by what shipped:** yes, as tabs, which was
   the lean. The tab bar's harness buttons start another conversation whatever is already running
   in that workspace, and Fork exists precisely to put a copy beside the original.
4. ~~**Do we ever want our own chat UI?**~~ **Settled 2026-09-21: no.** The centre stays a real
   PTY running the real CLI, as principle 2 says. The effort goes into the UI _around_ the
   terminal instead — the experience to match is [Superset](https://superset.sh), the app Yardsort
   is modelled on (see [01-vision](01-vision.md)), which is macOS-only. Agent protocols
   (`opencode acp` and friends) are not being taken up, but the core stays free of terminal-only
   assumptions where that is cheap, so the door is not nailed shut.
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
7. ~~**Windows harness support.**~~ **Settled 2026-09-21: the lean stands.** Native Windows
   first. WSL is a per-harness command prefix plus path translation — `wsl.exe -d <distro> --
claude …` with the worktree on the WSL filesystem — which M4's harness model already has the
   shape for, and which gets built when somebody needs it rather than on spec.
8. ~~**Worktree root default.**~~ **Settled in M3:** visible and short —
   `~/yardsort/<project>/<workspace>`; `YARDSORT_WORKTREE_ROOT` overrides it, and M4 makes it
   a setting.
9. ~~**Diff viewer.**~~ **Settled in M5:** CodeMirror 6's merge view — light, themable
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

16. ~~**May Assist read an agent's screen?**~~ **Settled 2026-09-21: yes** — build it. Assist
    stays away from the terminal today: it judges git diffs and the composer's text only. The
    decision is that it may send the last screen from the headless VT when an agent goes quiet,
    and have Jev say _why_ — waiting for permission, asking a question, finished, or failed — so
    the notification can say it instead of "is waiting".

    It is a deliberate exception to "the terminal is the truth, we never parse agent output", so
    the exception is narrow and the design is the work, not the plumbing. What still has to be
    decided before it ships: what exactly is sent (the visible screen, not scrollback), how a
    secret on screen is kept out of it, whether it is opt-in per project or per workspace on top
    of Assist's own switch, and what the notification is allowed to repeat. Never on a shell —
    only a harness session, and only when it has fallen quiet.

    _2026-09-24:_ the [agent-events work](10-agent-events-stage-1.md) neither builds nor depends
    on this. If it ships, its verdict has a place to go — an `evaluation.recorded` event from
    producer `assist`, method `screen`, fidelity `inferred` — and the event store takes nothing
    from it meanwhile. Stage 1 reads no terminal output.

17. ~~**Assist threshold defaults.**~~ **Settled 2026-09-23: they stand as shipped.** The defaults
    (flag at 70%, off-task at 60%, suggest at 50%) came from reading TypeSafe's guidance rather
    than from a pass over real diffs, and that is accepted rather than fixed. They are settings:
    a threshold is moved in Settings → Assist, and because the raw answers are cached, moving one
    re-reads what Jev already said instead of asking again — so the cost of a default being a few
    points off is a badge someone adjusts once, not a wrong answer. Building the labelled corpus
    it would take to choose better numbers — and to re-measure every time a question's wording
    changes, which moves the distribution under the thresholds — is more work than the noise it
    would save. The known limit stands: the right values may differ per repository, which the
    settings cannot express. If that turns out to bite, the change is per-project thresholds, not
    a better global default.
18. ~~**Which worktrees become workspaces by themselves?**~~ **Settled 2026-09-23: only ours.**
    M3's adoption took _every_ linked worktree of a repository, which made a project that already
    used worktrees for its own reasons open full of stale workspaces. The model now matches
    Superset's: a workspace comes from the composer or from an explicit **Import** that shows
    branch and path first. The one exception keeps a real behaviour: worktrees under Yardsort's
    own worktree root are still adopted, so removing and re-adding a project brings its
    workspaces back without a dialog. Pure Superset (no adoption at all) was considered and
    rejected for that case. **Forget** is the reverse — the row is hidden, not deleted, so the
    conversations are still there if the same worktree is imported later; the user can choose to
    drop them. Workspaces adopted by earlier versions are not migrated: nothing distinguishes
    them from ones Yardsort made, and Forget is one click. Removing a project got the same
    treatment the next day (2026-09-24): it hides the project and its rows rather than deleting
    them, so an imported worktree and its conversations survive a remove and re-add — which
    adoption, now narrowed, could no longer promise.

19. ~~**May a model write the words?**~~ **Settled 2026-09-23: yes, and not with Jev.** Assist
    cannot do it — Jev answers typed questions and never writes text — so having a commit message
    or a pull request written is its own feature with its own switch. The decision that shapes it:
    **no new credential of ours by default.** The coding agent the user already has does the
    writing through the non-interactive mode it already has, in the worktree, billed to the
    account it already uses; a user's own Anthropic API key is the fallback for when no configured
    agent can. Nothing is applied by itself — the answer lands in the box the user was already
    looking at, and the commit confirmation still stands between it and git.

    What is still open, and deliberately unanswered until there is use to learn from: whether the
    prompts should learn a repository's own commit style (reading `git log` would be the obvious
    way, and the obvious way to make every message sound the same); whether a failed agent should
    fall through to the key silently, as it does now, or say which one answered; and whether the
    120-second agent timeout is anywhere near right for a cold agent on a large repository.

## Agent events

20. **Which native surface first, per harness?** Stage 1 of
    [09-agent-events-and-memory](09-agent-events-and-memory.md) records only what Yardsort itself
    sees. The coverage matrix in [10 §6](10-agent-events-stage-1.md#6--stage-0-baseline-and-the-stage-2-coverage-matrix)
    is a reading of each CLI's `--help` on one day, not fixtures. The lean: Claude Code hooks first
    (documented, and the CLI shows a hook mechanism), recorded as fixtures for the exact version
    before an installer is written; Codex second, choosing between its trust-gated hooks and its
    session files once samples exist; OpenCode's plugin third. OMP, Cursor, Grok and Pi stay
    lifecycle-only until someone finds a stable surface. Not decided: whether an adapter's
    receiver lives in the app process or in the daemon, which the fit report deliberately left
    for after the owner/crash boundary was tested.
