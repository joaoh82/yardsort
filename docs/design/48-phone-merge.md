# 48. Merging from the phone

Status: proposed, answered and built on 2026-11-18. Not yet seen on a phone. The screens are on the page **Merge ·
proposal** of the phone's Figma file (frames M1 to M5).

## The problem

The phone shows a workspace's pull request with its checks and reviews (45), on the desktop app
and on a computer with no screen alike. When the checks pass and the review is in, the last
thing left is to merge it, and for that the person still has to open GitHub, sign in there on the
phone, and find the pull request again. The computer already has `gh` signed in, and already
merges for the desktop window when that window works on another computer (44, slice 5).

## What it is built from

Nearly all of it exists. The phone needs screens, not a new protocol.

- **`pull_request.merge`** (protocol minor 14, under `terminal:control`) takes a workspace, the
  pull request's number, a method, and the commit the person saw at its head. The host merges only
  the workspace's own pull request, open, not a draft, still at that commit, and asks `gh` to
  match that commit too (`--match-head-commit`). A pull request pushed to since the phone read it
  is refused with "The pull request changed or is no longer ready to merge", and `gh` is not
  asked. Both hosts answer it: the desktop app (`src-tauri/src/remote.rs`) and `yardsort-host`
  (`crates/host/src/home.rs`), through `crates/core/src/remote/publish.rs`.
- **The head commit** is not in what `pull.list` and `pull.describe` carry. `workspace.publish`
  has it (`PublishState.pullRequest.headOid`), with `gh` saying whether the computer can merge
  without a browser. The card asks for it when it opens on a workspace's own pull request.
- **The TypeScript client** already has `Channel.mergePullRequest` (`packages/remote-client`).
- **Its refusals are sentences the host wrote**, never what Git or `gh` printed: no `gh`, the
  forge did not answer, it changed, the forge refused (checks or branch rules).
- **The smoke test harness** already checks a merge from the window (`publish-there.yaml`:
  `merges-there`), and `headless-phone-sees-pull-requests.yaml` already pairs a pretend phone
  with a host that has a stand-in `gh`.

## What is proposed

**Where.** On the pull request's card (the one a badge opens), a **Merge…** button below the
checks and reviews, beside the way out to the forge. Nowhere else: not on the list's rows, and not
in Chat's bar (45 kept that bar for the session).

**When it shows.** All of these:

- the pull request is a workspace's own (its badge is on a workspace), open, and not a draft;
- this phone was let type on that computer (`terminal:control`);
- the computer speaks minor 14 or later, and `workspace.publish` says it has `gh` and gives a
  head commit.

Otherwise there is no button. A pull request on the open list that no workspace opened says to
merge it on the forge, since the host merges only a workspace's own. A draft says it is a draft.

**When it is shown but cannot be pressed**, with the reason under it:

- the forge says it conflicts: "Conflicts with main. Resolve them in the workspace first.";
- the computer is not reachable just now.

**Pressing it** opens a sheet:

- the title and number, and the commit it will merge at, short (`a1b2c3d`);
- the method: **Squash** (the default, as in the window), **Merge**, **Rebase**;
- a line when the checks are failing or still running ("1 check failed"). The forge's own rules
  have the last word, so this warns and does not block;
- **Merge** and **Cancel**. The branch is not deleted, as in the window.

**After.**

- Merged: the card says so, and the list and the badge are read again. Merged means purple, as
  in 45.
- Refused: the host's sentence, and the card is read again. For "it changed", the new head is
  then shown, and pressing again merges at that one.
- No answer (the link dropped mid-way): "It may have been merged. Reading it again.", and it is
  read again. Nothing is sent a second time by itself.

**Nothing new on the computer.** No protocol minor, no new scope, no new host code, apart from
what review turns up.

## Testing

- The phone's own tests: the button's conditions one by one, the sheet, a refusal's words, and
  what a lost answer says (the logic's tests; the screen tests do not wait out a lost answer).
- A smoke test, `headless-phone-merges.yaml`, built on `headless-phone-sees-pull-requests`: the
  pretend phone merges at a stale head and is refused, with `gh` not asked to merge; then at the
  head, and `gh` is asked `pr merge 12 --squash --match-head-commit <head>`. A phone let only
  watch being refused is `publish-there.yaml`'s already. The stand-in `gh` says yes to a merge, and no forge is reached.
- On the iPhone, against `yardsort-box` and the demo repository, as for 45.

## Slices

One pull request: the card's button and sheet, its tests, the smoke test, the guide
(`docs/guide/remote.md`, the phone's pull requests) and the changelog. Figma first.

## What was decided

Answered by the owner on 2026-11-18, after the Figma frames:

1. **Squash by default, with all three methods offered**, as the window does.
2. **Failing or running checks warn and do not block.** The forge's own rules decide.
3. **The branch is kept**, as drawn and as the window does. Deleting the workspace stays a
   separate act.
4. **Mark ready for review comes later**, on its own: it needs a new operation on the host and a
   protocol minor.
5. **No Face ID or passcode.** The sheet names the commit, and the host refuses one that changed.

## What was built

- `apps/mobile/src/lib/merge.ts`: when the card offers Merge…, the sheet's words, and what a
  refusal or a lost answer says. `apps/mobile/src/ui/merge.tsx`: `PullMerge`, which asks
  `workspace.publish` (with `forge`) for the head with each reading of the card, and the sheet.
  The card is on the workspace screen and on the pull request's own screen, and both carry it.
- A phone let only watch is shown no Merge…: `workspace.publish` is under `terminal:control` too,
  so the computer refuses to say where the workspace stands, and with no commit to name there is
  nothing to offer. The phone does not know its grant's scopes; this is how it learns.
- Tests: `apps/mobile/test/lib/merge.test.ts`, and the card's merging in
  `apps/mobile/test/screens/app.test.tsx` against the fake computer, which answers
  `workspace.publish` and `pull_request.merge` now. The smoke test `headless-phone-merges.yaml`
  passed on 2026-11-18, run by hand: a merge at a stale commit was refused and `gh` was not asked
  to merge; at the head, `gh` was asked `pr merge 12 --squash --match-head-commit <head>`. The
  stand-in `gh` of `app.sh seed-a-pull-request` says yes to a merge for it.
