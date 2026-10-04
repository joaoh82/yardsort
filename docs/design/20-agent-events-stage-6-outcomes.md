# 20 — Agent events, stage 6: outcomes

Status: **first slice shipped** · 27 September 2026 · Stage 6 of
[09](09-agent-events-and-memory.md): _first-class task/attempt relations and measured outcomes,
then optional suggestions using user strengths plus local history_, with the exit gate _success
labels distinguish explicit user choice/merge/test evidence from weak heuristics; small sample
sizes do not imply a winner_.

## 1 · What the fit pass found

- **Evidence the code already sees.** Strong: a pull request's merged state (the publish panel's
  `gh pr list --state all`, cached per project); a branch merged into its base, from git. Weak:
  a workspace archived or deleted with its branch unmerged, runs that exited with errors, tools
  that failed. Missing: any outcome the user states, and any notion of a task beyond a
  workspace's first message.
- **A squash merge leaves no trace in git.** This repository, like most on GitHub, squash-merges:
  the branch's commits never reach the base's history. Git alone cannot tell a squash-merged
  branch from an abandoned one, so the forge's pull-request state is the reliable signal; git
  still catches true merges and fast-forwards.
- **Deleting a workspace removes its row and cascades its sessions** — the moment an attempt is
  most often judged is the moment its record disappears. An outcome must carry its own copy.
- **An empty range proves nothing.** `base..branch` is empty for a branch whose work reached the
  base, for one that never had any, and for one reset or recreated since; only the commit seen
  ahead, checked against the base, tells them apart.

## 2 · Decisions

Four put to the user on 2026-09-27; all answered as proposed.

- **An attempt is a workspace.** Its task is its first message, its agents its sessions'
  harnesses. No new way to start work; "try with several agents" is not built.
- **Outcomes: the user labels, merges back them.** _Kept_, _partly_, _discarded_ — one optional
  click after archiving or deleting, and any time in the Outcomes view. Without a label, a merged
  pull request or a git merge counts as kept, marked as such. Nothing weak is ever an outcome.
- **Where it shows:** beside the composer's agent picker (per agent, across projects) and an
  Outcomes view per project. Not fed to Assist.
- **Jev later.** A judgment of whether an attempt met its request is a guess about quality, and 09
  says never to treat it as ground truth; it is worth more once there are labels to compare with.

Further decisions made in building:

- **A self-contained row.** `workspace_outcomes` is keyed by the workspace's id (never reused),
  carries the name, branch, base, task (300 characters) and harnesses, and keeps
  `workspace_id` only as a nullable link. `Workspaces::delete` and `archive` snapshot before
  removing anything, in the core, so `ys workspace delete` keeps outcomes too; a failed snapshot
  never blocks a delete. `restore` clears how it ended.
- **Git merged means "the tip seen ahead reached the base".** While `base..branch` has commits,
  the branch's tip is recorded (`ahead_tip`); a merge is recorded only when that tip is found
  reachable from the base (`merge-base --is-ancestor`). An empty `base..branch` proves nothing
  alone: a branch reset to its base or recreated has one too, and its discarded work is not in
  the base. A branch deleted after a real merge still counts. (Found in review: the first cut
  read "ahead, then empty" as merged, so a reset counted as kept.)
- **An attempt keeps its own pull request.** Branch names are reused (`open_branch`), so an
  attempt follows the state of the pull request it was first matched to and is never re-matched
  by branch name. A first match needs the pull request to have been opened during the attempt's
  life — from its first record (less ten minutes) to its archive or delete — using `createdAt`
  from `gh`, which the forge now asks for and never sends to the window. (Found in review: a later
  pull request on a reused branch rewrote an earlier, deleted attempt.)
- **The forge's state is read from the cache only.** `outcomes_get` uses `Forge::cached`, which
  never fetches, whatever its age; the publish panel keeps it current. (Found in review: the first
  cut's "not a refresh" still fetched on a stale cache, so opening Outcomes could wait on `gh`.)
- **An attempt begins when its workspace did** (4 October 2026, migration `0015`). The window
  above was measured from the outcome row's `created_at`, which is when the row was first
  written — and for a workspace nobody opened Outcomes on, that is the moment it was deleted. So
  a pull request had to be opened in the ten minutes before the delete to be matched; on the
  user's own profile seven attempts, most merged through pull requests, had no evidence at all.
  `began_at` is the workspace's creation time, copied by the snapshot. Rows from before it,
  whose workspace is gone, have none: their lower bound is the end of the attempt before them on
  the same branch, which is all a reused branch name could confuse.
- **Evidence is gathered where it is seen, not where it is shown.** Every read of a project's
  pull requests (`publish::commands::look`) snapshots its live workspaces and matches attempts
  (`outcomes::observe_project`), so a merge counts without the Outcomes view being opened.
  Archiving or deleting reads git's evidence for that attempt once more
  (`outcomes::observe_ending`), while the branch certainly exists. `outcomes_get` still never
  fetches.
- **Shared work counts for each agent.** An attempt two agents worked in counts in both
  histories.
- **Five outcomes before a history says anything** (`MIN_SAMPLE`); below that, "too few to say
  yet" with the count. Always with the sample: _kept 3 of 5 (1 by merge)_.
- **Local workspaces are not attempts.** The project's own checkout is skipped.

## 3 · What shipped

- Migration `0011_workspace_outcomes.sql`.
- `crates/core/src/outcomes.rs`: `snapshot`, `observe_git`, `observe_pull_requests`, `refresh`,
  `outcome` (label, merge, unknown), `history` with `MIN_SAMPLE`; the store's outcome methods;
  delete, archive and restore hooks.
- App: `outcomes_get`, `outcome_label`, `outcomes_agents`. The sidebar's _How did it go?_ after an
  archive or delete; **Outcomes…** in the project menu; the composer's history line.
- Guide: [Outcomes](../guide/outcomes.md); workspaces and projects guides; checklist
  [08 §21](08-manual-checklist.md#21--outcomes).

## 4 · Verification

- `outcomes` (6): a snapshot outlives its workspace, with name, branch, task and agents in
  first-seen order, and can be labelled afterwards; git's merge is seen only after the branch was
  ahead (empty, then ahead, then fast-forwarded, in a real repository); a merged pull request is
  kept until labelled otherwise, a closed one is not an outcome, and taking a label back returns
  to the merge; a later pull request on a reused branch never rewrites an earlier attempt, and
  none opened after an attempt ended is matched to it; a branch reset to its base is not a merge,
  and a branch deleted after a real merge still is; history counts outcomes only, credits both
  agents of a handoff, counts kept by merge, and is not enough below five. `workspaces`: deleting
  and archiving keep the outcome with how it ended; restoring clears it.
- Frontend: the prompt appears only after a delete that happened, records the answer, can be
  dismissed; the Outcomes view's histories, outcome words, evidence and labelling (and taking a
  label back); the composer's history line and its "too few" wording.
- Live: owed — [08 §21](08-manual-checklist.md#21--outcomes).

## 5 · Not in this slice

- **Jev's judgment** of whether an attempt met its request (09's stage-6 row), and **suggestions**
  from local history: after there are labels to measure them against.
- **Try with several agents**: grouping attempts at one task, for head-to-head comparison.
- **Test evidence**: no adapter records output, so a passing test run is not evidence yet; a
  pull request's check results are available and could be shown beside an attempt.
- **Stage 7**, optional sync, stands as written in 09 — only if there is demand.
