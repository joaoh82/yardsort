# 38 — Pull requests in other repositories

_Proposed 2026-11-09. Slices 1 and 2 are built — see [slice 1](#slice-1-what-shipped) and
[slice 2](#slice-2-what-shipped). Slices 3–5 are to do;
[M31](05-roadmap.md#m31--pull-requests-in-other-repositories) tracks them._

A project is one repository, and every pull request Yardsort shows is one of that repository's.
Some projects are not used that way. A **parent repository** holds other repositories as
nested clones — not submodules, just clones in folders the parent ignores — and an agent working
in one of the parent's workspaces goes into those clones and opens pull requests there. Those
pull requests appear nowhere in Yardsort: not on the workspace's row, not in the session
toolbar, not in the Pull requests view. Their checks and reviews have to be followed on GitHub.

This doc makes a workspace able to **track** pull requests in other repositories, and follow
them as it follows its project's own: number and checks on the row and in the toolbar, a row in
the Pull requests view with its Summary, and — in later slices — the actions, the diff and code
review.

## How a workspace finds its pull requests today

Pull requests are fetched **per project**: one `gh pr list` in the project's checkout, the
newest fifty of any state, plus every open one while the Pull requests view is showing
([22](22-pull-requests.md)). A workspace then claims the ones whose branch is its branch, whose
head commit is in its history, or whose branch it checked out before the pull request was opened
(`pull_requests_from` in `crates/core/src/forge.rs`). Everything that shows a pull request reads
that one list — the sidebar badge, the toolbar, the view, outcomes, workflows — and everything
that acts on one names it by **project and number**.

A pull request in another repository is in no project's list, so nothing can claim it, and its
number means nothing without its repository.

## The layout this is for

Measured on the user's machine on 2026-11-09, in two projects that work this way:

| Project            | Its repository             | Nested clones in its checkout                                |
| ------------------ | -------------------------- | ------------------------------------------------------------ |
| `flotta_parent`    | `joaoh82/flotta_parent`    | `flotta/`, `flotta-cloud/` (and a staging clone of `flotta`) |
| `rustunnel-parent` | `joaoh82/rustunnel-parent` | `rustunnel/`, `rustunnel-web/`, `docs/`, and four more       |

Two things follow from what is on disk:

- **The nested clones are only in the project's main checkout.** They are ignored by the
  parent, so a Yardsort worktree of the parent has none of them. An agent in a workspace leaves
  its worktree for `~/Projects/flotta_parent/flotta`, or for a git worktree of that clone it
  makes itself (`flotta-worktrees/…`), and runs `gh pr create` there.
- **So the pull request has no tie to the workspace that git or the forge can see.** Its
  branch is in another repository, its commits are not in the worktree's history, and its
  author is the same login as every other session's. Only the session that opened it knows.

## Decisions

Settled with the user on 2026-11-09:

1. **Track, then follow.** A workspace keeps a list of pull requests in other repositories,
   each with the session that opened it when that is known, and Yardsort follows each one as it
   follows the project's own.
2. **Two ways a pull request gets on the list, both built** — the recommendation, taken:
   - **Told** (slice 1): `ys pr track <url>`, which an agent is pointed at when it starts in a
     project with nested clones, and **Track pull request…** in the app, for anything else.
   - **Seen** (slices 2–3): the agent adapters notice a tool call that _created_ a pull request
     — `gh pr create`, or a GitHub MCP tool that creates one — and keep its link. That is one
     narrow exception to [09](09-agent-events-and-memory.md)'s _metadata only_: the link is
     read out of the tool's output and nothing else of it is kept. Recorded there when it is
     built.
3. **Every agent.** Claude Code is the one used most, and goes first; Codex, OpenCode, pi, OMP,
   Grok and Cursor follow, each saying whether its tool output can be read at all.
4. **The nested-clone layout is the one designed for.** Where a later slice needs a local copy
   of the other repository — the diff, a reviewer's checkout — it looks for a clone of it in the
   project's main checkout first.
5. **One doc, followed through the slices**, each slice its own pull request with its docs,
   tests and changelog line.

Detection by searching GitHub for your recent pull requests was considered and set aside: two
sessions running at once cannot be told apart by time, and the author is the same login.

## What a tracked pull request is

A row in `tracked_pull_requests` (migration 0027):

| Column         | What it holds                                                                        |
| -------------- | ------------------------------------------------------------------------------------ |
| `id`           | Its own id.                                                                          |
| `workspace_id` | The workspace it is followed with. Deleting the workspace deletes the row.           |
| `session_id`   | The conversation that opened it, when known. Set to null if that record goes.        |
| `repo`         | `host/owner/name`.                                                                   |
| `number`       | Its number in that repository.                                                       |
| `url`          | Its link, rebuilt from the three above: `https://host/owner/name/pull/number`.       |
| `tracked_by`   | `app` (**Track pull request…**), `cli` (`ys pr track`), `hook` (seen, from slice 2). |
| `created_at`   | When it was tracked.                                                                 |

One row per workspace and link: tracking it twice changes nothing. Two workspaces may track the
same pull request; each shows it.

Only GitHub links are accepted, as with every other pull request in Yardsort. A pull request of
the **project's own repository** is refused with the reason — it is followed already, and
listing it twice would put it twice in the view. A link that names no pull request — an issue,
a commit — is refused before anything is written; anything after the number, like `/files`, is
dropped. Nothing is asked of the forge to track a pull request: a link to one that does not
exist, or that this login cannot see, is tracked, and the Pull requests view says what `gh` said
about it, with **Retry** and **Stop tracking**.

**Stop tracking** removes the row and nothing else. It is Yardsort's own bookkeeping, not anyone's
work, so it is not confirmed.

## Following one

`gh pr view <url> --json <the list's fields>`. `gh` takes a link in place of a number and then
needs no repository around it, so the same reader serves every repository, from the project's
checkout as a working folder. Measured: 0.5 s for one pull request.

- Read with the project's own list (`project_pull_requests`), in the app's `Forge` beside it:
  an **open** one is read again when the list is refreshed and otherwise reused for 30 s, like the
  list; a **merged or closed** one is reused for ten minutes — they are final, or nearly.
- **At most 20 per project per look**, newest tracked first, **four at a time**, each stopped
  after 20 s. One that two workspaces track is read once. A pull request that cannot be read
  keeps what was last read of it, with the reason beside it; past the twenty, a row says it was
  not read and why.
- They arrive in the project's answer as `ProjectPullRequests::tracked`, each with its
  workspace, repository, number, link, how it was tracked, the pull request as read (the same
  `PullRequest` the project's own are) and the problem, if any.
- Who opened it, what its checks say and which reviews it has are the same fields as for the
  project's own; the sidebar badge, the toolbar and the view's row draw them the same way.

## Where they show

- **The workspace's row** shows them after its own, `flotta#207` — the repository's name and the
  number, since a bare number would read as the project's. Its checks colour it as always.
  Hovering lists them with the others under _Also opened from this workspace_.
- **The session toolbar** offers them in its menu beside the workspace's own, labelled with the
  repository. Chosen, the badge shows it, and the menu has **View on GitHub**, **Copy PR link**,
  **Refresh** and **Stop tracking**. Merging and resolving conflicts are not offered for one in
  slice 1.
- **The Pull requests view** lists each one under its workspace's project, with
  `owner/name` after the project's name, so the project filter finds it. The sidebar count
  includes the open ones. Its detail has **Go to workspace**, **Open on GitHub**, **Copy link**
  and **Stop tracking**, and a full **Summary** — description, checks with links, reviewers,
  conversation — read by link. The reply box, **Code**, **Merge**, **Close** and **Reopen** come
  in slice 4; until then **Code** says so and links to the diff on GitHub.
- **Track pull request…** is in the workspace's menu in the sidebar: a link, and **Track**. A
  refusal stays in the dialog.
- **One that could not be read** is said so above the Pull requests view's rows, with **Retry**
  and **Stop tracking**:
  `alpha: demo/lib#7, tracked with feature, could not be read: …`. One that was never read has
  no row to show it on; one read before keeps its row, with what was known then, and the line
  says that is what shows.

Outcomes and workflows still see only the project's own. A workflow that waits on a review
(`wait_pr_activity`) waits on the workspace's own pull request; slice 5 is the one about that.

## Telling the agent

When an agent is started with a first message in a workspace whose **project's checkout holds
other repositories** — a folder directly inside it with a `.git` of its own — the message gets
one more section:

```text
## Other repositories

This project's checkout holds other git repositories: flotta, flotta-cloud (in
/home/…/flotta_parent). When you open a pull request in one of them, run
`ys pr track <its url>` so Yardsort follows it with this workspace.
```

Like the smoke-test section, only for a fresh first message: a handoff and a fork carry what
the conversation already knew. At most ten are named. The note is the fallback once the adapters
see pull requests being created, and stays: an agent that opens one in some other way — the
browser, an API call — is still pointed at the command.

`ys pr track <url>` finds its workspace and session the way `ys memory propose` does — the
launch environment, or the folder it is run in — or takes `--workspace`. `ys pr tracked` prints the
workspace's tracked pull requests; `ys pr untrack <url>` removes one. The app sees what `ys`
wrote the next time it asks about the project: within a minute, or on focus.

## Seeing one created (slices 2–3)

The adapters already receive each tool call. For Claude Code, `PostToolUse` carries the
command (`tool_input.command`) and what it printed (`tool_response`), and today both are dropped
on the floor. The change:

- A tool call **creates a pull request** when it is a shell command that runs `gh pr create`,
  or an MCP tool whose name is a GitHub server's create-pull-request tool.
- From such a call's output, the first `https://<host>/<owner>/<name>/pull/<n>` is kept as
  `pullRequestUrl` on the `tool.completed` event, and becomes a tracked row by `hook` for the
  session the event belongs to. Nothing else of the input or the output is stored. A link to
  the project's own repository is ignored, since that one is followed already.
- The match is deterministic and tested against recorded payloads, like everything else in the
  adapters. Recorded in [09 § privacy](09-agent-events-and-memory.md) as the exception it is.

Each other adapter gets the same rule from whatever its agent exposes: Codex's session file
has command output, OpenCode's and pi's plugins see tool results, Grok's log and Cursor's hooks
need to be checked for it. An adapter that cannot see output says so in its coverage, and the
note and `ys pr track` cover it.

## What does not carry over at once

- **The diff.** **Code** fetches a pull request's commits into the project's repository, which
  has none of the other repository's history. Slice 4 fetches into the nested clone instead,
  when there is one in the project's checkout whose remote is that repository, and otherwise
  links to GitHub.
- **Acting on it.** Merge, close, reopen and comments are `gh pr merge|close|reopen|comment`
  with the link in place of the number, the same checks as today (`--match-head-commit`, a
  confirmation naming the repository). Slice 4.
- **Code review.** _Request code review_ tells its reviewer it is in the pull request's
  worktree and to run `gh pr review <n>`. For a pull request elsewhere, the reviewer needs the
  nested clone — or a worktree of it — and the link. Slice 5 lets the workflow be run on a
  tracked pull request, gives the reviewer the clone to work in, and points `wait_pr_activity`
  at the link.
- **Start workspace** is not offered: the repository is not a project. Adding it as one is
  what [24](24-repositories.md) is for.

## Slices

1. **Tracking and following.** ✅ The table, `ys pr track | tracked | untrack`, **Track pull
   request…**, the launch section, reading by link, and every surface above with the Summary.
   See [slice 1](#slice-1-what-shipped).
2. **Seen: Claude Code.** ✅ The `PostToolUse` rule, recorded fixtures of `gh pr create` and of
   a GitHub MCP tool, the exception written into 09. See [slice 2](#slice-2-what-shipped).
3. **Seen: the other adapters**, one by one, each with its coverage stated.
4. **Acting on them.** Merge, close, reopen and the reply box by link; **Code** from the nested
   clone.
5. **Code review on one.** The workflow on a tracked pull request, in the nested clone; review
   activity waited on by link.

## Tests

- The store: tracking once and twice, two workspaces, the cascade when a workspace is deleted,
  the session nulled when its record goes, untracking.
- Links: GitHub's shapes (`/pull/7`, `/pull/7/files`, a trailing slash, `http`, a GitHub
  Enterprise host), and refusals — an issue, a commit, not a link, the project's own repository.
- Reading: the arguments `gh` is given, against a stand-in `gh`; the cache's two lifetimes, the
  cap, a failure keeping the last answer, a forgotten one read again.
- `ys pr track` in the CLI's own tests: from the launch environment, from a folder, with
  `--workspace`, and refused outside a workspace.
- The launch section: present with a nested clone, absent without, absent for a handoff.
- The window, through Testing Library: the badge's name and number, the toolbar's menu, the row
  in the view and its Summary read by link, **Stop tracking**, **Track pull request…** and its
  refusal shown in the dialog.

## Open questions

[56–57 in open questions](06-open-questions.md#pull-requests-in-other-repositories).

## Slice 1: what shipped

Built 2026-11-09, as proposed above, with these differences and findings:

- **The project's own list is untouched.** A tracked pull request is not folded into
  `ProjectPullRequests::pull_requests` or the `workspaces` map, whose numbers every action takes
  as the project's: it travels beside them in `tracked`, and the window puts the two together
  (`shownFor` in `src/stores/publish.ts`), keying one elsewhere by its link, since two hosts can
  each have an `owner/name#7`. So nothing that merges, closes or resolves conflicts by number
  can be handed one by mistake; the toolbar's merge returns early for one as well.
- **Read by link, with no `-R`.** `gh pr view <url>` was checked against a real pull request in
  a nested clone's repository, from a folder that is not a repository at all: every field the
  list asks for, 0.5 s. So the reader is `Gh::pull_request_at`, and the Summary is
  `Gh::pull_request_summary_at` — the same two queries as for a project's own, with the link
  where the number was.
- **Its Summary is kept apart.** `Forge::summary` is keyed by project and number; a tracked one is
  kept under `tracked:<link>` so a #7 elsewhere is never served as the project's #7.
- **A link that cannot be read** needed somewhere to say so, since every surface draws a row from
  a pull request as read — the notice line above the view's rows. A refresh that fails after a
  read that worked says so there too: the row's checks and reviews are then the old ones.
- **Its Summary needs no folder.** Archiving removes a workspace's folder and keeps its row, and
  its tracked pull requests still show, so the Summary reads the workspace's row, not its folder.
- **`ys pr tracked`, not `ys pr list`.** The Floor Manager ([37](37-floor-manager.md)) landed
  first with `ys pr list` for every project's open pull requests, so the workspace's tracked ones
  are `ys pr tracked`, in `crates/cli/src/commands/pr_tracked.rs` beside it.
- **The CLI looks at the launch environment first.** An agent that opened its pull request in a
  nested clone is standing outside its worktree — in the project's main checkout, which is the
  `local` workspace's folder — so the folder would name the wrong workspace. The run or workspace
  in the environment wins, as for `ys memory propose`. Tested from a folder that belongs to no
  workspace.
- **A session id that names no record of this workspace** — an environment left over from
  somewhere else — is dropped rather than refused: the pull request is still the workspace's.

How it was checked: the store, the link rules, the refusals and the launch section with unit
tests against real repositories; `ys pr` end to end with the real binary and a real database;
the cache with unit tests; the window through Testing Library — the row's badge, the dialog and
its refusal, the toolbar's menu without merge, the view's row beside the project's own #7, the
Summary read by link — also once the workspace is archived — **Code**'s link to GitHub, the
unread notice, before and after a first read, two hosts with the same `owner/name#7` in the
toolbar, and **Stop tracking** from each place. Not yet done: the app by hand against a real
nested clone, and the three systems' hands-on pass.

## Slice 2: what shipped

Built 2026-11-09. A pull request Claude Code opens is tracked with its workspace, by `hook`, as
soon as the inbox is drained, with nobody typing its link — when **Capture what Claude Code
reports** is on, which it is not by default. The launch section and `ys pr track` stay as
they were: they cover the switch off, and every other agent until slice 3.

- **Claude Code says so itself.** Recorded against 2.1.319 (`scripts/record-claude-pr-hooks.sh`,
  fixtures in `crates/core/fixtures/claude-hooks-pull-requests/`), a `Bash` call that ran
  `gh pr create` comes back with `tool_response.gitOperation.pr` —
  `{ number, url, action: "created" }`. That is the first rule: Claude Code's own judgement,
  not ours. A `gitOperation.pr` with any other `action` is taken at its word, as not a creation.
- **The command, when it does not.** For a version without `gitOperation`, a `Bash` call whose
  command runs `gh pr create` — in command position, after `cd … &&`, a variable set for it,
  `env`, `sudo` or a path to `gh`, with `-R owner/name` between — gives the first pull request
  link on its stdout. `gh` as an argument (`echo gh pr create`, a commit message) does not count.
  The command is split as a shell splits it: a separator inside quotes is text, and comments
  and here-document bodies are left out, so `printf '…; gh pr create …'` or a here-document
  holding the words, both of which print a link without opening anything, do not count either.
  A `$(gh pr create)` inside double quotes is missed on purpose — a link missed is still in the
  launch note's reach, a link invented tracks the wrong pull request.
- **The MCP tool.** `mcp__<server>__create_pull_request` from a server whose name says GitHub
  (`github`, `claude_ai_GitHub`). Its answer, recorded, is text blocks holding JSON; the link is
  its `html_url` when there is one — a full answer names other pull requests in its description —
  then a `url` that is a pull request's, then the first link anywhere. An API address
  (`/repos/…/pulls/9`) and a branch's "open a pull request" link (`/pull/new/…`) are not links.
- **Only the link is kept**, as `pullRequestUrl` on the `tool.completed` event; the adapter's
  tests check that nothing else of either recorded call — the title, the body, the command, the
  MCP answer's id — reaches the payload. A failed call keeps nothing.
- **Tracked when drained, not in the hook.** Telling the project's own repository apart needs
  `git`, which the hook has no business running. `import_inbox` hands on the links of events it
  recorded for the first time (`InboxReport::pull_requests`), and every drainer tracks them with
  `tracked::track_seen`: the app's watcher and its drain on exit, `yardsort-host`, and `ys`
  before any command that drains — `ys pr tracked` now drains first. The import at start is a
  drain too: `settle_at_start` returns the links it took (`Settled`, which must be used), and
  the app tracks them once its state, and so the login shell's environment `git` is found by,
  exists; `yardsort-host` at once. `git` is found only when there is something to track. An event drained twice is handed on once, so a pull request
  untracked since is not tracked again. Without `git` nothing is tracked and
  `pull_request_seen_no_git` is counted; a link `track` refuses for another reason counts
  `pull_request_seen_refused`.
- **The window hears of it at once.** The app emits `TrackedPullRequestsChanged` with the
  projects whose workspaces gained one, and the window asks those projects' pull requests again
  instead of waiting for the next minute's look.

How it was checked: the adapter against both recordings and against hand-made payloads for a
version without `gitOperation`, a `gh pr view`, a failed call and a full GitHub answer; the
link, command and tool rules with unit tests, quoted and here-document text among them; a
report left in the inbox while nothing ran, tracked from the start and not again by the drain
after; `track_seen` against a real repository with an
`origin`, refusing the project's own and doing nothing without `git`; the drain handing on a
link once; the real `ys` binary as the hook with the recorded payloads, then `ys pr tracked`
showing it by `hook`; the sidebar's badge appearing on the event. Not yet done: a real Claude
Code session in a real parent project opening a real pull request, and the three systems'
hands-on pass.

## Documentation this touches

`docs/guide/commits-and-pull-requests.md` (a workspace's pull requests elsewhere),
`docs/guide/pull-requests.md` (their rows), `docs/guide/cli.md` (`ys pr`), `CHANGELOG.md`,
[03-architecture](03-architecture.md) (the tracked table and how it is read),
[05-roadmap](05-roadmap.md) (M31), [06-open-questions](06-open-questions.md) (56–57),
[09](09-agent-events-and-memory.md) and [11](11-agent-events-stage-2-claude.md) (the exception,
in slice 2), `docs/guide/activity.md` (what the Claude Code hook keeps), and this doc's index line in
[README](README.md).
