# Pull requests

Every pull request of every project, in one place: who opened it, what CI and the reviewers
made of it, and a way to merge it, close it or start an agent on it. Open it with **Pull
requests** at the very top of the sidebar, or from the
[command palette](shortcuts.md#navigate-without-the-mouse) (**Mod+K**, then _Pull requests_). It
takes over the center panel the way a workflow and [Usage](usage.md) do; **×** in its header
closes it, and so does pressing the row again or selecting a workspace.

The number on the sidebar row is how many pull requests are open across your projects, with a
**+** after it when a repository has more open than the list reads.

It needs the [GitHub CLI](https://cli.github.com), installed and logged in, and it is for
projects on GitHub. Yardsort still holds no credential of its own: everything here is `gh`,
with the permissions you already have.

This is the view for pull requests you did _not_ open from a workspace — a teammate's, one from
another machine, one an agent opened last week. The pull request a workspace opened is also on
its own row and toolbar: see [Commits & pull requests](commits-and-pull-requests.md).

## The list

One row per pull request, most recently updated first, from every project whose repository is
on GitHub:

- the **project** and the **number**, with a pill that says **Open**, **Draft**, **Merged** or
  **Closed** in the colours a workspace's badge uses;
- the **title**;
- who **opened** it, the **checks** as passed out of all that reported (**✓ 12/12**, **● 9/12**
  while some are running, **✕ 10/12** when any failed — a cancelled check is a failure), how
  long ago it was opened, the lines **+added −removed**, and **⚠ conflicts** when GitHub says it
  cannot merge into its base as it is.

A pull request that has a workspace says which under its row; press that to go there.

**↑** and **↓** move between rows, **Home** and **End** jump to the ends, and **Enter** opens the
one in focus.

### What is in it

Every **open** pull request of each repository, up to the 200 most recently updated, and the 50
**newest** of any state. Past 200 open the view says so — _Showing the 200 most recently updated
of 1,394 open in api-gateway_ — with a link to the rest on GitHub. An older merged or closed
pull request is not listed; the **Merged** and **Closed** tabs show the recent ones.

## Filters

| Filter       | What it does                                                                                               |
| ------------ | ---------------------------------------------------------------------------------------------------------- |
| **State**    | The tabs in the header: **All**, **Open**, **Merged**, **Closed**. It starts on **Open**; a draft is open. |
| **Projects** | Any number of projects. None chosen means all of them.                                                     |
| **Author**   | One author, out of everyone in the list. **Me** is whoever `gh` is logged in as.                           |
| **Reviews**  | Where it stands with its reviewers — see below.                                                            |
| **Search**   | Title, branch or number. `#12` and `12` both find number 12.                                               |

The review filters:

- **No reviews** — nobody has reviewed it yet.
- **Review required**, **Approved**, **Changes requested** — GitHub's own decision. A repository
  with no review rule has none, and then the latest reviews decide: any request for changes
  outranks any approval.
- **Reviewed by you** and **Not reviewed by you** — by your latest review. Your own pull
  requests are not counted as ones you have not reviewed.
- **Awaiting review from you** — a review was asked of you, by name, and you have not given it.
  A request made of a team you are in is not counted.

The three about _you_ are greyed out until `gh` has said who you are.

State, projects, author and reviews are **remembered** across restarts. Search is not. **Clear
filters** appears whenever something is narrowing the list, and puts everything but the tab back.

Filters and search work on the pull requests that are loaded. That is why the view tells you when
GitHub has more open than it read, rather than letting a search come up empty.

## The details

Press a row and its details open beside the list. At the top: the project, who opened it and
when, and [what you can do](#what-you-can-do) with it. Under that, its **Summary**:

- **What it is** — number, state and review decision, the title, **branch → base**, the lines
  added and removed, whether it conflicts with its base, and when it last changed.
- **Description** — what its author wrote, shown as GitHub shows Markdown: headings, lists,
  tables, task lists, code.
- **Checks** — one line that sums them up (_All 12 passed_, _2 of 12 failed_, _3 of 12 still
  running_), then every check with its workflow and how it went, failures first. Press a check
  to open its run on GitHub. A cancelled check counts as failed.
- **Reviewers** — each reviewer's latest word, and everyone a review is still being asked of.
- **Conversation** — the comments and reviews, oldest first, each with who, what they did
  (_commented_, _approved_, _requested changes_) and when. **↗** opens that one on GitHub. A
  comment GitHub has hidden — as spam, off-topic, outdated — says so and stays hidden here too.

The top of the Summary comes from the list and is there at once; the rest is read when you open
the row, and again whenever the list notices the pull request changed — a push, a comment, a
review, a check finishing. If it cannot be read, the reason is shown with **Retry**, and what
the list knows stays on screen.

### What is shown of other people's words

A description or a comment is written by whoever opened or commented on the pull request, so it
is shown with care:

- **No images are loaded.** An image is a request to a server of the author's choosing, made the
  moment you read their words. It appears as a link — _[image: the new dialog] ↗_ — that opens
  in your browser if you choose to press it.
- **Links open in your browser**, never inside Yardsort. Only web and mail addresses are links.
- **HTML is not rendered**: a template's hidden comments disappear, and nothing a page could
  run is run.
- The boxes of a task list show what was ticked. They are not ticked from here.

### What is not in it

**Comments on particular lines of the code** are not shown yet. A review that has only such
comments reads _Left comments on lines of the code_, and the foot of the conversation links to
GitHub for them. You cannot reply from here yet either, and the diff itself is the next piece
being built; **Open on GitHub ↗** has all three.

### Moving around

**⇤** in the corner hides the list to give the details the whole panel, and **⇥** brings it
back; you can also drag the line between them. **×** or **Esc** closes the details and puts the
focus back on the row. In a narrow window the two stack, list on top.

## What you can do

| Button               | Offered when                       | What it does                                           |
| -------------------- | ---------------------------------- | ------------------------------------------------------ |
| **Go to workspace**  | a workspace already has it         | Selects that workspace.                                |
| **Start workspace**  | it is open and no workspace has it | Fetches its branch and opens the composer on it.       |
| **Merge ▾**          | it is open                         | Squash, merge commit or rebase — after a confirmation. |
| **Close**            | it is open                         | Closes it without merging — after a confirmation.      |
| **Reopen**           | it is closed, not merged           | Opens it again — after a confirmation.                 |
| **Open on GitHub ↗** | always                             | Opens it in your browser. **Copy link** is beside it.  |

Any pull request of a project can be acted on from here, not only your own, so **every
confirmation says whose it is** — _grace opened it, not you_ — along with its number, title and
branches. Cancelling does nothing at all.

GitHub decides what you may do. If it refuses — no permission, a required check, a branch
protection — its own words appear under the buttons, and the list is read again.

### Merge

**Merge ▾** offers **Squash and merge**, **Create a merge commit** and **Rebase and merge**. It
is greyed out, saying why, for a draft and for a pull request that conflicts with its base.

The confirmation shows the commit it will merge. Yardsort asks GitHub about the pull request
again and merges only if it is still open, still not a draft, and still at that commit: if
someone pushed in the meantime, nothing is merged and you are asked to look again. **No branch is
deleted**, and nothing is forced past a protection. On a branch that requires a merge queue,
`gh` queues it instead, and the view says the request was sent rather than that it merged.

### Close and reopen

**Close** closes the pull request without merging it. Its branch is kept and no comment is
posted. **Reopen** is confirmed too, for what it sets off rather than for any risk: reviewers
are notified, and checks may run again.

### Start a workspace from a pull request

**Start workspace** gets the pull request's branch into the project's repository and opens the
[composer](workspaces.md#starting-one-the-composer) with **Open existing branch** already on
it. Pick the agent, say what you want — _review this_, _fix the failing test_ — and start. Until
you press **Start** no worktree exists; cancelling leaves only the branch.

What it does to the repository depends on where the branch is:

- **You already have the branch locally** — it is opened exactly as it is. Yardsort never moves
  a branch of yours: if it is behind the pull request, the composer says by how many commits.
- **The branch is in the project's own remote** — it is fetched, and a local branch is made
  that tracks it, so a push from the workspace updates the pull request.
- **The pull request comes from a fork** — see below.
- **Something already has the branch checked out** that Yardsort does not know as a workspace —
  nothing happens, and the message says where. Git allows a branch in one place at a time.

#### From a fork

A fork's branch is not in your project's remote, so the workspace gets a local branch named
**`pr/<number>`** at the pull request's head, set to follow it:

- `git pull` in the workspace brings in what the author pushes.
- **Yardsort does not push it.** The changes panel shows no **Push** there, and says why: a
  push would create `pr/<number>` on your project's remote instead of updating the fork. A plain
  `git push` in a terminal is refused by git for the same reason.
- The pull request still shows on the workspace's row and toolbar, as it would for a branch of
  your own.

If you have push access to the fork and want to update the pull request, push by hand from a
shell tab to the fork's URL and branch.

## How often GitHub is asked

- **When Yardsort starts**, every open pull request is read once, so the count is right and the
  view opens with rows in it.
- **While the view is open**: when you open it, when you come back to the window, once a minute,
  on **Refresh**, and after anything you do to a pull request.
- **While it is closed**, only the newest fifty per project are read, as they always have been
  for the workspace badges — plus, one at a time, any older open pull request a workspace shows,
  so its badge does not stay as it was when Yardsort started.

The open pull requests are read fifty at a time. On a busy repository that is a few seconds a
page, which is why the whole list is only kept fresh while you are looking at it.

A pull request's Summary is one more question, about a second, asked when you open it. The answer
is reused for half a minute, and asked for again when the list says something about the pull
request changed.

## When something is missing

The view says why, on a line above the rows, without hiding the projects that are fine:

- **`gh` is not installed** — install the [GitHub CLI](https://cli.github.com) and run
  `gh auth login`.
- **Nobody is logged in** — run `gh auth login` in a terminal, then **Refresh**.
- **A project is on GitLab or Bitbucket** — pull requests are listed for GitHub only. Opening a
  merge request from a workspace [still works](commits-and-pull-requests.md#without-it).
- **A project has no remote on a forge** — there is nowhere to ask.
- **`gh` failed** — what it said, with **Retry**. `HTTP 502` and `HTTP 504` are GitHub taking
  too long, which a busy repository does now and then.
- **Not every open pull request could be read** — the ones that arrived are shown, with what
  went wrong and **Retry**.
- **One pull request could not be read** — its Summary says why, with **Retry**. The row and
  everything the list knows about it are still there.

## What it does not do

- Show the diff, or comments made on particular lines of it — those are being built next.
- Post a comment or a reply.
- Approve or request changes, edit a title, or change labels, assignees or reviewers.
- Delete a branch, after a merge or ever.
- List pull requests for GitLab, Bitbucket or Gitea.
- Search GitHub: search is over what is loaded.
