# Tasks

What is waiting to be done in every project, in one place. To begin with that means each
project's **GitHub issues**: who opened each, what has been said, and which ones are waiting on
you. Open it with **Tasks**, under Pull requests at the top of the sidebar, or from the
[command palette](shortcuts.md#navigate-without-the-mouse) (**Mod+K**, then _Tasks_). It takes
over the center panel the way [Pull requests](pull-requests.md) and [Usage](usage.md) do; **×**
in its header closes it, and so does pressing the row again or selecting a workspace.

It needs the [GitHub CLI](https://cli.github.com), installed and logged in, and it is for
projects on GitHub. Yardsort holds no credential of its own: everything here is `gh`, with the
permissions you already have.

This first version **reads**. Handing a task to an agent, and creating, answering and closing
tasks from here, are the next two steps and are not built yet; **Open on GitHub** is the way to
act on one until then. The same list is on the command line as
[`ys task`](cli.md#ys-task), which is how an agent in a workspace sees it.

## The number in the sidebar

The number on the **Tasks** row is how many open tasks **need an answer** across your projects —
not how many are open. It is the count that asks something of you, and it goes away when
nothing is waiting.

### What "needs an answer" means

An open task needs an answer when the last person to speak on it is not one of the project's
maintainers:

- nobody has commented, and it was opened by someone who is not a maintainer; or
- the latest comment by a person is from someone who is not a maintainer.

A **maintainer** is whoever GitHub says is one: an owner of the repository, a member of its
organisation, or a collaborator. So a co-maintainer's reply counts, and there is nothing to
configure. A **bot**'s comment is nobody's word: a triage bot's "thanks for the report" does not
answer a reporter, and a stale-bot's nudge after your reply does not ask you anything. Neither
does a comment from an account that has since been deleted.

An issue you opened yourself, that nobody has commented on, is not waiting on you.

Yardsort looks at the last five comments of each task to decide. On a task whose last five
comments are all from bots, the person who opened it decides.

## The list

One row per task, most recently updated first, from every project whose repository is on
GitHub:

- the **project** and the task's **key** (`#91`), **Needs an answer** when it does, and a pill
  that says **Open**, **Closed**, **Not planned** or **Duplicate**;
- the **title**;
- who **opened** it, how long ago it last **changed**, how many **comments** it has, who it is
  **assigned** to, an open **pull request** that will close it when it merges, and its
  **labels** in their own colours.

**↑** and **↓** move between rows, **Home** and **End** jump to the ends, and **Enter** opens the
one in focus.

### What is in it

Every **open** issue of each repository, up to the 200 most recently updated. Past 200 the view
says so — _Showing the 200 most recently updated of 1,039 open in api-gateway_ — with a link to
the rest on GitHub. Pull requests are not tasks; they have [their own view](pull-requests.md).

**Closed** issues are read only when you turn to the **Closed** or **All** tab, and then the 50
most recently updated of each project. An older closed issue is on GitHub.

## Filters

| Filter              | What it does                                                                  |
| ------------------- | ----------------------------------------------------------------------------- |
| **State**           | The tabs in the header: **Open**, **Closed**, **All**. It starts on **Open**. |
| **Projects**        | Any number of projects. None chosen means all of them.                        |
| **Label**           | One label, out of every label in the list.                                    |
| **Assignee**        | One person, **Me**, or **Assigned to no one**.                                |
| **Author**          | One author, out of everyone in the list, or **Me**.                           |
| **Needs an answer** | Only the tasks waiting on a maintainer.                                       |
| **Search**          | Title or number. `#12` and `12` both find number 12.                          |

**Me** is whoever `gh` is logged in as, and is offered once `gh` has said who that is.

Everything but the search is **remembered** across restarts. **Clear filters** appears whenever
something is narrowing the list, and puts everything but the tab back.

Filters and search work on the tasks that are loaded. That is why the view tells you when a
repository has more than it read, instead of letting a search come up empty without a word.

## One task in full

Press a row and the task opens beside the list:

- its **title**, state, labels, who it is **assigned to**, and the pull request that will close
  it, if one is open;
- its **description**, as Markdown;
- its **conversation**, oldest first. Each comment says who wrote it and when; a maintainer's
  and a bot's are marked. A comment GitHub has hidden — as spam, off-topic, outdated — stays
  hidden here, and says why.

A conversation longer than a hundred comments shows its latest hundred, says how many there
are, and links to the rest.

Descriptions and comments are written by anyone who can reach the repository, so they are shown
with the same care as a pull request's: no HTML, **no image is loaded** — it becomes a link —
and every link opens in your browser, never in the Yardsort window.

**Open on GitHub** opens the issue in the browser, and **Copy link** copies its address. **⇤**
hides the list to give the task the whole width, **⇥** brings it back, and dragging the divider
resizes the two. **Esc** or **×** closes the details.

While the details are open they follow the list: a new comment on GitHub appears within about a
minute, without pressing anything.

## How often GitHub is asked

Once for each project when Yardsort starts, so the sidebar's number is there. After that, only
while the Tasks view is open: when you open it, when the window comes back into focus, every
minute, and on **Refresh**. With the view closed nothing is asked, so the number in the sidebar
is as of the last time you looked.

## When something is missing

A line above the list says why a project has no rows, without hiding the other projects':

- **`gh` is not installed**, or **nobody is logged in** — install the
  [GitHub CLI](https://cli.github.com), run `gh auth login`, then **Refresh**.
- **The project has no remote on a forge** — there is nowhere to ask.
- **The project is on GitLab or Bitbucket** — tasks are read for GitHub only, for now.
- **The repository has issues switched off** — it has no tasks.
- **`gh` said something else** — its own words are shown, with **Retry**. The rows that did
  arrive stay.

## What it does not do, yet

- **Hand a task to an agent**, with the task as the workspace's first message. Next.
- **Create, answer, close, reopen, label or assign** a task. After that. Until then, use
  **Open on GitHub**, or `gh issue` in a terminal.
- **Other sources.** Linear and others are planned behind the same view; only GitHub issues are
  read today.
- Search GitHub. Search is over what is loaded.
