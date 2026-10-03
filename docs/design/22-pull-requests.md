# Pull requests

_Proposed 2026-10-02. Slices 1 to 3 are built — see [slice 1](#slice-1-what-shipped),
[slice 2](#slice-2-what-shipped) and [slice 3](#slice-3-what-shipped), which also say where they
differ from the proposal below. Slice 4 is still a proposal._

Every pull request of every project in one place: a **Pull requests** row at the top of the
sidebar opens a view in the center panel that lists them, filters them, shows one in detail —
its description, checks, reviewers, conversation and diff — and acts on it: start a workspace
from it, merge it, close it, reopen it. Today a pull request is only visible from the workspace
that opened it ([commits & pull requests](../guide/commits-and-pull-requests.md)); a teammate's
pull request, or one opened from another machine, is invisible until you leave the app.

Everything goes through `gh`, as it does now. Yardsort still holds no forge credential.

## Decisions

Ten questions were put to the user on 2026-10-02. The answers, and what follows from each:

1. **One row at the very top of the sidebar, above Workflows**, with a count of open pull
   requests. It opens the view in the center panel, the way a workflow and Usage do. It is a row
   and not a section: the list needs the width of the center panel, and a second list squeezed
   into the sidebar would be a worse copy of it.
2. **Every pull request of every project's repository**, whoever opened it and wherever from.
   A row whose pull request has a workspace says which, and goes there.
3. **Read first.** The first release is the list, the filters and a read-only **Summary**:
   description, checks with links, reviewers, conversation. The diff (**Code**) is the next
   slice.
4. **Writing to the forge comes last**: a reply box, comments on lines, and sending selected
   lines of a diff to a workspace's agent. They need the read views to stand on.
5. **Start workspace, Merge, Close, Reopen, Open on GitHub and Go to workspace** are all in the
   first release. Merge and Close are confirmed first; so is Reopen, which notifies people and
   starts CI again.
6. **Filters: state, project, author, review status, search** — remembered across restarts.
   The review filters that depend on who you are (_Reviewed by you_, _Not reviewed by you_,
   _Awaiting review from you_) are in. _Awaiting review from you or your team_ is not, yet: it
   needs your teams in each repository's organisation, which is a second question per
   organisation that the list does not otherwise ask. See
   [not in this version](#not-in-this-version-on-purpose).
7. **Every open pull request up to 200, and the 50 newest of any state.** The answer stands;
   the means changed once it was measured — see [what was measured](#what-was-measured).
   `gh pr list` cannot fetch 200 pull requests with their checks from a busy repository at all.
8. **GitHub only.** Everything past the link is `gh`. A project on another forge, or with no
   remote, is named in the view as not covered rather than left out without a word.
9. **In the command palette, with a shortcut you can bind but no default key**, like Usage.
10. **One design, four slices**, each its own pull request with its docs, tests and changelog
    line: the list with filters and actions; Summary; Code; writing.

## What was measured

Against real repositories on 2026-10-02 with `gh` 2.102.0, because decision 7 rests on it:

| Query                                                            | Repository                    | Result                               |
| ---------------------------------------------------------------- | ----------------------------- | ------------------------------------ |
| `gh pr list --state all --limit 50`, the fields asked for today  | this one                      | 2.5 s                                |
| the same, plus author, review requests and latest reviews        | this one                      | 2.8 s                                |
| `gh pr list --state all --limit 50`, today's fields              | `rust-lang/rust` (1,394 open) | 4.5 s                                |
| `gh pr list --state open --limit 200`, today's fields            | `rust-lang/rust`              | **HTTP 504** after 11 s              |
| `gh pr list --state open --limit 200`, with the new fields       | `cli/cli` (73 open)           | **failed four times in five**, ~11 s |
| our own query, 100 rows a page, check _counts_                   | `rust-lang/rust`              | 7–11 s — at GitHub's 10 s limit      |
| our own query, 50 rows a page                                    | `rust-lang/rust`              | 4.5 s, 2 rate-limit points           |
| our own query, 25 rows a page                                    | `rust-lang/rust`              | 2.2 s                                |
| our own query without checks, line counts or reviews, 100 rows   | `rust-lang/rust`              | 2.0 s                                |
| `gh pr view` with body, comments, reviews, files and every check | `cli/cli`                     | 0.5 s                                |

What it says:

- `gh pr list --json statusCheckRollup` asks for every check of every pull request, a hundred
  pull requests to the page. On a repository with real CI that is more than GitHub will answer
  in its ten seconds. The failures are HTTP 502, HTTP 504 and a truncated body `gh` reports as
  `unexpected end of JSON input`.
- A row costs roughly 60–100 ms on a busy repository whatever is done, so the page size is the
  lever: **50 rows a page** stays well inside the limit.
- GitHub will count checks by state for us (`checkRunCountsByState`), which is all a list row
  shows: _12/12_. The names and links are only needed for the one pull request in the detail
  pane, and `gh pr view` fetches those in half a second.
- The query that exists today is already slow on a busy repository (4.5 s for 50 rows). It is
  left alone here; folding it into the lighter one is [open question 26](06-open-questions.md).

## Where the data comes from

Two tiers into one list per project, in the cache `crate::publish::Forge` already keeps.

**The recent tier** is what exists today, unchanged in shape and cadence:
`gh pr list --state all --limit 50`, asked once per project, reused for 30 s, asked again on
focus and every minute.
It gains four fields — `author`, `reviewRequests`, `latestReviews`, `isCrossRepository` — so a
row looks the same whichever tier it came from. It carries every check's name, which the
workspace preview's **Show checks** needs.

**The open tier** is new: every open pull request, most recently updated first, up to 200.
`gh api graphql` with a query of our own, 50 rows a page, pages asked for one after another with
the cursor, each with its own time limit (`Gh::run_within`):

```text
query ($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: 50, after: $after,
                 orderBy: { field: UPDATED_AT, direction: DESC }) {
      totalCount
      pageInfo { hasNextPage endCursor }
      nodes {
        number url title isDraft state createdAt updatedAt
        headRefName headRefOid baseRefName isCrossRepository
        additions deletions mergeable reviewDecision
        author { login }
        reviewRequests(first: 10) {
          nodes { requestedReviewer { __typename ... on User { login } ... on Team { slug } } }
        }
        latestReviews(first: 10) { nodes { state author { login } } }
        commits(last: 1) { nodes { commit { statusCheckRollup {
          contexts(first: 1) {
            checkRunCountsByState { state count }
            statusContextCountsByState { state count }
          }
        } } } }
      }
    }
  }
  viewer { login }
}
```

`owner` and `name` are filled in by `gh` itself: `-F owner={owner} -F name={repo}` are its own
placeholders for the repository of the folder it runs in, so the open tier asks about exactly the
repository `gh pr list` picked there — in a clone of a fork, the parent. `--hostname` is passed
only for a GitHub that is not github.com, because `gh api` otherwise asks its default host
whatever the folder's remote says. `viewer` comes along for nothing and is what the _by you_
filters compare against. This query was run against both repositories above, and again through
`Gh::open_pull_requests` once it existed: 73 of 73 open pull requests of `cli/cli`, in two pages.

The open tier is fetched **once per project when the app starts** (so the sidebar's count is
right and the view opens with rows in it), and then **only while the view is showing**: when it
opens, on focus, every minute, on **Refresh**, and after any action. With the view closed, the
forge traffic is what it is today.

How the two meet:

- One list, keyed by number. A pull request both tiers have is the **recent tier's**: it is asked
  at least as often, it knows when one was merged or closed, and it has the checks by name. The
  open tier adds the open pull requests the newest fifty did not reach.
- A pull request the open tier used to list and no longer does, and the recent tier does not
  have either, was merged or closed outside the newest 50: it is dropped.
- A page that fails does not empty the list. What arrived is shown, the recent tier still
  stands, and `problem` says what `gh` said, with **Retry**.
- More open than the cap: the view says _Showing the 200 most recently updated of 1,394 open_
  and links to the forge for the rest.

Everything that reads the list today reads the fuller one — the workspace badges, the toolbar,
outcomes, workflows. A workspace whose open pull request has fifty newer ones in front of it has
no badge today, because the list stops at fifty; with the open tier it gets one.

### Types

In `crates/core/src/forge.rs`, added to what is there:

```rust
pub struct PullRequest {
    // number, url, title, branch, state, draft, checks, details — as today
    pub author: Option<String>,   // login
    pub created_at: Option<i64>,  // now sent to the window: a row shows its age
}

pub struct PullRequestDetails {
    // base, head_oid, additions, deletions, review, updated_at, checks, mergeable — as today
    pub check_counts: CheckCounts,            // passed, failed, running
    pub review_requests: Vec<ReviewRequest>,  // a login or a team's slug, and which it is
    pub reviews: Vec<PullRequestReview>,      // login + approved | changes requested | …
    pub cross_repository: bool,               // its branch lives in a fork
}
```

A check's link (`PullRequestCheck::url`) waits for slice 2, which is what shows it.

`CheckCounts` is the one place a check's state is classified — a cancelled check is a failure, a
skipped one passes — and `Checks`, the verdict a badge shows, is worked out from it. One test
feeds the same checks through both shapes, listed and counted.

`ProjectPullRequests` (`src-tauri/src/publish/mod.rs`) gains `repo: Option<Repo>`,
`viewer: Option<String>`, `open_total: Option<u32>` and `open_problem: Option<String>` — why
the open ones could not all be read, kept apart from `problem` so the pull request dialog does
not start complaining about a list it never needed.

## The view

```
┌────────────────────┬─────────────────────────────────────────────────────────────────┐
│ ⇅ Pull requests 12 │ Pull requests      All  [Open]  Merged  Closed                ⟳ │
│ WORKFLOWS        + │ Project ▾   Author ▾   Reviews ▾   [ search title or number… ] │
│   Request code re… │─────────────────────────────────┬───────────────────────────────│
│ PROJECTS       ⌕ + │ yardsort  #91                Open │ #91  Add the pull requests… │
│ ▾ yardsort       + │ Add the pull requests view        │ joao · ys/pulls → main  Open │
│    local           │ joao · ✓ 12/12 · 2d · +340 −12   │ [Go to workspace] [Merge ▾]  │
│    like-now-add… ● │   ↳ like-now-add-pull             │ [Close] [Open on GitHub ↗]   │
│                    │ marvinapp  #212             Draft │──────────────────────────────│
│ ⚙ Settings  Usage  │ Fix login redirect                │  Summary   Code              │
│                    │ maria · ● 3/9 · 5h · +18 −4      │  …                           │
└────────────────────┴─────────────────────────────────┴───────────────────────────────┘
```

**The sidebar row** sits above the Workflows section inside the same `<aside>`. It carries the
number of open pull requests Yardsort knows of across all projects — with a **+** when a
repository has more open than the list holds, and nothing at all without `gh`, as everywhere
else. `aria-current="page"` while the view is open.
Pressing it again closes the view and returns to the selected workspace, as Usage does.

**The center panel** shows the view while `pullRequestsOpen` is set in the projects store,
beside `usageOpen` and `workflowId`: selecting a workspace, composing, opening a workflow or
Usage each close it, and it closes them. The right panel is left alone, as it is for Usage and
workflows.

**List and detail** are two panes of a `react-resizable-panels` group inside the view, with
their own remembered sizes. With no pull request selected the list has the whole width. The
details have a button that hides the list and one that closes them. Below 720 px of view width
they stack, list on top.

### A row

Project name (the `owner/name` in its tooltip), **#number**, title, the state pill in the
colours the workspace badge already uses (`pull-requests/appearance.ts`), then one quiet line:
author, checks as _passed/total_ in their colour, age since it was opened, **+added −removed**,
and **⚠** when the forge says it conflicts. A pull request with a workspace has a second line
naming it; pressing that goes to the workspace.

Rows are most recently updated first. There are no sort controls in the first release.

Up and Down move between rows, Enter opens the detail, Escape closes the detail and then
returns focus to the list. These are plain keys inside a list that has focus, not app
shortcuts; nothing here takes a key from a terminal.

### Filters

| Filter      | Choices                                                                                                                              |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| **State**   | All · Open · Merged · Closed. Starts on Open. A draft is open.                                                                       |
| **Project** | Any number of projects. None chosen means all.                                                                                       |
| **Author**  | One author out of those in the loaded rows, with **Me** first when `gh` said who you are.                                            |
| **Reviews** | Any · No reviews · Review required · Approved · Changes requested · Reviewed by you · Not reviewed by you · Awaiting review from you |
| **Search**  | Case-insensitive, over title, number and branch.                                                                                     |

- _Approved_, _Changes requested_ and _Review required_ are the forge's own `reviewDecision`.
  A repository with no review rule has none, and then the latest reviews decide: any change
  request outranks any approval.
- _Awaiting review from you_ is your login among the review requests. _Reviewed by you_ is your
  login among the latest reviews; _Not reviewed by you_ is the rest, without your own.
- The three _you_ filters are disabled, saying why, until `gh` has said who you are.
- Search and every filter work on **the rows that are loaded**. That is why the view says when
  there are more than it has, rather than letting a search fail quietly.
- State, project, author and reviews are remembered in `ui_state` (`pullRequests.filters`),
  through `remember`/`recall` like the diff mode. Search is not. A remembered project that is no
  longer open is dropped. **Clear filters** appears whenever one is set, and in the empty state.

Filtering is one pure function, `visible(rows, filters, viewer)`, in
`src/features/pull-requests/filters.ts`, with tests of its own. The frontend derives; it holds
nothing: the rows are the publish store's `byProject`, selected whole and filtered outside the
selector.

### What it says when there is nothing to show

Per project, as a line above the rows, never a dialog and never instead of other projects' rows:

- **`gh` is not installed** — one line with the link, once for the whole view.
- **Logged out** — _Run `gh auth login`_, the fix the pull request dialog already names.
- **Not on GitHub** — _marvinapp is on GitLab. Pull requests are listed for GitHub only._
- **No remote** — the project is named as having nowhere to ask.
- **`gh` failed** — what it said, and **Retry**.
- **Nothing matches** — _No pull requests match_, with **Clear filters**.

## The detail

A header — number, title, state, author, `head → base`, age — the actions, then two tabs. Until
slice 3 gives the second tab something to show, the Summary is simply what is under the actions.

### Summary (slice 2)

`pull_request_summary(project_id, number, refresh)` runs `gh pr view <n> --json` for the body,
comments, reviews, review requests, latest reviews, the count of changed files and every check,
reused for 30 s.

- **Description**, as Markdown. `react-markdown` with `remark-gfm` is a new dependency: there is
  no Markdown renderer in the app today. Raw HTML in a description is not rendered. A link opens
  in the browser through the opener and never navigates the window. **Images are not loaded** —
  the content policy forbids remote images, and fetching one would tell a stranger's server when
  you read their pull request — so an image is shown as a link with its alt text.
- **Checks**: a header that sums them up — _All 12 passed_, _2 of 12 failed_, _3 of 12 still
  running_ — then each check with its workflow, its state and a link to its run. A cancelled
  check is a failure, as it is on a row.
- **Reviewers**: who was asked and has not answered, and each reviewer's latest verdict.
- **Conversation**: comments and review summaries, oldest first, as Markdown under the same
  rules. Read-only until slice 4. Comments on lines are not part of what `gh pr view` returns;
  they arrive with slice 4 and until then the Summary links to the forge for them.

### Code (slice 3)

The pull request's diff, without checking anything out and without a worktree.

The commits come through git, into refs of Yardsort's own in the project's repository:

```sh
git fetch <remote> +refs/pull/<n>/head:refs/yardsort/pull/<n>/head <baseRefOid>
git merge-base <baseRefOid> <headRefOid>     # what the diff is measured from
```

Tried against a public repository on 2026-10-02: the ref lands on exactly `headRefOid`, no
branch or remote-tracking ref is touched, and a pull request merged long ago still gives its
diff back, because the base is the commit the forge recorded and not the branch as it is now.
It works for a fork's pull request too — the forge publishes `refs/pull/<n>/head` on the base
repository.

From there it is the changes panel's own machinery pointed at two commits instead of a working
tree: `Changes` learns to list and read between two revisions (`diff --name-status -M`,
`--numstat`, `at_revision` on both sides), which gives the same `FileChange` and `FileDiff` the
panel uses. So the file list with per-file counts and its renamed and modified marks, the
syntax-highlighted `CodeView`, the collapsed unchanged regions and the inline / side-by-side
switch (the same remembered `changes.diffMode`) are the components that exist, not new ones. A
**Files** dropdown jumps between files.

The fetch uses git's credentials, with prompts off, like a push. When it fails the tab says what
git said and offers the diff on the forge. The fetch happens when the tab is first opened for a
pull request and again when its head moves. Refs for pull requests that are no longer listed are
deleted on the next full load; they are Yardsort's, under a namespace nothing else writes to.

## Actions

| Action               | When                                   | What it does                                         |
| -------------------- | -------------------------------------- | ---------------------------------------------------- |
| **Go to workspace**  | a workspace has its branch checked out | selects that workspace                               |
| **Start workspace**  | open, and no workspace has it          | prepares the branch, opens the composer on it        |
| **Merge ▾**          | open, not a draft, not conflicting     | squash, merge commit or rebase, after a confirmation |
| **Close**            | open                                   | `gh pr close <n>`, after a confirmation              |
| **Reopen**           | closed, not merged                     | `gh pr reopen <n>`, after a confirmation             |
| **Open on GitHub ↗** | always                                 | the browser. **Copy link** beside it                 |

A disabled action says why in its tooltip: _It is a draft_, _It conflicts with main_.

**Merge** is the toolbar's merge with the workspace taken out. The confirmation names the
number, the title, the author and both branches; the core then asks the forge again and merges
only if it is still open, not a draft, and at the head commit that was confirmed
(`--match-head-commit`). Never `--admin`, never `--auto`, never `--delete-branch`.

The toolbar's merge refuses a pull request the workspace did not open, because from a workspace
that would be acting on someone else's work by accident. Here it is the point of the view, so
the rule is different and the confirmation carries it: when the author is not you, it says so
in so many words.

**Close** never deletes a branch and posts no comment. **Reopen** is confirmed for what it sets
off, not for what it risks. A _no_ to any of the three sends nothing, and there is a test for
each.

The forge decides who may do what. A refusal — no permission, a required check, a merge queue —
is shown beside the header in the forge's own words, and the list is asked again.

### Start workspace

The one action that touches the repository. What it does depends on where the branch lives:

1. **A workspace or `local` has the branch checked out** — there is nothing to start; the button
   is **Go to workspace**.
2. **The branch exists locally and is not checked out** — it is opened as it is. It is never
   reset to the pull request's head: if it is behind, the composer says by how many commits.
3. **Same repository, no local branch** — the branch is fetched into its remote-tracking ref
   (`+refs/heads/<branch>:refs/remotes/<remote>/<branch>`) and `git branch --track` makes the
   local one.
4. **From a fork** — a local branch `pr/<n>` at `refs/pull/<n>/head`.

Then the composer opens for that project, on **Open existing branch** with the branch chosen,
through a `composingBranch` beside the `composingPrompt` the store already has. The agent, the
message and the button are the composer's, and so is everything that follows —
`Workspaces::open_branch`, the project's setup script. Cancelling the composer leaves the branch
where it is; a branch is cheap and removing one is not this view's to do.

A workspace started this way shows the pull request on its row by the rule that exists: its
branch is the pull request's. A fork's does not have that — `pr/<n>` is not the name on the
forge — and pushing from it would create `pr/<n>` on the project's own remote rather than update
the fork. So the branch's upstream is set to `refs/pull/<n>/head`, which is where the tie is read
back from, and Yardsort does not push it: [open question 25](06-open-questions.md), settled in
slice 1.

## Writing to the forge (slice 4)

- **Reply**: a box under the conversation. `gh pr comment <n> --body-file -`, the text on
  standard input — never in the argument list, which Windows caps and a shell could mangle.
  `Ctrl+Enter` / `⌘Enter` sends. Sending is the confirmation; nothing is posted any other way.
- **Comments on lines**, read with `gh api repos/{owner}/{repo}/pulls/{n}/comments` and shown
  beside the lines they are on in **Code**.
- **Send to an agent**: select lines in a diff, write what you want done, send. The agent gets
  the file, the line range, which side of the diff, the lines themselves and the pull request.
  Where it goes follows the rules the conflict helper set (`publish/conflicts.rs`): typed into
  a running, quiet agent of the linked workspace; a resumed or new session otherwise; **never
  into a busy one**. With no workspace, it is **Start workspace** with the message as the
  composer's first message. **Also post it on GitHub** is a box, off by default; ticked, the
  same text becomes a comment on those lines. This is the roadmap's "diff comments sent back to
  the agent as a prompt", with the diff being a pull request's.

## Commands

| Command                                                                              | Slice | Notes                                                 |
| ------------------------------------------------------------------------------------ | ----- | ----------------------------------------------------- |
| `project_pull_requests(project_id, refresh, full)`                                   | 1     | exists; `full` asks for the open tier too             |
| `pull_request_prepare_branch(project_id, number)`                                    | 1     | the branch to open, and how far behind it is          |
| `pull_request_merge(project_id, number, head_oid, method)`                           | 1     | shares its checks with `workspace_merge_pull_request` |
| `pull_request_close(project_id, number)` · `pull_request_reopen`                     | 1     | forget the project's cache afterwards                 |
| `pull_request_summary(project_id, number, refresh)`                                  | 2     | body, checks with links, reviewers, conversation      |
| `pull_request_changes(project_id, number)`                                           | 3     | fetches, then lists the changed files                 |
| `pull_request_diff(project_id, number, path, old_path)`                              | 3     | both sides of one file                                |
| `pull_request_comment` · `pull_request_line_comments` · `pull_request_send_to_agent` | 4     |                                                       |

All in `src-tauri/src/publish/`, in a new `pull_requests.rs` beside `conflicts.rs`; the `gh` and
git calls in `yardsort-core`. Every program is spawned with an argument array. `bindings.ts` is
regenerated, never edited.

## Tests

- **`gh`'s answers**: recorded JSON under `crates/core/fixtures/gh/2.102.0/` — a list page, a
  last page, a detail — parsed in unit tests, like the agents' fixtures. Check counts against
  `roll_up` on the same checks.
- **Paging and failure**: a stand-in `gh` in a temp directory, as `forge.rs` already does for
  the time limit: three pages read in order; the second page failing keeps the first; a page
  that never answers is stopped; the cap stops at 200 and reports the total.
- **The two tiers**: merging, the newer answer winning, a vanished pull request dropped, check
  names kept only while the head is the same.
- **Git, for real**: a bare repository in a temp directory with a `refs/pull/1/head` made by
  hand. Fetching into the private ref; the diff of an open and of a merged pull request;
  **Start workspace** in each of its four cases, including that an existing local branch is
  _not_ moved.
- **Merge, close, reopen**: the argument arrays `gh` is given; a head that moved refuses; a
  draft refuses.
- **The view**, through Testing Library: every filter, alone and together; the remembered
  filters coming back; a remembered project that is gone; each empty state; **a _no_ to Merge,
  Close and Reopen calls nothing**; the row count in the sidebar; opening and closing against
  workspaces, workflows and Usage.
- **By hand**, on all three systems: a new [08 §23](08-manual-checklist.md).

## Slices

Each is one pull request with its docs, tests and changelog line, in this order.

1. **The list, the filters and the actions.** ✅ See [slice 1](#slice-1-what-shipped).
   1. Core: the new fields on `PullRequest`; `Gh::open_pull_requests` over `gh api graphql`
      with paging and the cap; `gh pr close` and `reopen`; fixtures.
   2. `Forge`: the open tier beside the recent one, the merge of the two, `full`.
   3. Git: fetching a branch or a pull request's head; `pull_request_prepare_branch`.
   4. Commands: merge, close, reopen, with the toolbar's merge sharing the check.
   5. Store: `pullRequestsOpen`, `composingBranch`; the sidebar's poll passes `full` while the
      view is open; a `pullRequests` command with no default key.
   6. UI: the sidebar row; the view, the list, the filters; a detail pane made of the header,
      the actions and the `PullRequestDetails` card that exists; the three confirmations.
   7. Docs: a new `docs/guide/pull-requests.md`, the rest of
      [what this touches](#documentation-this-touches), and a screenshot.
2. **Summary.** ✅ `pull_request_summary`; the Markdown renderer and its rules; checks with links,
   reviewers, the conversation, read-only. See [slice 2](#slice-2-what-shipped).
3. **Code.** ✅ `Changes` between two revisions; the private refs and their cleanup; the file
   list, the viewer and the Files dropdown in the detail pane. See
   [slice 3](#slice-3-what-shipped).
4. **Writing.** The reply box; comments on lines; send to an agent, with or without a workspace.

Slice 1 is the large one. If it grows past what one review can hold, steps 3–4 and the action
buttons split off as their own pull request, leaving a list that only reads and links out.

## Slice 1: what shipped

Everything in the slice's seven steps, with these differences from the proposal above, each
already folded into the text it changes:

- **The repository is `gh`'s to name.** The open tier's query takes `{owner}` and `{repo}` from
  `gh`'s own placeholders rather than from the remote Yardsort parses, so both tiers ask about
  the same repository, also in a clone of a fork. The parsed remote still decides two things:
  whether to ask at all (not for GitLab or Bitbucket) and `--hostname` for a GitHub elsewhere.
- **The recent tier wins.** "The newer answer wins" needed both tiers' clocks compared per pull
  request to say what one rule says outright: the recent tier is asked at least as often.
- **Forgetting a project keeps the open tier**, marked out of date. Dropping it would have
  taken the older open pull requests off every workspace row until the view was next opened.
- **A read that gets nothing keeps what was known**, with the reason beside it. Half a list
  replaces the old one; no list does not.
- **A fork's pull request** is settled as open question 25 leaned: `pr/<n>`, upstream
  `refs/pull/<n>/head`, no push from Yardsort — refused in the core, not only hidden in the
  window. `PublishState::follows_pull_request` carries it to the changes panel.
- **The remote to fetch from** is the one whose URL names the pull request's repository, not
  always `origin`.
- **A branch name from the forge is untrusted.** Whoever opened the pull request chose it;
  `git check-ref-format --branch` sees it before any other command does, and a name with a
  leading dash goes nowhere.
- **Escape closes the details from the list as well as from inside them**, since pressing a row
  leaves the focus on the row.
- **Hiding the list collapses its panel.** Taking the panel out of the tree while the details
  stayed tripped an assertion in `react-resizable-panels`; collapsing is what the shell's own
  side panels do. The rows are `hidden` while collapsed, so they take no focus.
- **`Review` was taken** — Assist has a type by that name and the bindings are one namespace —
  so a reviewer's verdict is `PullRequestReview`.

How it was checked: the parsers against JSON recorded from `gh` 2.102.0, names replaced
(`crates/core/fixtures/gh/`); paging, a failing page, a silent page and the cap against a
stand-in `gh`; the branch preparation, in all its cases, against real repositories with a
`refs/pull/7/head` made the way a forge makes one; the view through Testing Library, including
a _no_ to each confirmation. Two things no test here can do were done by hand: the real `gh`
against this repository and against `cli/cli` (two pages, 73 of 73), and the view in headless
Chromium with demo data — list, details, hide and show, close, tabs, and the stacked layout
below 720 px — because jsdom lays nothing out.

**Not done: the screenshot.** It wants a real window, and the stand-in `gh` in
`scripts/screenshots.sh` still answers only `gh pr list` with one pull request and no author. It
needs to answer `gh api graphql` too, and the demo repositories need a GitHub remote, before the
view has anything worth a picture. The hands-on pass, [08 §23](08-manual-checklist.md), is also
still to be done on all three systems.

## Slice 2: what shipped

The Summary: description, checks with links, reviewers, conversation. Read-only.

- **`Gh::pull_request_summary`** asks `gh pr view <n> --json` for the list's own fields plus
  `body,comments,reviews,changedFiles`, with a time limit. It returns the pull request as the
  forge has it now, so the Summary's checks and reviewers are fresher than the row's.
  `PullRequestCheck` gained `workflow` and `url` (`detailsUrl`, or `targetUrl` for a commit
  status), which every reader of the list now gets too.
- **`Forge::summary`** keeps each one for 30 s by project and number. Unlike the lists it is an
  `Err` when `gh` cannot answer: the Summary has nothing else to show there and says why. A
  summary read while the project was being forgotten — a merge landing mid-read — is returned
  but not kept.
- **The window follows the list.** The store keeps one summary per row with a _stamp_ of what
  the list said when it was asked for: state, head commit, `updatedAt` and the check counts. The
  list is polled every minute while the view is open, and when a row's stamp moves the summary
  is read again, past the core's cache. A check finishing moves nothing on a pull request but
  the counts, which is why they are in the stamp. The old summary stays on screen until the new
  one arrives, and an answer overtaken by a later question is dropped. After Merge, Close or
  Reopen it is read again whatever the list says.
- **Markdown** is `react-markdown` with `remark-gfm`, loaded when the first pull request is
  opened. `skipHtml` drops raw HTML altogether rather than printing it. Only `http(s):` and
  `mailto:` are links, opened through the opener with the click and the middle click both
  prevented, so the webview never navigates. An image is a link labelled with its alt text.
  Task-list boxes are disabled. Each of these has a test.
- **A comment the forge hid stays hidden.** `isMinimized` comments keep their place and their
  author and lose their words in the core, not only in the window.
- **A review with no words** is shown for what it was: _approved_, _requested changes_, or, for a
  bare `COMMENTED`, _left comments on lines of the code_ — those comments being the one part of
  a conversation `gh pr view` does not return. The foot of the conversation says so and links
  to the forge. They arrive with slice 4.
- **No tabs yet.** One tab is not a choice. The Summary sits under the actions, and becomes the
  first of two when slice 3 brings Code.
- **Named `pull_request_summary`**, not `pull_request_detail`: there is already a
  `PullRequestDetails`, and one letter is not enough to tell two types apart.

How it was checked: the parser against a recorded `gh pr view` answer, names and words replaced;
the cache and the stamp with unit tests; the Summary and the Markdown renderer through Testing
Library; and the pane in headless Chromium with demo data, where the description rendered with
its table, task list and code, no image element existed, and the template comment was gone.

Still not done: the screenshot, and the hands-on pass ([08 §23](08-manual-checklist.md)), which
has rows for the Summary now.

## Slice 3: what shipped

Code: the pull request's diff, with nothing checked out. And the tabs, now that there are two.

- **`changes::Between`** in the core lists what differs between two commits and reads both
  sides of a file, with the parsers and the per-revision reader `Changes` already had. Same
  `FileChange`, same `FileDiff`.
- **`fetch_for_diff`** makes sure the commits are in the project's repository and says which two
  the diff is between. It fetches `refs/pull/<n>/head` into `refs/yardsort/pull/<n>/head` and
  the base commit by its id in the same round trip, pins the base under
  `refs/yardsort/pull/<n>/base` so git does not collect it, and answers with the head _as
  fetched_ and its merge base with the base. When both are here already it fetches nothing.
- **The base is the commit the forge recorded** (`baseRefOid`, which the summary's question
  gained), not the base branch as it is now. For a pull request merged with a merge commit the
  branch's tip contains the head, and a diff against it is empty. There is a test for exactly
  that.
- **A server that will not give a commit by its id** gets a second try: the head alone, then the
  base branch by name. Its tip stands in for the recorded commit — the same thing for an open
  pull request.
- **Ids from the forge and from the window are checked to be ids** (`git::is_commit_id`) before
  git sees them as arguments, and the base branch's name goes through `check-ref-format`.
- **`prune_refs`** deletes the refs kept for pull requests the list no longer has, on each full
  read of the list — and only when that list is whole: `gh` answered, for both tiers.
- **Two commands.** `pull_request_changes(project_id, number)` fetches and lists;
  `pull_request_diff(project_id, base_oid, head_oid, path, old_path)` reads one file between
  the two commits the first one named. The window hands them back rather than a number, so a
  file is always read from the same pair the list was.
- **The window**: `DiffBody` was lifted out of the changes panel's viewer so both show a diff
  the same way — inline or two panes, images side by side, a line for what cannot be shown —
  and the remembered `changes.diffMode` is shared. The tab opens on the list of files and a
  file opens in the viewer, with a Files box and previous/next. It is not every file in one
  long scroll: each would be its own CodeMirror.
- **Fetched once per head.** The store keeps the list of files with the head commit the pull
  request list named when it was asked for. A comment does not move that; a push does.
- **The tabs** are Summary and Code, and the one in view is kept from one pull request to the
  next until you quit. Code is only fetched when it is looked at.
- Found on the way: the test fixture from slice 1 made its "pull request" branch in a clone that
  had checked nothing out, so it shared no history with the base. Slice 1's tests never
  compared the two. It grows from the base now.

How it was checked: `Between` and the fetch against real repositories, including a merged pull
request, a head that moved on, the fallback, and a second look with the remote gone; the tab
through Testing Library with the viewer stubbed; the real viewer in headless Chromium; and
`changes_of` against a real, long-merged pull request from a fork on GitHub, fetched into an
empty repository.

Still not done: the screenshot and the hands-on pass, as before.

## Not in this version, on purpose

- Other forges. GitLab's `glab` is the same idea and a second implementation of all of it.
- _Awaiting review from you or your team_. `gh` has the permission for it by default
  (`read:org`); what it lacks is the question — your teams in each organisation, asked once and
  kept. A small follow-up once the other review filters have been used for a while. A token
  without `read:org`, and a repository owned by a person rather than an organisation, would
  leave the filter disabled with the reason.
- Approving or requesting changes from the app; editing a title or description; labels,
  assignees, milestones; draft ↔ ready.
- Deleting branches after a merge or a close. Yardsort keeps branches.
- Searching the forge. Search is over what is loaded, and says when there is more.
- Sort controls; saved filter sets; a pull request opened in a window of its own.
- Issues.

## Open questions

[24–27 in open questions](06-open-questions.md#pull-requests): whether the right panel should
get out of the way while the view is open; how a workspace started from a fork's pull request is
tied to it and whether to push to forks (settled in slice 1); whether the recent tier should
become the lighter query too; whether descriptions may ever load images.

## Documentation this touches

`docs/guide/pull-requests.md` (new), `docs/guide/commits-and-pull-requests.md` (how often the
forge is asked; "what it does not do" loses review comments), `docs/guide/workspaces.md`
(starting one from a pull request), `docs/guide/shortcuts.md`, `docs/guide/troubleshooting.md`
(`gh` errors a user can now meet), `docs/README.md`, `website/src/lib/docs.ts`, `README.md` and
the landing page's copy, `CHANGELOG.md`, [02-ux](02-ux.md) (a row above Workflows),
[03-architecture](03-architecture.md) (the two tiers; Yardsort's own refs),
[05-roadmap](05-roadmap.md) (M23), [06-open-questions](06-open-questions.md) (24–27),
[08](08-manual-checklist.md) (§23).

Screenshots need care this feature has not needed before: a pull request list shows **logins**,
and the throwaway profile does not change who `gh` is. They are taken with a stand-in `gh` on
`PATH` that prints the recorded fixtures — the same trick as the agent shims in `AGENTS.md` —
so no real account, repository or person appears.
