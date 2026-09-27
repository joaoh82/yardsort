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
- **An empty branch looks merged.** `base..branch` is empty both for a branch whose work reached
  the base and for one that never had any; only having seen it ahead tells them apart.

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
- **Git merged means "ahead, then not".** `ahead_at` is set the first time `base..branch` has
  commits; `merged_at` only after that, when it has none. An empty branch is never merged.
- **The forge's state is read, never fetched for this.** `outcomes_get` reuses the pull requests
  the app has cached for the project; the newest per branch wins.
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

- `outcomes` (4): a snapshot outlives its workspace, with name, branch, task and agents in
  first-seen order, and can be labelled afterwards; git's merge is seen only after the branch was
  ahead (empty, then ahead, then fast-forwarded, in a real repository); a merged pull request is
  kept until labelled otherwise, a closed one is not an outcome, and taking a label back returns
  to the merge; history counts outcomes only, credits both agents of a handoff, counts kept by
  merge, and is not enough below five. `workspaces`: deleting and archiving keep the outcome with
  how it ended; restoring clears it.
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
