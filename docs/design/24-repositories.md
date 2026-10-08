# Repositories

_Proposed 2026-10-08, from [issue #91](https://github.com/joaoh82/yardsort/issues/91). Slices 1
and 3 are built — see [slice 1](#slice-1-what-shipped) and [slice 3](#slice-3-what-shipped),
which also say where they differ from the proposal below. Slice 2 is not._

The issue asks for the forge to be the front door: a new piece of work starts from a repository,
an issue or a pull request, not from a folder on disk. Most of that arrived the day after the
issue was opened — **Delegate** in [Tasks](23-tasks.md) and **Start workspace** in
[Pull requests](22-pull-requests.md) both start an agent from a GitHub issue or pull request —
but only for a project that is already added, and adding one still means typing a clone URL
and choosing a folder. This doc closes that gap in three slices: **pick a repository from a
list** instead of typing its URL, **clone from any git host** and not only GitHub, and **start
from a pasted link** to an issue or pull request, cloning first when the repository is not a
project yet.

What does not change: a project is a local clone, and every workspace is a worktree of it. That
is the "git-native, no lock-in" principle of [01](01-vision.md), and nothing here asks the user
to think about it less than they do now — only to type less. Everything goes through `gh`, as
pull requests and tasks do; Yardsort still holds no forge credential.

## Decisions

Settled with the user on 2026-10-08, after the issue was reviewed against what exists:

1. **Three slices, built 1, 3, 2.** The list and the any-host clone are small and close the
   visible gap; starting from a link mostly routes to what Tasks and Pull requests already do,
   and comes once the first two have been used.
2. **The project stays a local clone.** The issue's model — a repository or an issue as the
   scope, with no folder in sight — is served by never asking for the folder twice, not by
   taking the clone away. Remote and cloud workspaces remain out of scope ([01](01-vision.md)).
3. **Through `gh`, like the rest.** No token, no setting. Without `gh`, or logged out, the
   clone step is today's URL field with one line saying why there is no list.
4. **Other forges' lists stay deferred.** Cloning from GitLab, Forgejo, Gitea or Bitbucket is
   slice 3; their issues and pull requests are a second `TaskSource` and a second pull request
   implementation each, behind [open question 31](06-open-questions.md), and wait for demand.
5. **The reporter is answered** on the issue with what exists already and what this doc plans,
   by the user, from a draft.

## What was measured

Against this account on 2026-10-08 with `gh` 2.102.0. The account owns or collaborates on 230
repositories and is a member of 16 organisations, one of them large:

| Query                                                                                  | Result |
| -------------------------------------------------------------------------------------- | ------ |
| `gh repo list --limit 100` (owner only)                                                | 1.9 s  |
| `viewer.repositories`, owner + collaborator, 100 to the page, through `gh api graphql` | 1.9 s  |
| the same, 100 to the page, owner + collaborator + organization member                  | 7.7 s  |
| the same, 50 to the page                                                               | 5.0 s  |
| `user/repos?affiliation=owner,collaborator&per_page=100` through `gh api`              | 1.1 s  |
| `user/repos?affiliation=owner,collaborator,organization_member&per_page=100`           | 3.5 s  |
| `search/repositories?q=yard in:name`, 20 rows                                          | 3.6 s  |
| `gh search repos yardsort --limit 20`                                                  | 2.4 s  |

What it says:

- **Your own and your collaborations are a list; your organisations' repositories are not.**
  The two first affiliations total 230 here and a page comes in under 2 s. Adding organisation
  membership makes the total 64,812, a page takes 5–8 s, and no cap makes sense of it. So the
  list is **owner + collaborator**, most recently pushed first, up to 200 in pages of 100 —
  the cap tasks use, and the same _showing N of M_ line when there are more — and anything
  else is reached by **search**, which asks the forge and costs 2–4 s per query.
- **GraphQL over REST, by a small margin.** REST is faster for the page, but one GraphQL query
  gives in one answer what the dialog wants and REST would need a second call for: the fork's
  parent with both its URLs, whether issues are on, the language, and the total count. The
  shape also matches the tasks and pull requests queries, with the same paging.
- **Both clone URLs come back**, HTTPS and SSH, so the choice between them is
  `gh config get git_protocol`, the setting the user already made for `gh` itself, and no
  question is asked.

## The dialog

The clone step of **Add a project** becomes one field with a list under it:

```text
┌ Clone a repository ─────────────────────────────────────────────────┐
│ Repository                                                           │
│ [ yard…                                                            ] │
│ ┌──────────────────────────────────────────────────────────────────┐ │
│ │ ● joaoh82/yardsort          Rust · pushed 2 h ago    Already added│ │
│ │   joaoh82/yardsort-site     TypeScript · pushed 3 d ago           │ │
│ │   acme/yard-tools   (fork of yardtools/yard)  · private · 2 w ago │ │
│ │   Search GitHub for “yard”…                                       │ │
│ └──────────────────────────────────────────────────────────────────┘ │
│ Name       [ yardsort-site                                         ] │
│ Location   ~/Projects                                       Change…  │
│            ~/Projects/yardsort-site                                  │
│                                                  ← Back  Clone project│
└──────────────────────────────────────────────────────────────────────┘
```

- **The field** takes anything the old one did — an HTTPS or SSH URL, `owner/repository` —
  and also filters the list as you type, on the name with its owner and on the description.
  Pasting a URL still works with no list at all: the first row is then _Clone <url>_.
- **A row** shows the repository with its owner, the language, when it was last pushed to,
  _private_, _archived_, and _fork of …_ with the parent. **↑**, **↓** and **Enter** work as
  in every list here; choosing a row fills the field and the name.
- **Already added** marks a repository that is one of your projects, matched on host, owner
  and name through `forge::repo_at` against each project's push remote. Choosing it selects
  the project and closes the dialog; nothing is cloned twice. A project that was removed with
  its history, whose folder is still there, comes back the way **Open a folder** brings it back.
- **Search GitHub for “…”** is the last row once three characters are typed. It is a row and
  not a debounce: pressing it is the one thing that sends a query, so no keystroke costs 3 s
  on its own. Results take the list's place with a line saying they are the forge's, up to 20,
  and _Your repositories_ brings the list back.
- **Name** follows the chosen repository until edited by hand, as it follows the URL today.
- **Location** is the remembered one, shown as a line with **Change…**, so the usual path is
  pick, then **Clone project**. The first clone ever, with nothing remembered, shows the field
  and **Browse…** as today; nothing is invented for it.
- **While it loads**, the field accepts a URL immediately and the list says _Reading your
  repositories…_. Without `gh`, logged out, or on a network that is away, the list is one line
  in the words the pull requests view uses, and the field is all there is.
- **Forks**: a clone of a fork gets its parent as `upstream`, added and fetched, which is where
  [22](22-pull-requests.md) already expects it — `gh` answers for the parent in a clone of a
  fork, and its branches are what a pull request from the fork is measured against.

The dialog grows from 28 to 36 rem for the list; the other two choices are unchanged.

## Where the data comes from

`gh api graphql`, with no repository to resolve, so from any folder:

```text
query ($after: String) {
  viewer {
    login
    repositories(first: 100, after: $after,
                 affiliations: [OWNER, COLLABORATOR],
                 ownerAffiliations: [OWNER, COLLABORATOR],
                 orderBy: { field: PUSHED_AT, direction: DESC }) {
      totalCount
      pageInfo { hasNextPage endCursor }
      nodes {
        nameWithOwner description url sshUrl
        isPrivate isFork isArchived hasIssuesEnabled pushedAt
        primaryLanguage { name }
        parent { nameWithOwner url sshUrl }
      }
    }
  }
}
```

- **The list**: two pages at most, each within its own time limit (`Gh::run_within`), kept for
  the life of the dialog and read again when it opens. A second page that fails keeps the first
  and says so. Nothing is read while the dialog is closed; adding a project is rare and the
  list is not a view.
- **Search**: `search/repositories?q=<text> in:name&per_page=20` through `gh api`, with the
  text sent as a query parameter and never spliced into a command line. The same fields come
  back in REST's names and are mapped to the same type. A private repository the account can
  reach is found; one it cannot is not, and that is the forge's rule, not ours.
- **The protocol**: `gh config get git_protocol`, once per dialog; `ssh` picks `sshUrl`,
  anything else `url`. A pasted URL is cloned as pasted.
- **Which host**: github.com in this version. A GitHub Enterprise host is the pull requests
  view's `--hostname` story again and is [open question 34](06-open-questions.md).

```ts
type RemoteRepository = {
  nameWithOwner: string; // "joaoh82/yardsort"
  description: string | null;
  cloneUrl: string; // HTTPS or SSH, by gh's git_protocol
  isPrivate: boolean;
  isFork: boolean;
  isArchived: boolean;
  parent: { nameWithOwner: string; cloneUrl: string } | null;
  language: string | null;
  pushedAt: string | null; // as the forge wrote it
  projectId: string | null; // already one of your projects
};
```

## Cloning from any host

Slice 3 relaxes `github_url` in `crates/core/src/projects/mod.rs` into a clone-URL validator
for any host:

- `https://host/path/to/repo(.git)`, `ssh://git@host/path/to/repo(.git)` and git's scp-like
  `git@host:path/to/repo(.git)` are accepted, and `owner/repository` on its own still means
  GitHub, so the shorthand keeps working.
- A path may have more than two parts — GitLab subgroups — and `forge::parse_remote` already
  reads such a remote back.
- Still refused: anything that is not one of those shapes, so no local path, no `git://`, no
  `file://`, nothing starting with `-`. The URL goes to git after `--`, as it does today.
- The name is the last path component without `.git`, as today.

Commit, push and **Open merge request** already work on GitLab, Bitbucket, Gitea and Forgejo
([commits and pull requests](../guide/commits-and-pull-requests.md)), so a project cloned from
one of them is useful at once. Its issues and pull requests are not listed, and the Tasks and
Pull requests views say so per project, as they do now.

The choice in **Add a project** becomes **Clone a repository**, with GitHub named in the detail
line as where the list comes from.

## Starting from a link

Slice 2 adds one entry point, **Start from a link…**, in the command palette and beside **New
task** and the pull requests' **Refresh**. It takes an issue's or a pull request's address —
`https://github.com/owner/repo/issues/12`, `…/pull/12`, or `owner/repo#12` — and:

1. **The repository is a project**: the matching view opens with that item selected, so
   **Delegate** or **Start workspace** is the next press. Everything from there is the view's:
   the message, the composer, the link recorded against the task. A number given without
   `issues` or `pull` is asked of the forge, which says which it is.
2. **It is not**: the clone step opens with the repository filled in, and once the clone is
   done — the dialog still open or not — the item is opened as in 1. A clone in the background
   that finishes while you are elsewhere shows its notice and nothing more; the link is not
   remembered across a restart.
3. **It is on another forge**: the clone offer still stands; the item itself is opened in the
   browser, with the line the views use for a project that is not on GitHub.

Nothing starts without the composer, as [23](23-tasks.md) decided. The entry point adds no
state: it is `showTask` or `showPullRequest` with a project found by `forge::repo_at`.

## Commands

| Command                                                        | Slice |
| -------------------------------------------------------------- | ----- |
| `forge_repositories(refresh)` — the list, with `projectId` set | 1     |
| `forge_search_repositories(text)`                              | 1     |
| `project_clone` takes a clone URL and an optional `upstream`   | 1     |
| `project_clone` takes any https or ssh URL                     | 3     |
| `forge_resolve_link(text)` — project, kind and number          | 2     |

In `src-tauri/src/projects/commands.rs`, with the `gh` calls on `Gh` in `forge.rs` and the
types beside `Repo`. `bindings.ts` is regenerated, never edited. The dialog's list is held in
`src/stores/projects.ts`, read through `src/lib/ipc.ts` only.

## Tests

- **`gh`'s answers**: recorded JSON under `crates/core/fixtures/gh/2.102.0/` — a page of
  repositories with a fork and an archived one, a last page, a search result — parsed in unit
  tests; `git_protocol` deciding the URL.
- **The stand-in `gh`** that `forge.rs` already uses: two pages read in order; the second
  failing keeps the first; a search sends its text as a parameter and not in the command line.
- **Already added**: a real repository in a temp directory with a GitHub remote, registered as
  a project, is matched by host, owner and name, and not by its folder name.
- **Forks**: cloning a repository with a parent adds `upstream` with the parent's URL, in a
  temp directory with a bare "forge" repository standing in for GitHub.
- **Any host** (slice 3): a table of accepted and refused URLs — subgroups accepted, a local
  path, `-flag`, `git://` and `file://` refused — and the shorthand still meaning GitHub.
- **Links** (slice 2): each accepted form parsed; a project found from its remote; a number
  that is a pull request routed as one; a repository that is not a project opening the clone
  step with it filled in.
- **The dialog**, through Testing Library: typing filters; a pasted URL with no list; **Enter**
  on a row; **Already added** selecting rather than cloning; the search row sending one query
  and only on press; the remembered location collapsed and **Change…**; the `gh`-less fallback.
- **By hand**, on all three systems: a new section in [08](08-manual-checklist.md).

## Slices

Each is one pull request with its docs, tests and changelog line, built in the order 1, 3, 2.

- **Slice 1 — the list.** ✅ See [slice 1](#slice-1-what-shipped). `RemoteRepository`, the
  query, paging, the cap; the search row; the protocol; **Already added**; `upstream` for a
  fork; the remembered location collapsed; the dialog, its fallback without `gh`, and the
  screenshot. `docs/guide/projects.md` rewritten for it.
- **Slice 3 — any host.** ✅ See [slice 3](#slice-3-what-shipped). The validator, the renamed
  choice, the guide and the README line.
- **Slice 2 — from a link.** `forge_resolve_link`, the palette entry, the two headers' field,
  the clone-then-open path.

## Slice 1: what shipped

Built 2026-10-08, as proposed, with these differences and findings:

- **`repositories.rs` is its own module** in the core rather than more of `forge.rs`, which
  had passed 1,500 lines; the `gh` calls are an `impl Gh` block there. `Gh::run_within` became
  `pub(crate)` for it. The GraphQL helper in `forge.rs` could not be reused: it always sends
  `-F owner={owner} -F name={repo}`, and this query has no repository to resolve — it is asked
  from the profile's folder, so a project whose folder has gone cannot get in the way.
- **The search goes through `gh api -X GET search/repositories -f q=…`**, with the text as a
  parameter `gh` encodes, as proposed; the stand-in `gh` test checks the argument array.
- **The protocol is read once per dialog**, by the command, not the dialog: `gh config get
git_protocol` runs before the list and before each search, and the clone URL in every row is
  already the right one. The frontend never chooses.
- **A fork's `upstream` is added, then fetched, and a fetch that fails does not fail the
  clone** — the parent's URL came from the forge and the clone itself is good; the fetch can
  wait for a network that is there. A `remote add` that fails does. Tested against a bare
  parent and a clone of it in a temp directory.
- **The combobox**: the field is `role="combobox"` over a `listbox` whose rows are `option`s,
  with `aria-activedescendant` following **↑**/**↓**. A row is picked on mouse _down_, not
  click, so the field does not lose focus first. **Enter** on the highlighted row picks it; on
  the _Clone <url>_ row it submits, which is the same as the button.
- **The index into the list is clamped, not reset**: typing resets it to the first row, and a
  list that shrinks under it keeps it on the last row rather than on nothing.
- **Already added** matches by push remote through `forge::repo_at`, as proposed, for the list
  and for search results alike; the first match wins when two projects share a remote.
- **Create a project** got the collapsed location too, since the two forms share
  `Destination`; it had no reason to keep asking either.
- **Found**: an account in sixteen organisations reaches 64,812 repositories, which is why the
  list is owner + collaborator; and `gh`'s REST and GraphQL answers name the same things
  differently (`full_name` / `nameWithOwner`, `private` / `isPrivate`), so the search has its
  own reader and the two meet in one `RemoteRepository`.
- **Not done**: the screenshot, which wants a real window; a run against a real `gh` from the
  window, which the manual checklist's new section 25 covers; GitHub Enterprise hosts
  ([open question 34](06-open-questions.md)).

## Slice 3: what shipped

Built 2026-10-08, as proposed, with these differences:

- **`github_url` stayed, under `clone_url`.** The GitHub spellings are still normalized exactly
  as before — the same tests pass unchanged — and `clone_url` only takes over for an input
  with a scheme or a `host:` that is not GitHub's. `Projects::clone_github` is now
  `Projects::clone_from`.
- **A bare `host/path` is refused** for any host but GitHub. `gitlab.com/group/repo` cannot be
  told from `owner/repo/subdir`, and the shorthand had to keep meaning GitHub; so another host
  wants its scheme, or git's `user@host:path`. The guide says so.
- **Credentials in an HTTPS URL are refused** on every host, as they were on GitHub, and
  `http://` to another host too: a password or a plain-text clone are not things a dialog
  should pass to git without a word. `ssh://user@host:port/path` takes any user and a numeric
  port. The scp form wants a dot in the host, so `C:…` and `localhost:…` fall out; `ssh://`
  covers `localhost`.
- **Output is normalized with `.git`** on every host, as it was for GitHub; every forge named
  accepts it.
- **Not done**: a clone from a real GitLab or Forgejo from the window — the manual checklist's
  section 25 gained a row — and listing those hosts' issues and pull requests, which stays
  deferred as decided.

## Not in this version, on purpose

- Organisation repositories as a list. They are reached by search; a per-organisation list is
  [open question 33](06-open-questions.md).
- A default projects folder, so that the very first clone asks nothing. The location is
  remembered from the first one on, and inventing `~/Projects` for everyone is a guess.
- Issues and pull requests of GitLab, Forgejo, Gitea or Bitbucket.
- Creating a repository on the forge from **Create a new project**; `gh repo create` is one
  command and a natural next step, but not this issue's.
- Cloning with `gh repo clone`. It would add `upstream` for us, but it would also make the
  clone itself depend on `gh` and on its choice of protocol, and today's clone needs neither.
- Remembering a pasted link across a restart, or a queue of links.

## Open questions

[33–34 in open questions](06-open-questions.md#repositories): whether organisation
repositories should be listed per organisation rather than searched; whether a GitHub
Enterprise host should be offered in the list.

## Documentation this touches

`docs/guide/projects.md` (the clone step, rewritten), `docs/guide/tasks.md` and
`docs/guide/pull-requests.md` (slice 2), `docs/guide/troubleshooting.md` (`gh` met from a new
place), `docs/guide/shortcuts.md` (slice 2), `docs/quick-start.md`, `README.md` and the landing
page's copy, `CHANGELOG.md`, [03-architecture](03-architecture.md) (project cloning),
[05-roadmap](05-roadmap.md) (M25), [06-open-questions](06-open-questions.md) (33–34),
[08](08-manual-checklist.md) (a new section), and the add-project screenshot in `docs/images/`.
