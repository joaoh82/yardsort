# Outcomes

Every workspace is an attempt at something: its first message is the task, and the agents that
held a conversation in it did the work. Outcomes are what became of each attempt — and, over
time, a record of how each agent does on your own work.

## Saying how an attempt went

Right after you **archive** or **delete** a workspace, the bottom of the sidebar asks: _How did
it go?_ **Kept**, **Partly** or **Discarded** — one click, and optional. **×** leaves it
unanswered; you can label it any time later.

**Outcomes…** in a [project's menu](projects.md#the-project-menu) lists every attempt in the
project, newest first — live, archived and deleted alike — with its task, its agents, its
outcome and the evidence beside it. Click a label to set it; click the same label again to take
it back.

## What counts

- **Your label** always counts, and wins over anything else.
- **A merge** counts as **kept** while you have said nothing: a pull request merged on the forge
  (seen through the [GitHub CLI](commits-and-pull-requests.md)), or a branch whose commits were
  later found in its base branch — a merge or a fast-forward. The view marks these _merged, not
  labelled_. A squash merge leaves no trace in git, which is why the pull request is read too.
  Resetting a branch back to its base throws its work away, and is not a merge. An attempt keeps
  the pull request opened while it was going; a later one on the same branch belongs to the
  attempt that opened it. Yardsort matches attempts to pull requests each time it reads the
  project's pull requests — for the badges on its workspace rows, the publish panel, the Pull
  requests view — so a merge is counted without you opening anything. The Outcomes view itself
  reads what is already known, and never waits on the network.
- **Nothing else.** A closed pull request, a branch ahead of its base, an archived or deleted
  workspace, an agent that exited with an error — each is shown as evidence beside the attempt,
  and none is ever counted as an outcome. An attempt with no label and no merge has no outcome.

What Yardsort can see for itself is limited to those two. A squash merge is only visible
through the pull request, so a project with no GitHub CLI, or a pull request older than the
newest fifty when Yardsort first looks, leaves the attempt for you to label. When a workspace is
archived or deleted its branch is checked one last time, so work that was ahead is on record
even if the branch is deleted afterwards.

Deleting a workspace keeps its attempt: its name, branch, task and agents are copied before the
workspace goes, so it can still be judged. Removing a project from Yardsort without keeping its
history removes its outcomes too.

## Each agent's history

Above the attempts, the Outcomes view gives each agent's history in the project: _kept 3 of 5 (1
by merge) · partly 1 · discarded 1_. An attempt two agents worked in — a handoff, say — counts for
both.

The composer shows the same for the agent you have picked, across all your projects, under its
pickers. Until an agent has five attempts with an outcome, it says _too few to say yet_ with the
count so far: a handful of attempts never makes a winner. It is shown, never used to choose for
you.
