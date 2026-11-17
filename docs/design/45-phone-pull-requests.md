# 45. Pull requests on the phone

Status: built on 2026-11-15, read-only. Not yet seen on a phone.

## The problem

The desktop shows what became of a workspace's branch: its pull request, whether CI passes, who
reviewed it, whether it still merges. The phone listed a computer's workspaces and their branches
and said nothing of any of that. Someone away from the computer could watch an agent push and
then had to open GitHub to learn whether the checks passed.

## What was decided

Proposed in Figma first, on the page **Pull requests · proposal** of the phone's file (frames P1
to P4), and answered by the owner on 2026-11-15:

1. **Read-only.** The phone shows a pull request. Merging and marking one ready from the phone
   are left for later.
2. **The list of every project's pull requests is built now** (P4), not later.
3. **No badge in Chat's bar.** It is full at a phone's width, and the badge is one press back.
4. **A push notification comes later**, as its own piece of work: issue #263.
5. **Pressing a badge opens the pull request's card in the app.** The card has the way out to
   the forge.

## What it is built from

The computer already reads its projects' pull requests with `gh`, with its person's sign-in, for
its own sidebar, toolbar and Pull requests view (`crates/core/src/forge.rs`, and the app's cache
of it in `src-tauri/src/publish`). The phone is told what was read. Three things follow, and all
three are meant:

- **The phone asks the computer, never the forge.** No forge credential is held on the phone, on
  the server or in the protocol, and nothing is asked of the forge for a phone that the computer
  would not ask for itself.
- **It is as old as the computer's last reading.** What the phone shows is what the window shows.
- **It needs `gh` on the computer, installed and signed in.** Without it nothing is shown and
  nothing complains, as on the desktop.

## The protocol: minor 11

Two operations (`crates/remote-proto/src/pull.rs`), both needing `workspace:read`. A pull request
is what a workspace list already says about a project, a little further on: a branch, then what
became of it. So no new permission is asked of anyone, and both keep to the projects a grant
reaches.

- **`pull.list`** gives a page of pull requests, `of` one kind:
  - `workspaces` (the default): those a workspace opened or follows, whatever their state. What a
    workspace list shows. Those workspaces opened come first, each workspace's in the computer's
    order for it, so the first of a workspace's own is the one the desktop shows on its row.
    Those followed in other repositories come after.
  - `open`, `merged`, `closed`: every one of that state the computer has read, most recently
    changed first.

  Each has what a row and a badge need: number, title, branches, state, draft, how its checks
  divide, the review decision, whether it conflicts, lines changed, author, when it last changed,
  and the workspaces it came out of. A page also says how many are open over every project the
  grant reaches, and whether the computer is reading them now.

- **`pull.describe`** gives one in full: each check by name, the failed first and then those
  running, and each reviewer's latest word. A project's own is named by its number. One a
  workspace follows in another repository is named by the computer's id for the following, since
  its number alone is the project's own of that number.

A host that reads no pull requests does not list the two operations among what it answers, and
refuses them as unsupported.

### What keeps an answer within a message

Every field has a limit: 40 checks of 100 characters, 12 reviewers, 8 workspaces. More checks
than that are counted (`moreChecks`), and the failed ones are never among those left out, since
they are sorted first. A test builds the longest pull request a forge could return, in characters
that are longest once written as JSON, and holds its description to one message.

### A link is only ever a web page

A pull request's link is opened in the phone's browser. The computer passes on only an `https`
link, and the client drops any other again when it reads the answer: a computer the phone is
paired with is trusted, but what it read came from a forge, and a link is the one thing here
that leaves the app.

## The computer never makes a phone wait for `gh`

A device's channel also carries its terminal. A question that waited a second or two for `gh`,
for each project, would freeze typing on the phone while it did.

So a question is answered at once with what was last read (`src-tauri/src/publish/lent.rs`).
When that is older than 45 seconds, or there is nothing, a reading is started off the channel
and the answer says one is on its way (`reading`). The phone asks again four seconds later and
has it. One reading runs for a project at a time.

Found while writing the smoke test: a reading that cannot succeed must not be tried at every
question. Every open pull request cannot be read of a forge `gh` does not speak to, and what
was last read never says they were, so a phone with the list open would have started a reading
each time it asked. A reading that takes in every open one now begins at most every 45 seconds
for a project, and any reading at most every 10.

The window reads a project's pull requests while it is in front. This is also what keeps them
read while only a phone is looking.

