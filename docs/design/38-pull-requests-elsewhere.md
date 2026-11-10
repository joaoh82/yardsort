# 38 — Pull requests in other repositories

_Proposed 2026-11-09, and built the same day in five slices — see
[slice 1](#slice-1-what-shipped), [slice 2](#slice-2-what-shipped),
[slice 3](#slice-3-what-shipped), [slice 4](#slice-4-what-shipped) and
[slice 5](#slice-5-what-shipped) — with triggers on them added on 2026-11-10
([slice 6](#slice-6-what-shipped)). [M31](05-roadmap.md#m31--pull-requests-in-other-repositories)
tracks the hands-on pass on the three systems that is left._

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
  list; a **merged or closed** one is reused for ten minutes — they are final, or nearly —
  whatever a look's `refresh` says, since the window's own poll asks with `refresh` every
  minute. A person pressing **Refresh** in the view or **Refresh pull request** in the toolbar
  is the exception: that look is `asked` (`project_pull_requests`'s flag for it), and reads them
  all again; one whose read then fails keeps what was known, as always.
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
  **Refresh** and **Stop tracking**, and since slice 4 the three ways to merge it, by its link.
  Asking an agent to resolve its conflicts is not offered: that agent works in the project's
  repository.
- **The Pull requests view** lists each one under its workspace's project, with
  `owner/name` after the project's name, so the project filter finds it. The sidebar count
  includes the open ones. Its detail has **Go to workspace**, **Open on GitHub**, **Copy link**
  and **Stop tracking**, and a full **Summary** — description, checks with links, reviewers,
  conversation — read by link. Since slice 4 it also has **Merge**, **Close**, **Reopen** and the
  reply box, by its link, and **Code**, read in the project's nested clone of its repository.
- **Track pull request…** is in the workspace's menu in the sidebar: a link, and **Track**. A
  refusal stays in the dialog.
- **One that could not be read** is said so above the Pull requests view's rows, with **Retry**
  and **Stop tracking**:
  `alpha: demo/lib#7, tracked with feature, could not be read: …`. One that was never read has
  no row to show it on; one read before keeps its row, with what was known then, and the line
  says that is what shows.

Outcomes see only the project's own. Workflows are about the workspace's own pull request,
except one whose `context` is `tracked_pull_request`, started from a tracked one (slice 5) or by a
trigger on a tracked one's `pr.*` events (slice 6).

## Telling the agent

When an agent is started with a first message in a workspace whose **project's checkout holds
other repositories** — a folder directly inside it with a `.git` of its own — the message gets
one more section:

```text
## Other repositories

This project's checkout holds other git repositories, which are not in this workspace's
folder: `/home/…/flotta_parent/flotta`, `/home/…/flotta_parent/flotta-cloud`. Every workspace of
the project shares them, so to work on a branch of one, make a worktree of it for yourself
(`git -C <the clone> worktree add <a new folder outside it> -b <branch>`) rather than changing
the clone's own checkout. When you open a pull request in one of them, run `ys pr track <its url>`
so Yardsort follows it with this workspace.
```

Each clone is named by its whole path, and said to be outside the workspace's folder unless
the workspace is the project's own checkout, its local one. The worktree advice is because the
clones are one checkout each, shared by every workspace of the project: two agents switching
branches in one would undo each other's work. Like the smoke-test section, only for a fresh
first message: a handoff and a fork carry what
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
  links to GitHub. ✅
- **Acting on it.** Merge, close, reopen and comments are `gh pr merge|close|reopen|comment`
  with the link in place of the number, the same checks as today (`--match-head-commit`, a
  confirmation naming the repository). Slice 4. ✅
- **Code review.** _Request code review_ tells its reviewer it is in the pull request's
  worktree and to run `gh pr review <n>`. For a pull request elsewhere, the reviewer needs the
  nested clone — or a worktree of it — and the link. Slice 5 lets the workflow be run on a
  tracked pull request, gives the reviewer the clone to work in, and points `wait_pr_activity`
  at the link. ✅
- **Start workspace** is not offered: the repository is not a project. Adding it as one is
  what [24](24-repositories.md) is for.

## Slices

1. **Tracking and following.** ✅ The table, `ys pr track | tracked | untrack`, **Track pull
   request…**, the launch section, reading by link, and every surface above with the Summary.
   See [slice 1](#slice-1-what-shipped).
2. **Seen: Claude Code.** ✅ The `PostToolUse` rule, recorded fixtures of `gh pr create` and of
   a GitHub MCP tool, the exception written into 09. See [slice 2](#slice-2-what-shipped).
3. **Seen: the other adapters.** ✅ Codex, OpenCode, pi, OMP, Grok and Cursor, each from what it
   can see, recorded where the agent could be. See [slice 3](#slice-3-what-shipped).
4. **Acting on them.** ✅ Merge, close, reopen and the reply box by link; **Code** from the
   nested clone. See [slice 4](#slice-4-what-shipped).
5. **Code review on one.** ✅ The workflow on a tracked pull request, in the nested clone;
   review activity waited on by link. See [slice 5](#slice-5-what-shipped).
6. **Triggers on them.** ✅ The `pr.*` events of each tracked pull request, for a trigger whose
   workflow is about one elsewhere. See [slice 6](#slice-6-what-shipped).

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
- **The window hears of it at once** — when the app drained it. The app emits
  `TrackedPullRequestsChanged` with the projects whose workspaces gained one, and the window
  asks those projects' pull requests again instead of waiting for the next minute's look. The
  tracking runs on a thread of its own, since it runs git and the drain after an exit is on the
  daemon's event thread. When `ys` drained first — an agent ran a `ys` command right after — the
  window finds it at its next look, within a minute or on focus.
- **What `gh pr create` printed ends with its link**, so a command's output gives its _last_
  pull request link: `cat notes.md; gh pr create` must not track a pull request the notes name.
  The shell's own words and the common runners in front of `gh` leave it in command position:
  `if`, `then`, `!`, `nice -n 10`, `xargs`, and `timeout 60`.
- **Known gaps.** Without `git`, a seen link is counted and dropped, as is one the store would
  not take: its event is recorded, and is never handed on again. A fork whose parent repository
  is known only to `gh`'s own configuration, with no remote for it, can have its own pull request
  tracked as elsewhere.

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

## Slice 3: what shipped

Every other built-in agent now gives a `tool.completed` its `pullRequestUrl` when the call opened
a pull request, from whatever its channel carries, by the same rules
(`tracked::created_by_command`, `tracked::is_create_pull_request`, `tracked::link_in_tool_result`):

| Agent    | Where the command and its output are                                                                                                                                        | MCP                                                                    | Recorded                    |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | --------------------------- |
| Codex    | The session file's `CommandExecution` with exit code 0: the script after the shell's flag (`bash -lc`, `powershell -NoProfile -Command`, `cmd /c`) and `aggregated_output`. | `McpToolCall`: `github`/`create_pull_request`, `codex_apps`/`github_…` | 0.167.0                     |
| OpenCode | `tool.execute.after`: `args.command` and `output`; the plugin hands both on for a likely call only.                                                                         | `<server>_create_pull_request`, by name                                | 1.18.34                     |
| pi, OMP  | The command on `tool_execution_start`, the output on `tool_execution_end`; the extension pairs them by `toolCallId`.                                                        | pi has none; OMP's names not recorded                                  | pi 1.5.0, OMP 18.18.3       |
| Grok     | `chat_history.jsonl`, beside the event log: the call's arguments and its result, under the id `tool_completed` names.                                                       | By name, not recorded                                                  | Not yet: Grok not signed in |
| Cursor   | `postToolUse` of `Shell`, sent only for a success: `tool_input.command`, `tool_output`'s `output` and `exitCode`.                                                           | `afterMCPExecution`, which alone names the server                      | 2026.11.05-86047df          |

- **Likely, then decided.** OpenCode's plugin and the pi family's extension run inside the agent,
  where Yardsort's rules (Rust) do not. They pass on a call's command and output only when it
  _may_ have opened a pull request — a `bash` command naming `gh`, `pr` and `create`, or a tool
  whose name says GitHub and ends `create_pull_request` — and the executable decides, keeping the
  link alone. Every other call's command and output stay in the agent's process, as before.
- **A failed call opened nothing.** `gh pr create` on a branch that has a pull request already
  fails and prints _that_ one's link, so a call counts only when it succeeded: Codex's exit code,
  OpenCode's `metadata.exit`, Cursor's `postToolUse` (a failure is `postToolUseFailure`), the pi
  family's `isError`, Grok's `outcome`.
- **Codex's and Grok's come from files**, read in the drain. Codex's links ride on the events a
  turn's trigger expands into; Grok's on its `tool_completed` lines, looked up in the chat
  history — streamed, line by line — only for a finished call that could have opened one. Grok
  writes the log and the history apart, and a line is never read for a link twice, so a recent
  call the history has not caught up with is left, with what follows it, for the next read, for
  up to two minutes. Both reach `track_seen` like the inbox's.
- **`scripts/record-pr-creation.sh <agent>`** runs each agent's own recording script with a
  prompt that opens a pull request — `gh pr create`, and for Codex, OpenCode and Cursor a GitHub
  MCP server's `create_pull_request` — against the same stand-ins as Claude Code's (a `gh` that
  prints a link, an MCP server that answers as GitHub's), into
  `crates/core/fixtures/<agent>-pull-requests/`. Found while recording: Codex's `exec` refuses an
  MCP call under its default approval policy unless the server's tools are approved
  (`default_tools_approval_mode`); Cursor's shell takes `PATH` from the login shell, so the stand-in
  is named by path; Codex has its own GitHub app tool, `codex_apps`/`github_create_pull_request`;
  and **OpenCode 2 does not load Yardsort's plugin at all** — its plugins are folders with a new
  module shape — so the OpenCode recording, and the adapter, were 1.x's. Since ported, and
  recorded against 2.2.6, whose shell tool is `shell` and whose MCP calls run inside its
  `execute` tool ([13 § 8](13-agent-events-stage-2-opencode.md#8--opencode-2)).

How it was checked: each adapter against its recording — the link on exactly the call that
opened it, nothing of the command, output or answer anywhere in a payload; hand-made negatives
(a `gh pr view`, a failed call, another forge's tool); OpenCode's plugin and the extension under
vitest, played through their real handlers, handing on a likely call's command and output and
nothing of any other; Grok's chat-history lookup, and its links handed on once through `import`;
and the real `ys` binary as Cursor's, OpenCode's and pi's hook, then `ys pr tracked` showing it by
`hook`. Not yet done: a recording of Grok, and the three systems' hands-on pass.

## Slice 4: what shipped

A tracked pull request is acted on as a project's own is, by its link instead of its number:

- **Merge, Close, Reopen** in the Pull requests view's detail, and the three merges in the
  session toolbar's menu: `tracked_pull_request_merge | close | reopen`, which read the pull
  request afresh by its link first and refuse as the project's own do — one no longer open, a
  draft, or a head that moved since the confirmation (`--match-head-commit`). Each confirmation
  names it `owner/name#7`. `gh pr merge|close|reopen <url>`: `gh` takes the link and needs no
  repository around it, so it runs in the project's checkout, or anywhere once that has gone.
- **The reply box** in its Summary: `tracked_pull_request_comment`, which runs
  `gh pr comment <url> --body-file -` with the words on standard input.
- **Code**: `tracked_pull_request_changes` finds the project's **nested clone** of the pull
  request's repository (`tracked::clone_of`: a folder directly in the checkout with a `.git` of
  its own and a remote that is that repository, host included), fetches the pull request's
  commits there into refs of Yardsort's own (`refs/yardsort/pull/<n>/…`) as a project's own are
  fetched into its checkout — nothing checked out, no branch moved — and reads the diff between
  them; `tracked_pull_request_diff` reads one file there. With no such clone, **Code** says so,
  with a link to the diff on GitHub. The comments on its lines are read from its own repository,
  named, on its own host (`tracked_pull_request_line_comments`, `gh api --hostname`). **Stop
  tracking** deletes the refs when no workspace of the project follows it any more.
- **Fetching in someone's clone.** The nested clone is the user's own, and they or an agent may
  be working in it, so the fetch leaves `FETCH_HEAD` alone (`--no-write-fetch-head`; a git
  older than 2.29 fetches as before), as every pull request fetch now does. The remote fetched
  from, and the clone itself, are the ones on the pull request's own host first, then by owner
  and name alone — a clone with the repository on two hosts fetches from the right one, and one
  reached through an SSH host alias is still found. The clone is found once per pull request.
  The refs are keyed by number, so two tracked pull requests with the same number in one clone
  share them, and a workspace deleted with its rows leaves them behind: both only cost a fetch.
- **Not offered for one elsewhere**: **Start workspace** (its repository is not a project),
  asking an agent to resolve its conflicts (that agent works in the project's repository), and
  **Note on lines…** (a note goes to the workspace's agent about the workspace's own pull
  request, or onto lines by number). Slice 5 is about review on one.

How it was checked: the by-link `gh` calls against a stand-in `gh` — the link in each command, the
comment on standard input, line comments asked of the named repository on its host; the diff
read in a real nested clone with the commits where a fetch puts them, nothing in the clone
changed, and a missing clone refused with where to look instead; and the window through Testing
Library — the actions offered and not, a close confirmed naming the repository and sent by link
(a "no" sends nothing), a reply sent by link, **Code** failing without a clone and then reading
the files, and a merge from the toolbar sent by link at the head shown. Not yet done: the three
systems' hands-on pass.

## Slice 5: what shipped

**Request code review…** in a tracked pull request's details runs a new built-in workflow,
**Request code review elsewhere** (`code-review-elsewhere`), on that pull request:

- **A run about a tracked pull request.** A workflow's `trigger.context` may now be
  `tracked_pull_request`. Such a run is queued with the pull request, read by its link by the
  caller (`publish::tracked::for_a_run`) and open, in `Request::tracked`; it is refused without
  one — `ys workflow run`, a schedule, a trigger — and a workspace workflow is refused one. The
  Workflows view disables **Run…** and **Schedule…** for it, and the Schedules and Triggers
  dialogs do not offer it. `pr.number`, `pr.url` and `pr.title` are that pull request's, and two
  new variables say where it is: `pr.repo` (`owner/name`) and `pr.clone`, the project's nested
  clone of it. The checker refuses both in a workspace workflow, where they would always be
  empty. The run dialog, given the tracked pull request, offers only such workflows, and blocks
  **Run** with the reason when the pull request cannot be used.
- **The clone, when it is used.** A workflow whose steps use `pr.clone` is refused without the
  project's nested clone of the repository (`runs::tracked_problem`); the built-in uses it, since
  its reviewer runs the checks there. Any other problem finding the clone is said as itself.
- **The reviewer** starts in the workspace like any other, and is told to read the diff with
  `gh pr diff <link>`, to run the checks in a worktree of the clone of its own and remove it
  afterwards, and to post with `gh pr review <link>`. The worktree is made with
  `git worktree add --detach`, and the pull request checked out in it with
  `gh pr checkout <link> --detach`: a linked worktree has its own `HEAD` and `FETCH_HEAD`, so
  the clone's own checkout is left alone.
- **Waited on by link, for a review.** `Hands::pr_posts` is given the run's `PrFacts`, and asks
  `gh pr view <selector>` — the number for a workspace's own, the link for one elsewhere
  (`PrFacts::selector`) — from the workspace's folder, or anywhere if that has gone. The
  built-in waits for a review (`kind: review`), not any comment: the author, a bot or a person
  may comment meanwhile.
- **Told to the agent that opened it.** The tracked row's conversation is kept as
  `Elsewhere::opened_by`; at the run's start, `session: origin` is that one if its terminal is
  running, else the workspace's own agent, as for any run. The author's prompt names `gh api`'s
  `--hostname` for a pull request off github.com.

How it was checked: queueing in the core — the tracked one's facts and variables, the workspace's
own not looked for, the refusals both ways, the selector; the workflow's file parsed with every
variable it uses known; the by-link `pr view` against a stand-in `gh`; the run dialog through
Testing Library — only the workflows about one offered, previewed and started with the tracked
id, the agent that opened it named, and refused without a clone — and the button in the Pull
requests view opening it. Not yet done: a real review run on a real tracked pull request, and the
three systems' hands-on pass.

## Slice 6: what shipped

Built 2026-11-10, after a workspace of `flotta_parent` tracking three pull requests in two nested
clones had a trigger on `pr.review`/`pr.comment` and one on `pr.opened` that never fired: the
watch ([36](36-triggers.md)) asked only about the workspace's own branch, and a workflow about one
elsewhere was refused a trigger.

- **Which trigger hears which.** A trigger whose workflow's `context` is `tracked_pull_request`
  hears the `pr.*` events of every pull request its workspace tracks, and none of its own; any
  other hears its own and none elsewhere. The workflow decides, not a choice of the user's: an
  `{{ pr.* }}` is the one pull request the run is about, and a workspace workflow's would be the
  wrong one. It is kept on the trigger as `Filters::elsewhere`, set from the workflow when the
  trigger is saved, so the watch thread and `observe` need no workflow file. A workflow edited to
  the other context afterwards has its fires refused by the door, with the reason.
- **One watch per tracked pull request**, `tracked_pr_watches` (migration 0028), keyed by the
  tracked row and going with it, holding what `pr_watches` holds but the branch. The rules are
  `watch::compare`'s, unchanged; `watch::look_at_tracked` asks by repository and number
  (`Gh::pr_look_in`: `gh api graphql -F owner=… -F name=…`, `--hostname` off github.com) from the
  project's checkout or anywhere, as slice 1's reads do. `watch::due` lists each as a
  `Watched` of its own, and drops a watch nothing listens to. A merged or closed one stays
  tracked until someone untracks it, so it is looked at every ten minutes
  (`LOOK_CLOSED_EVERY_MS`), as the window reads one, for its reopening.
- **Being tracked is opening.** The pull request exists before Yardsort hears of it, so a first
  look that is not a seed would never see one open. An agent tracks the one it has just opened,
  so the first look of one tracked in the last ten minutes (`OPENED_WITHIN_MS`) tells
  `pr.opened`, dated when it was tracked, with the reviews and comments posted since. Dating it
  then, rather than at the look, is what keeps it from a trigger saved after the tracking whose
  seed GitHub did not answer. A first look later than that — the three rows that were already
  there when this shipped — only seeds: their reviews from before are not news.
- **Counted by link.** A fire's subject is `pr:<link>` for one elsewhere, never `pr:<number>`, so
  the cap, the gathering and `from: anyone`'s own-run check never mix lib#7 with the project's #7.
- **One run at a time per workspace, kept on purpose.** Found in review: the lock that makes a
  fire wait (`workflow_active_in`, and the door's `workflow_already_running`) is per workflow and
  workspace, so while `address-review-elsewhere` waits up to two hours for its agent on one
  tracked pull request, a review on another is **waiting**, retried at every tick and started
  when the first run ends, with what came meanwhile. Making it per subject was weighed and left:
  the agent that opened them is usually the same one, and two runs typing into it at once would
  collide. The cap stays per link. A workspace that wants two addressed side by side needs the
  lock per tracked pull request in both places.
- **The run is about the one that set it off.** At the fire, the tracked row is found by the
  subject's link — gone if it was untracked: **refused** — and read again by its link through
  `Look::tracked_pull_request`, which in the app is slice 5's `for_a_run`: open, with its clone and
  the conversation that opened it. That goes to the door as `Request::tracked`.
- **Refused when saved:** such a workflow on any event but `pr.*`, and on `pr.merged` or
  `pr.closed`, after which it is not open. The dialog greys those out too once such a workflow
  is chosen (found in review).
- **Address reviews elsewhere** (`address-review-elsewhere`) is the new built-in for the second of
  that workspace's triggers: `address-review`, telling `session: origin` — the agent that opened
  it — with the commands by link. It does not use `pr.clone`, so it runs without the clone.
- **The window.** The trigger dialog offers such workflows under **On a pull request it tracks
  elsewhere** unless an event other than `pr.*` is ticked, and greys the rest out once one is chosen; the
  sentence and the list say "a pull request it tracks elsewhere" — "the checks on a pull request it
  tracks elsewhere" for the check events, which first read as a possessive of the long name (found
  in review); the trigger's details list each
  one watched (`TriggerItem::watching_elsewhere`).

How it was checked: the watch and the fires in the core against a real store and a forge from a
table — a review on lib#7 starting `address-review-elsewhere` about lib#7 and not the trigger on
the workspace's own #7, and the reverse; opening soon after tracking and not long after, nor for
a trigger saved after the tracking; the refusals; untracking dropping the watch and refusing what
was gathering; the arguments `gh` is given, against a stand-in `gh`; `ys trigger add` end to end
with the real binary; the dialog and the list through Testing Library. Not yet done: the app by
hand against a real tracked pull request, and the three systems' hands-on pass.

## The hands-on pass

The rows are [08 § 28](08-manual-checklist.md#28--pull-requests-in-other-repositories).

- **Linux, 2026-11-09, without the window.** A real Claude Code (2.1.319), started by
  `ys workspace new` in a parent project with a nested clone, under a throwaway profile kept by
  `yardsort-host serve`, with a stand-in `gh`: it ran `gh pr create` in the clone and the host's
  drain tracked the pull request by `hook`, linked to its conversation, keeping only the link.
  **Found:** the launch section named the clone as `lib (in <parent>)`, and the agent went looking
  for `lib` in its own worktree — a worktree of the parent, which has none of them. It now names
  each clone by its whole path, says they are outside the workspace's folder (unless it is the
  project's local one), and points at a worktree of the clone rather than its shared checkout.
  Run again on that build, the agent went to the clone by itself and its pull request was
  tracked the same way.
- **Linux, 2026-11-09, the window.** Rows 3–7 in the dev window of 0.24.0, against a stand-in
  `gh` that keeps one pull request's state: tracking and its refusals, the Summary and a reply,
  **Code** from the clone with the clone untouched, close, reopen and a toolbar merge each
  confirmed by name, a real Claude Code reviewing it in a worktree of the clone and the run
  seeing the review by link, and **Stop tracking** from two places. All passed. Found: **Refresh
  pull request** did not read a merged or closed one again within its ten minutes — fixed
  since: a look a person asked for (`asked`) reads them all.
- **Still to do:** every row on macOS and Windows, which this machine cannot do, and the window's
  rows against real GitHub pull requests.

## Documentation this touches

`docs/guide/commits-and-pull-requests.md` (a workspace's pull requests elsewhere),
`docs/guide/pull-requests.md` (their rows), `docs/guide/cli.md` (`ys pr`), `CHANGELOG.md`,
[03-architecture](03-architecture.md) (the tracked table and how it is read),
[05-roadmap](05-roadmap.md) (M31), [06-open-questions](06-open-questions.md) (56–57),
[09](09-agent-events-and-memory.md) and [11](11-agent-events-stage-2-claude.md) (the exception,
in slice 2), [12](12-agent-events-stage-2-codex.md)–[16](16-agent-events-stage-2-cursor.md) (each
adapter's coverage, in slice 3), `docs/guide/activity.md` (what each agent's reporting keeps), and
this doc's index line in [README](README.md).