## A computer with no window

The desktop app's cache lives in the app, so `yardsort-host` has a reader of its own,
`remote::pull::polled` in the core (2026-11-17). It reads only when a device asks, on the same
terms as the window's lender: what was last read is answered at once, a reading older than 45
seconds starts another on a thread of its own and the answer says one is on its way, and one
that left nothing is not tried again for 10 seconds. A reading is `gh pr list` of the newest
fifty in the project's checkout, and `gh`'s open pull requests too when the device asks for every
open one; a reading of only the newest keeps the older open ones the last full one found. Which
workspace opened which is `forge::pull_requests_from`, as in the window. Without `gh`, or with it
signed out, the reading is of none.

Not read there yet: the pull requests a workspace follows in another repository.

## On the phone

Nothing of pull requests is kept on the phone. They show while the computer can be reached, and
a screen showing them asks again each minute while it is in front.

- **A workspace's row** (P1) carries its pull request's badge, coloured as on the desktop: green
  for one open with nothing pending or wrong, yellow for a draft or one waiting on checks or a
  review, red for a failed check, changes asked for or conflicts, purple for merged, and red
  struck through for closed. A merged or closed one says so in place of its number. `#88 +2` is
  a workspace with two more; `lib#7` is one in another repository. A screen reader is told what
  the colour means in words.
- **The workspace's screen** (P2, P3) has the card above Conversations: number, state, title,
  branch and base, lines added and removed, the checks in a line that opens to each by name,
  the review, conflicts, and **Open on GitHub**. The checks are open without a press when one
  failed. Others the workspace opened or follows are listed under it.
- **Pull requests** (P4), a button on the computer's screen with the count of open ones, lists
  them over every project: **Open**, **Merged** and **Closed**. Merged and closed are those among
  the newest fifty of each project, which is what the computer reads of them.
- **Pressing a badge or a row of the list** opens that pull request's own screen: its card, and
  the workspaces it came out of.

## Where it differs from the Figma file

- The light colours for green's tint and for merged are not in the file, whose three new
  variables have dark values only. The app's are chosen to match the others: `#e3f3e8`,
  `#6b46c1` on `#ece6fb`.
- P2 shows a workspace's other pull requests with a chevron and no screen for where it leads.
  They open the same screen a badge does.
- A pull request's own screen lists the workspaces it came out of, which no frame shows.

## Tests

- **Protocol**: fixtures for both requests and replies, read by Rust and by the TypeScript
  client; a word from a newer host is read as something else, not refused.
- **Host** (`crates/core/src/remote/tests/pull.rs`), against a real store with two projects:
  who may ask and of which projects, a host that reads none, the order of a workspace's own, one
  followed by two workspaces told once, a workspace put away, checks with the failed first,
  paging within a frame with none told twice, and the longest pull request there could be.
- **App**: what was last read is handed over whole and asks nobody; when a reading is due; that
  one is not started twice, and not retried at every question.
- **Phone**: the words and colours (`test/lib/pulls.test.ts`), and nine screen tests against the
  stand-in server: the badge and its words, a merged one, a computer that tells of none and is
  not asked, asking again while it reads and each minute after, the card and its way out, a link
  that is no web page, the badge's press, the list by state, and one the computer no longer has.
- **By eye**, on the Android emulator with sample data, beside frames P1, P2, P3 and P4.
- **Smoke test** `.yardsort/tests/remote-phone-sees-pull-requests.yaml`, passed on Linux on
  2026-11-15, 16 of 16 steps: the real window, a stand-in `gh` that answers from a file so no
  forge is asked and nobody's sign-in is used, and a pretend phone. The phone's first answer says
  the computer is reading; the next has the checkout's pull request as `gh` said it, its checks
  by name with the failed one first, and one never read refused as not found. `gh` was asked
  for the list and for nothing that changes a pull request.

  Its first run failed, on the test's own data. The second pull request, meant as someone
  else's, was given the base commit as its head. That commit is in the checkout's history, and
  a pull request whose head was made in a workspace is that workspace's own, whatever its branch
  is called (`forge::pull_requests_from`). So the computer was right to count it, and the data
  now gives it a head the checkout never had.

## What is left

- **See it on a phone**, iPhone and Android, against a real repository.
- A push notification when a check fails or a review arrives: issue #263.
- Merging and marking ready for review from the phone.
- On a computer with no window, the pull requests a workspace follows in another repository.
- The description and the conversation of a pull request, which the desktop's Summary shows.
