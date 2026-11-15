# Remote, R6: durable sync

**Status:** planned on 2026-11-13. The owner took the defaults for the four questions in §3 the same
day. Slice 1, the server's records, was built on 2026-11-14 (§9); its tests and its local smoke test
passed, and so did its staging run on 2026-11-15. Slice 2, turning a project on and linking it, was
built the same day (§10); its local smoke test passed, and so did its owner run through staging,
this computer and the box, on 2026-11-15. Slice 3, approved notes, was built on 2026-11-15 (§11);
its local smoke test passed, and its owner run through staging is the owner's to make. Slice 4,
workspaces seen elsewhere, was built the same day (§12); its local smoke test passed. Slice 5,
checking a branch out here, was built the same day too (§13): all five slices are built, and the
gate's runs through staging (§5) are the owner's to make.

This is the plan for the phase after R4 and R5 ([35](35-remote-r4-r5.md)): what it has to show, what
it starts from, the decisions taken before any code, the slices in the order they are built, and
what each slice's smoke test needs. Like 35, it becomes the record of the slices as they are built.
The proposal is [25](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md), §10 for sync itself; R0 and
R1 are in [26](26-remote-r0-r1.md), whose ledger (§8) says which phase is where.

## 1 · What the phase has to show

**The gate** (proposal §13):

- **offline edits converge or surface conflicts**: two installations, each editing while the other
  is offline, end with the same records, or with a conflict the person sees and settles, never with
  one side's text dropped by a clock;
- **revocations do not resurrect**: a note revoked on one installation never comes back as approved
  from another one's older copy, a pending edit included;
- **no files or processes silently migrate**: syncing moves records only. No checkout, worktree,
  branch, terminal or agent appears, moves or ends on another machine without the person's own act
  there.

In R6, a person can:

- turn on sync for a project on one computer, after seeing what would go up;
- link that project on another computer, to a checkout that is already there, and choose which one;
- have the notes they approved on either computer reach the other's agents, and revoke one
  everywhere;
- see the other computer's workspaces for that project, with the branch, the task and where each
  lives, without any of them being made here;
- settle a conflict between two edits of one note, with both texts in front of them;
- stop syncing a project, and remove its records from the server.

**Not in R6** (proposal §10.1): candidate and rejected notes, the memory-sharing switch, paths,
running sessions, transcripts, native harness history, Git objects, uncommitted changes, credentials
and environments. Each stays on its own machine.

## 2 · Where it starts

Checked against `main` at `953bc8f` on 2026-11-13. No open pull request or branch touches sync.

**Nothing for sync exists, in code or schema.** The only mentions are two doc comments: the protocol
is versioned apart from "any future sync schema" (`crates/remote-proto/src/lib.rs:5`), and
`ProjectId` is host-scoped "until sync links them explicitly" (`crates/remote-proto/src/ids.rs:102`).
The ledger already says what R6 is: "a mapping table beside the local ids, never a replacement"
([26](26-remote-r0-r1.md) §3) and an "outbox written in the same transaction as each opted-in
mutation" (26 §8).

**The local side.** SQLite, 40 migrations, ids are UUID v4 text and times are milliseconds
(`store.rs`, `new_id`, `now_ms`). The `Store` is one `rusqlite::Connection` behind a mutex, in WAL,
with immediate transactions, since the app and `ys` write at once.

| Records           | Tables                                                             | What already exists that sync needs                                                                                                   | What does not                                                                                                      |
| ----------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Projects          | `projects` (0001, `removed` 0006): id, name, `root_path` unique    | One `add` in the policy layer (`projects/mod.rs`), deduplicating on the path                                                          | Any identity beyond the path. The Git remote is read live (`git.rs` `remote_url`, `forge.rs` `repos_at`), not kept |
| Workspaces        | `workspaces` (0001, `forgotten` 0005), `workspace_tasks` (0014)    | Name, branch, base branch, archived, task links                                                                                       | A transaction around most writes; `branch` is only set at creation and revival, the live HEAD is read from Git     |
| Memory            | `memory_entries`, `memory_history`, `project_memory` (all 0010)    | One table for all four states, an append-only history with the replaced text, provenance by name beside the ids, writes transactional | A revision number per entry (`memory_history.seq` is global), and any notion of where a note was approved          |
| Outboxes, cursors | `mobile_notice_outbox` (0036), the activity cursors in `remote.rs` | A retried, settled, pruned outbox pattern (`notify.rs` `enqueue_on`, `settle`, `prune_outbox`)                                        | An outbox written in the same transaction as the change it records                                                 |

**Every write is one of about eighteen `Store` methods in `crates/core/src/store.rs`.** Projects:
`add_project` `:1461`, `revive_project` `:1512`, `remove_project` `:2019`, `reorder_projects`.
Workspaces: `add_worktree` `:1727`, `add_worktree_if_new`, `remove_worktree`, `rename_workspace`
`:1794`, `set_workspace_archived` `:1801`, `forget_workspace` `:1812`, `revive_workspace` `:1823`,
`link_task` `:1562`. Memory: `add_memory` `:2182`, `propose_memory` `:2217`, `change_memory` `:2292`,
`set_memory_shared` `:2357`. No other SQL writes these tables outside tests. Their callers are the
app's commands (`src-tauri/src/projects/commands.rs`, `workspaces/commands.rs`, `memory.rs`), `ys`
(`project add`, `workspace new` and `delete`, `pr checkout`, `memory propose`), the remote facade's
`workspace.start` (`remote/start.rs`), and two that write while listing: `adopt_unknown` and
`import_worktrees` (`workspaces/mod.rs:413`, `:469`). So the outbox goes **inside those methods'
transactions**, not in the layer above. The workspace methods that are single statements today gain a
transaction. Rows a foreign key cascades away (a project's removal takes its workspaces and notes)
are recorded by the method that starts the cascade.

**An agent reaches memory only through `ys memory propose`**, which always makes a candidate; the
CLI has no approve, edit, reject or revoke, and a test asserts it ([19](19-agent-events-stage-5-memory.md)).
The remote facade has `memory.list` for approved notes and no memory write at all (`remote.rs`).
Sync must keep both true.

**The server side.** `crates/server`, `tokio-postgres` with a pool, nine migrations embedded by hand
in `postgres.rs` and run at start under an advisory lock, append-only. Every store method is on the
`Records` trait twice, `MemoryRecords` for tests and the stand-in and `PgRecords`, both held to
`tests/records.rs`. Every per-account table cascades from `accounts`, and every query filters by
account and answers "not found" for another account's id. One kind of credential, a device's bearer
token; the device's kind (`Phone`, `Computer`, `Host`, `Other`) is what tells a host apart, and
`submit_notice` already refuses anything but a host (`notices.rs:88`). A body limit of 16 KiB
(`http.rs:49`), per-route rate gates, background loops by `tokio::spawn`. No audit table, no general
idempotency table (notices deduplicate by `UNIQUE (account, event_key)`), and account deletion and
export are not built ([27](27-remote-r2.md)).

**The desktop is two devices**: a `Host`, which serves its own work, and a `Computer`, its client of
other hosts (`remote-host/src/computer.rs`). `yardsort-host` is one `Host`. A phone has no project
data of its own.

**The stand-in** (`crates/server/examples/stand-in.rs`) is the real server on `MemoryRecords`, and the
Remote smoke tests already run against it, so sync's smoke tests can too.

## 3 · Decisions

**Sync is between hosts.** Each installation syncs as its `Host` device: the desktop with Remote on,
and `yardsort-host`. The `Computer` device and the phone never push or pull sync records. The phone
keeps reading approved notes from a host it is let into, through `memory.list`, as now. A desktop
without Remote on has no sync, and loses nothing it has today.

**A project syncs only where the person turned it on or linked it, at that machine.** Being on the
account is not enough, the same rule as a host's grant (35 §3). A host pulls only the synced projects
it is linked to, and the server refuses it any other. On the desktop, turning on and linking are in
the window. On `yardsort-host` they are commands behind the same guard as `pair`: the operator's own
terminal, never an agent's (35 §11). `ys` gets no sync command that changes anything, so no agent
turns sync on, links a project or settles a conflict.

**Logical ids beside the local ones.** A local table maps `(kind, local id)` to a logical id, a
UUID v4 made by the installation that first syncs the record. Local ids, protocol ids and the
facade are unchanged; a phone still sees each host's own ids. The server never sees a local id.

**Linking is explicit, and the person chooses the checkout.** Turning a project on sends its name and,
as a hint shown when linking elsewhere, its fetch remote's address with any user and password
removed. On the other machine, **Link a synced project** lists the account's synced projects that
host is not linked to, with each one's hint, and the person picks the local project it is. Yardsort
may put the project whose remote matches first; it never links by itself (proposal §10.1). A project
with no checkout here stays unlinked; making one is the person's `git clone`, then a link.

**What a record carries.**

| Kind          | Synced                                                                                                                                                                   | Never synced                                                   |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------- |
| Project       | Name, the remote hint                                                                                                                                                    | Path, sort order, automation, the sharing switch               |
| Approved note | Text, approved or revoked, the revision, its history (who did what, when, the text an edit replaced), provenance by name: harness, workspace name, the host it came from | Candidates and rejected notes, the local workspace and run ids |
| Workspace     | Name, branch, base branch, archived or not, task links, the host it lives on                                                                                             | Path, preparation state, sessions, outcomes                    |

A note's source on another machine shows as "from Build box, workspace fix-login", with "source
unavailable on this device" where the local run would have been a link (proposal §10.3).

**A note approved elsewhere arrives approved**, since it was the person who approved it there
(settled by the owner, §3). A note an agent proposed is a candidate and stays on its own machine
until the person approves it there; approval is still only the person's act at a machine.

**Revisions and conflicts.** Every record has a server revision. An op names the revision it was
made against. The server applies an op whose base is current and returns a conflict for one whose
base is not, with the current record. No wall clock decides anything; times are shown only.

- **Note text:** a conflict keeps both texts. The note keeps its last uncontested approved text for
  injection, and the Memory view shows the conflict in a section of its own, with both texts, until
  the person keeps one, edits a third, or revokes it. That choice is a new op against the latest
  revision.
- **Revocation wins.** A revoke applies over any base. A stale edit or approval of a revoked note
  becomes a conflict, never a restore. Restoring is the person's new act against the latest
  revision, as it is locally.
- **Workspace metadata:** a conflict shows both; nothing renames a branch, moves a directory or ends
  a session because of a record.
- **Deleted records** keep a tombstone with their last revision, so a stale create or edit is
  refused rather than bringing them back.

**Push and pull.** The core's sync loop runs in the host runtime, so the app and `yardsort-host` share
it. It pushes the outbox in batches of at most 100 ops, then pulls after its cursor, and applies a
pulled batch and advances the cursor in one transaction, writing no outbox for what it imports (no
echo). It runs at start, after a local change, when the server hints that a linked project changed,
and every five minutes. The hint is a message on the host's relay link, which it already holds, and
carries no record. Transient failures retry with backoff and jitter; a refused op is held in a
visible state, never retried forever. Sync never blocks a terminal, a launch or the window; while the
server is unreachable, everything works locally and the outbox waits.

**The server.** New tables keyed by account and cascading from it: the records with their revisions
and tombstones, an append-only change log with a cursor per account, the op ids it has applied, and
which hosts are linked to which project. `POST /v1/sync/push`, `GET /v1/sync/pull?after=`, and
`GET /v1/sync/snapshot` for a host that is new to a project or whose cursor fell behind the retention
floor. Host devices only; their own body limit (256 KiB) and rate gate. One transaction per op:
check the account and the link, deduplicate the op id, check the base revision, write the record,
append the change. A pruner loop removes changes and op ids past retention with bounded deletes,
and turns tombstones past it into markers (§9), safe on several instances.

**Retention.** Changes and tombstones 90 days; op ids 30 days, the offline retry window. A host whose
cursor is older than the floor takes a snapshot; its pending ops are kept and pushed against the
snapshot's revisions, and any that no longer apply become conflicts (proposal §10.2, §10.3).

**Stopping.** **Stop syncing** on one machine unlinks that host and leaves the records for the
others. **Remove from the server** deletes the project's records everywhere, leaving tombstones until
retention, and every linked host keeps its local copy, unlinked. Account deletion and export stay
with R7, where they are planned, and will cascade to these tables.

**Audit.** The change log is the record of sync mutations: kind, logical id, revision, source host,
outcome and time, never text. No server log line carries a note's text, a name or a hint (the leak
audit from 35 §19 extends to sync).

**Versions.** Sync's records have a schema version of their own, beside `PROTOCOL_MINOR`. A host that
does not know a record's version leaves it on the server untouched and says so, rather than
dropping fields. Only the relay hint is a protocol change: minor 9, since minor 8 went to the
phone's chat ([43](43-phone-chat.md)).

**Settled by the owner on 2026-11-13, all four as the defaults:**

1. **What the server can see.** Synced notes, names and remote hints are kept in the server's
   PostgreSQL in plain text, readable by its operator, as relayed terminals already are in passing.
   Decided: say so in the guide and in the preview before anything goes up, and add no encryption of
   our own (proposal §9). End-to-end encryption of synced records would be its own reviewed design,
   after R6.
2. **A note approved on one machine arrives approved on the others.** Decided, as above. Not
   chosen: arriving as a candidate, to be approved on each machine, safer against a compromised host
   and more clicks for the person.
3. **The remote hint.** Decided: the fetch remote's address goes up, without credentials, so linking
   can suggest a match. Not chosen: the name alone.
4. **Choosing where a task runs** (35 §9). Decided: out of R6, its own milestone after slice 3,
   since it needs only linked projects. Not chosen: R6's last slice.

## 4 · Slices

| #   | Slice                                             | What it adds                                                                                                                                                                                                                                                                                                                                                          | Its smoke test                                                                                                                                                                                                                                                          |
| --- | ------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | The server's records (built, §9)                  | `remote_proto::sync`: logical ids, op ids, revisions, cursors, ops and their answers, with fixtures. The server's tables, `Records` methods on both stores, the three routes, the pruner, the relay hint (minor 9). No client                                                                                                                                         | The server's own: two accounts, duplicate push, stale base, revoke over a stale edit, a tombstone refusing a create, cursor past the floor answered with "take a snapshot", a body over the limit, a phone's token refused. Run on PostgreSQL in CI and on the stand-in |
| 2   | Turning a project on, and linking it (built, §10) | The local tables: the logical-id map, the outbox, the cursor. The core's sync loop in the host runtime. Projects only: **Sync** in Project settings with its preview, **Link a synced project** on another machine, **Stop syncing**, **Remove from the server**; `yardsort-host sync on`, `link`, `status`, behind the operator's guard. `ys sync status`, read-only | `sync-link.yaml`: two throwaway profiles on the stand-in, one turning a project on, the other linking it to a clone of the same repository; the hint offered first, nothing linked until chosen; stop and remove                                                        |
| 3   | Approved notes (built, §11)                       | The outbox written in `add_memory` and `change_memory`'s transactions for a synced project's approved and revoked notes; import without echo; revisions; conflicts kept locally with both texts; the Memory view's **Conflicts** section; "from Build box", "source unavailable on this device". Agents' launches get the synced notes as their own                   | `sync-memory.yaml`: two profiles; approve on one, launch an agent on the other and find the note in its first message; both offline, edit the same note on each, reconnect, settle the conflict; revoke on one while the other edits, reconnect: revoked on both        |
| 4   | Workspaces, seen elsewhere (built, §12)           | The outbox in the workspace methods, each in a transaction. On a linked machine, the other host's workspaces under the project, named with their host, branch and task, and never made here. A conflict shown, nothing renamed                                                                                                                                        | `sync-workspaces.yaml`: make, rename and archive on one profile, see each on the other; no worktree, branch or directory appears on the second                                                                                                                          |
| 5   | Checking a branch out here (built, §13)           | **Check out here** on another host's workspace: fetches its branch with Git and makes a workspace for it through `Workspaces::open_branch`, refusing when a local branch of that name has work the remote does not. The record of the other one is untouched                                                                                                          | `sync-checkout.yaml`: push a branch from one profile's workspace, check it out on the other; a second attempt with a diverged local branch refused                                                                                                                      |

Slice 1 comes first because every other slice pushes to it. Slices 3 and 4 depend on 2 and not on
each other. Slice 5 depends on 4. The gate's three checks are slice 3's two-machine runs (converge
and conflict, no resurrection) and slice 4's and 5's (nothing moves unasked).

Each slice brings its tests, failing without it, its guide, its status lines here, in 26 and in 05,
and a local smoke test; slices 2 to 5 also an owner run with this computer and the box through
staging.

## 5 · What the gate needs

- **Offline edits converge or surface conflicts:** slice 3's test and smoke test on the stand-in,
  and the owner's run: this computer and the box both offline from staging, the same note edited on
  each, both reconnected, the conflict shown on both and settled once.
- **Revocations do not resurrect:** slice 1's server test (revoke over a stale base), slice 3's
  two-machine test, and the owner's run: revoke here while the box is offline with an edit pending.
- **No files or processes silently migrate:** slices 4 and 5, and the owner's look at the box after
  each run: no new worktree, branch, directory or terminal there.

Nothing in the gate needs a phone. A Mac as the second desktop is welcome, not required: the box is
an installation as good as any.

## 6 · What only the owner can provide

| What                                          | For           | When                         |
| --------------------------------------------- | ------------- | ---------------------------- |
| Go-ahead for each **staging deploy**          | Slices 1 to 5 | At each slice's owner run    |
| Go-ahead for a **release**, for the box's run | Slices 2 to 5 | Or the box builds the branch |

**Not needed:** no new service, secret, domain or paid plan. The new tables are a migration the server
applies as it starts (28). Staging's Neon free tier has room for records this size.

## 7 · Risks

- **A write that skips the outbox.** The routes are known today; a new one added later could miss it.
  A test lists every `Store` method that writes a synced table and fails when one is added without
  an outbox write or an entry saying why not.
- **A compromised or stale host.** Every linked host can write a project's notes. Revocation wins
  over stale writes, a revoked device's token stops it at once, and imported notes arriving approved
  is the owner's choice (§3, question 2): a note syncs only from a host the person linked.
- **An injected note.** Synced notes reach agents' first messages, as local ones do, framed as notes
  and cited ([19](19-agent-events-stage-5-memory.md)). What a note may contain does not change.
- **The 16 KiB body limit, Neon's suspend, staging's sleeping Machine.** Sync's route has its own
  limit; a pull after a sleep is a normal pull; the pruner runs whenever a Machine is up, and nothing
  depends on it running on time.
- **SQLite and `ys` writing at once.** The outbox is in the same immediate transaction as the
  change, so a crash between them cannot leave one without the other.

## 8 · Later, not in these slices

- Choosing where a task runs from the composer (proposed in [44](44-other-computers-like-local.md)),
  and handing a task to another machine (35 §9).
- Candidate notes, directory and global memory scopes, conversations and outcomes across machines.
- End-to-end encryption of synced records (§3, question 1).
- Account export and deletion, with backups that cannot bring back revoked records (R7).
- Moving an account to another server: an explicit export and import, never forwarding (proposal
  §10.2).

## 9 · Slice 1: the server's records

**Built on 2026-11-14.** No client syncs yet: this is what slices 2 to 5 push to.

**What it adds.**

- **`remote_proto::sync`**: logical ids and op ids (`SyncId`, `OpId`, both lowercase UUID v4s,
  checked on the way in), revisions and cursors, the three kinds (project, note, workspace), an op
  (put, revoke or delete, with its base revision, and a put's schema and body), its four answers
  (applied, conflict with the record as it is, refused with the reason, and one from a newer
  server), and the pull, snapshot and project pages. The limits are there too: 100 ops a push,
  256 KiB a push, 16 KiB a record's body, 200 records a page, 90 days for changes and for
  returning tombstones, 30 for op ids. Its fixtures are in `crates/remote-proto/fixtures/sync-v1/`, apart from the remote
  protocol's: only hosts sync, and every host is Rust, so the TypeScript client does not read them.
- **The server's tables** (migration 10): the records at their latest revision, each account's
  head and floor, the change log, the applied op ids and the links. All cascade from the account,
  so R7's account deletion takes them.
- **What an op comes to** is decided once, by `sync::decide`, for both stores, then written in one
  step: the account's head row is locked, so its ops apply one at a time and cursors come in the
  order they commit.
- **The routes**, host devices with a registered host only, each counted per device (120 a
  minute):

  | Route                                | What                                                   |
  | ------------------------------------ | ------------------------------------------------------ |
  | `POST /v1/sync/push`                 | Up to 100 ops, answered one by one, in order           |
  | `GET /v1/sync/pull?after=`           | The linked projects' records changed after a cursor    |
  | `GET /v1/sync/snapshot`              | A page of one linked project's records, or all, by id  |
  | `GET /v1/sync/projects`              | The account's synced projects, to link one, with hints |
  | `POST /v1/sync/projects/{id}/link`   | Link this host to one                                  |
  | `POST /v1/sync/projects/{id}/unlink` | Unlink it ("Stop syncing")                             |

- **The pruner**, hourly on every instance, in batches of 1000.
- **The relay's hint**, `syncChanged` on a host's link, protocol minor 9, sent once a push to every
  other host linked to a project it changed, and to every instance by signal. The TypeScript
  client reads it and states minor 9 too. A host before minor 9 ignores it, as it ignores any
  notice it does not know.
- **`remote_client::Client`** gains the six calls, for slice 2's loop.

**What building it settled, beyond §3.**

- **Six routes, not three.** Linking is a host's own state, not a record, so it is its own call,
  and the list a person links from is the projects call. Creating a project links the host that
  created it.
- **The server reads no body.** A body is a JSON object of at most 16 KiB, kept as text. Each kind's
  fields are defined by the slice that syncs it: projects in 2, notes in 3, workspaces in 4.
- **Removing a project from the server** deletes every record of it, each with a tombstone and a
  change of its own. Its hosts stay linked until its tombstone is pruned, so each one pulls the
  tombstones and learns of it. It is then no longer offered for linking, and nothing more can be
  written to it.
- **A revoke of a revoked note, and a delete of a deleted record**, answer applied, at the
  revision that did it, and write nothing.
- **Only applied ops are remembered**, including a revoke or delete that changed nothing, so a
  retried one cannot undo a restore made since. A refused one changed nothing, so retrying it is
  decided again. So is an op retried more than 30 days later, which comes to a conflict.
- **A pruned tombstone stays as a marker**: the record's id, kind, project and last revision,
  without a body, kept for as long as the account. Pulls and snapshots no longer return it, but it
  still answers an op, so a create retried after its op id was forgotten is a conflict, not the
  deleted record back (proposal §10.3). A project's marker refuses anything more in that project.
  The markers are small and only grow with deletions; R7's account deletion takes them.
- **A cursor is expired** when it is below the account's floor, which pruning a tombstone raises to
  that tombstone's change, or past its head, as after a server lost its records. Pruning the change
  log does not expire a cursor: a pull reads the records, not the log. A pull reads its floor and
  its records in one snapshot, as a snapshot reads its head and records, so a prune committing
  between them hides nothing.
- **What is logged** is each request's method, path and status, as for every route. A path can hold
  a project's logical id, as other routes' hold a grant's or a host's; never a body, a name or a
  hint.

**Its tests.** `crates/remote-proto` pins each message to its fixture and checks every op on its
own. The server's `tests/records.rs` holds both stores to the same seven cases: revisions, a
duplicate op, a stale base, revoke over a stale edit, a tombstone refusing a stale create or edit,
a project's removal and its hosts hearing of it, two accounts and linked hosts, paging, pruning and
the floor, a create retried past retention, and eight ops racing on one record, of which one
applies. On PostgreSQL, a pull held between its floor and its records while the pruner commits
still returns the tombstone. `tests/sync.rs` goes through
HTTP on both stores: a phone, a computer acting as a client and an unregistered host refused; two
hosts syncing with the hint reaching only the other's link; a push over 256 KiB refused as too large
while one past every other route's 16 KiB goes through, a wrong op refused alone among good ones;
a cursor ahead of the head answered `cursorExpired`; and the per-device limit. CI runs the
PostgreSQL half on Linux.

**Its smoke test.** `.yardsort/tests/sync-server.yaml`, against the stand-in, with
`examples/sync.rs` as two pretend hosts and a phone, then a host of another account. Passed on
2026-11-14 with its steps run by hand, 16 checks and then 4, since a new test file waits for the
owner's approval in the app before `ys test run` runs it. With the stand-in's logs on, nothing of a
body, name or hint was in them.

**On staging**, once the owner says to deploy this branch's server:

```sh
cargo run -p yardsort-server --example sync -- two-hosts https://remote-staging.yardsort.sh \
  --keep /tmp/sync-project --clean
```

It signs three devices in, in the browser, and ends by removing the project from the server and
both hosts from the account. `other-account` with the same `--keep` checks a second account, if
there is one to sign in as.

**Run on staging on 2026-11-15**, after the owner's go-ahead to deploy `main` at `1749f4b` (the
workflow's run 43698600795): `two-hosts` passed all 17 checks and removed what it made. The browser
was signed in to staging already, so the three sign-ins went through without a person. Not run:
`other-account`, for want of a second account.

## 10 · Slice 2: turning a project on, and linking it

**Built on 2026-11-14.** Projects sync, and only projects: the name and the remote hint. Notes and
workspaces are slices 3 and 4.

**What it adds.**

- **The local tables** (core migration 43): `sync_ids`, a record's logical id beside its local id,
  with its project's, the server's revision as last heard (none until the server has it) and
  whether it was just linked; `sync_outbox`, the ops waiting to go up, a refused one held with the
  server's reason; and `sync_state`, the cursor, when a pass last went through, and why the last
  one did not.
- **The rules**, in `yardsort_core::sync` and the store's `sync` methods, shared by the app,
  `yardsort-host` and `ys`: turning on writes the id and the create in one transaction; linking
  writes the id of a project the server already has; settling each answer, taking pulled records
  and snapshots in one transaction with no outbox written; and the project's state as every
  surface shows it (`ProjectSync`: Remote off, off with its preview, or on: reached, being
  removed, refused).
- **The loop**, `remote_host::sync::Syncer`, started with the host's link and stopped with it, so
  the app and `yardsort-host` run the same one. A pass unlinks projects removed from Yardsort here,
  pushes the outbox in batches of 100 ops and 256 KiB, takes a snapshot of each project just
  linked, then pulls after the cursor. It runs at start, after each act here, on the relay's
  `syncChanged`, and every five minutes, and does nothing on the network while nothing syncs. A
  failed pass waits 2 seconds, doubling to five minutes, each with a tenth's jitter.
- **The desktop**: **Sync** in Project settings, with the preview before anything goes up, and
  **Link to a synced project…**, the account's synced projects this host is not linked to, the
  ones whose hint matches first, none chosen. Its six commands answer the project's state, and a
  `SyncChanged` event says when a pass changed something.
- **`yardsort-host sync`**: `status`, `on` (shows the preview, asks), `link`, `stop` and `remove`
  (asks). All but `status` and the preview `on` asks for are behind `pair`'s guard.
- **`ys sync status`**, read-only. The test that covers it also checks that `ys sync` offers
  nothing else.

**What building it settled, beyond §3.**

- **The hint** is the fetch address of `origin`, or of the only remote, with any user and password
  removed. A remote on this disk, by path or `file://`, a Windows drive included, sends none: its
  path is nobody else's. Two hints match when their host and path do, whatever the scheme, port,
  case or `.git`, so a clone over SSH matches one over HTTPS.
- **A project's body** is `{name, remote}` at schema 1. A project's name elsewhere renames nothing
  here; it is what linking shows.
- **Answers settle in one place.** Applied sets the revision; a project's delete applied forgets it
  here. A conflict on a create (the server had it already) takes the server's revision and drops the
  op. A conflict on a delete moves its base to the current revision and tries again; one whose
  current record is deleted forgets the project. Refused holds the op with the reason, shown on the project, and never
  sends it again; ops after it still go up.
- **Pulled and snapshot records** set revisions; a project deleted on the server, by this host or
  another, is forgotten here and the local project kept. A snapshot that no longer has a project
  forgets it too, unless its create has not reached the server yet. A cursor answered
  `cursorExpired` takes a full snapshot, and the cursor moves to its head.
- **Remote off, or another server, clears every sync table.** The logical ids belong to that
  account on that server; turning on or linking again starts over, and what is on the server stays
  for the other hosts.
- **Removing a project from Yardsort** stops it syncing here at the next pass, which unlinks it at
  the server; its records stay for the others.
- **One act at a time.** A pass, a link and a stop take turns, so a stop never forgets a project
  whose create is on its way: it waits, the create settles, and the stop unlinks what the server
  now has. Turning on and removing only queue, and need no turn.
- **Linking** calls the server first and writes the id after. If the write fails, it unlinks
  again, but only a link this call made: one the server had already is left. A synced project
  another project here is linked to is refused before the server is asked.
- **A link the server has and this host does not**, after a link whose answer was lost or a
  Remote turned off while away, is offered for linking like any other; the person still picks the
  project.
- **The window is told** when a pass changed what it shows: a project reaching the server, one
  forgotten or refused, and a pass going through after one that failed.
- **Removing from the server** needs the server to have the project first: until its create has
  gone up, it is refused with a reason saying so.

**Its tests.** `crates/core/src/sync/tests.rs`, 11: hints and matching, drive letters included;
turning on and settling; a conflict on a create; a refused op held and not sent again; removal
rebased after a conflict, and ending; removal by another host; linking once, either way, and the
snapshot after; a snapshot keeping an unsent create; Remote off and a server move clearing
everything; and a pass's state. `crates/remote-host/tests/sync.rs`, 4, against a stand-in server on a
real socket: a stop held behind a create on its way and nothing left linked, a lost link offered and
not undone by a link that failed here, a second clone refused with the first kept, and reaching
the server and recovering from a failed pass both telling the window. `yardsort-host`'s control
test pins which asks are guarded. The
app's `ProjectSyncSection` and `LinkSyncedProjectDialog` tests, 16, and one in Project settings:
the preview shown before turning on, nothing chosen until the person picks, the same-remote mark,
Remove asking first and its "Keep it" respected, Escape closing only the link dialog.

**Its smoke test.** `.yardsort/tests/sync-link.yaml`, two throwaway `yardsort-host` profiles on the
stand-in. The one turns a project on, refused first from input that is not a terminal and then
answered "no", with nothing sent either time; the preview shows the remote without the password the
test's `origin` has, and the password shows nowhere. The other is offered it, matched to a clone
whose `origin` is the SSH address, links nothing by itself, and links it when told. It stops there,
while the first keeps syncing; then the first removes it from the server, it is offered nowhere,
and both keep their projects. Passed on 2026-11-14 with its steps run by hand, all 26 steps; like
`sync-server.yaml`, it waits for the owner's approval in the app before `ys test run` runs it.

**The owner run**, on staging on 2026-11-15, the same deploy as slice 1's: a throwaway
`yardsort-host` profile on this computer and one on the box, each built from `main` (the box's
in a folder of its own, beside its running 0.29 host, which was left alone), signed in through the
browser here, the box's over a forwarded port. The same steps as `sync-link.yaml`: refused from
input that is not a terminal, a "no" sending nothing, the preview without the password, the box
offered the project matched to its SSH clone and linking nothing until told, then linked, stopped
and offered again, while this computer kept syncing; then removed from the server here, offered
nowhere, and both kept their projects. All passed, and neither host's log had the password. Both
hosts were signed out afterwards, which removes them from the account.

## 11 · Slice 3: approved notes

**Built on 2026-11-15.** A synced project's approved and revoked notes sync; candidates and
rejected notes never do.

**What it adds.**

- **Core migration 44**: a note's host of origin, by id and name, and a conflict waiting for the
  person, its id, the other text, the host it is from and when. Nothing for candidates.
- **The outbox in the memory methods' transactions.** `add_memory` and `change_memory` call
  `store::sync::queue_note`, which queues a put for an approved note (a create the first time) or a
  revoke for a revoked one, only in a project that syncs here. Turning a project on queues its
  approved notes; linking one queues this host's own once the snapshot is in. A test reads
  `store.rs` and fails when a method that writes `memory_entries` is not on its list of three
  (`add_memory` and `change_memory` queue; `propose_memory` makes candidates).
- **A note's body**, `{text, author, harness, workspace, origin, from, createdAt, history,
conflict, settles}` at schema 1: the local workspace and run never go. Revoked is the record's
  flag. The oldest history lines are dropped when it would not fit in 16 KiB.
- **Taking notes in**, with nothing queued back. One this host has not got becomes an approved or
  revoked note here, with its history, or is taken as the same note as one here with the same text,
  any case, that does not sync yet. One with an op of this host's still out is left until the
  answer comes.
- **The Memory view's Conflicts section**, before the others: the text that stands, which agents
  get, and the other host's, with **Keep this one**, **Keep _host_'s**, **Write another** and
  **Revoke**, through a new `memory_settle` command. The view reads again on `SyncChanged`. A note
  from elsewhere is cited "from the user, on Build box", in the view and in an agent's first
  message.
- **`yardsort-host memory`**: `list`, `write`, `approve`, `reject`, `revoke`, `restore`, `edit`
  and `settle`, through the running host, which wakes its loop. All of them are behind `pair`'s
  guard, `list` too, since it shows every state whether the project shares its memory or not: on a
  computer with no screen too, no agent approves its own proposal or reads more than `ys memory`
  gives it. `ys` gains nothing.
- **The preview** of turning sync on counts the approved notes that go with it, in the window and
  in `yardsort-host sync on`.

**What building it settled, beyond §3.**

- **Each op is made against the revision the server will have** once the ops ahead of it apply:
  the last heard, and one more for each still out. So an edit made while a create is on its way
  goes up against revision 1.
- **A conflict drops every op still out for the note** and queues one made from the note as it
  is now, against the revision as it is there: those ops were made against a revision that is gone.
- **The conflict is on the server, not only here.** The host whose edit lost takes the text that
  stands and pushes it again with its own text as the note's conflict, with an id. So both hosts
  show the same conflict, and the gate's "shown on both" holds without either being online at
  the same moment. Agents get the text that stands.
- **Settled once.** A settle says which conflict it settles. One that meets a newer revision where
  that conflict is no longer open yields to the settle that got there first; one where it is still
  open goes up again. A new text or a revoke settles a conflict too, since the person saw both
  texts to make either.
- **A revoke there wins** over an edit here: the note is revoked here, and this host's text is
  kept in its history. A revoke here over an edit there goes up again against the current revision.
  A revoke carries no body, so the host taking one in adds its "revoked" history line.
- **A third text while a conflict is open** replaces the one waiting, which is kept in the history.
  The history table's actions are fixed by migration 10, so such lines are "edited" ones with the
  replaced text.
- **"Source unavailable on this device"** has nowhere to go: the Memory view links no run, here or
  elsewhere. A note from another host names that host instead.
- **A pass pushes again** after a project's first snapshot, so a link sends this host's notes at
  once, and pushes at most 20 times, since an answer can queue another op.

**What its review settled** (#262):

- **One op per record in a push.** An op's base is the revision the op before it makes, so it goes
  only once that one is answered (`sync_batch`). Sent together, a second offline edit applied
  against a revision another host had made, a revoke's, and restored the note when the first was
  answered with a conflict.
- **A conflict this host published can itself meet a newer text.** Its own text is then the one it
  set beside the other, not the note's text here, so that is what goes beside the newer one; the
  text that stood here before is kept in the history.
- **`memory list` is behind the guard too.** It shows candidates and rejected notes, and a project's
  notes whether it shares them with its agents or not, which `ys memory` does not.

**Its tests.** `crates/core/src/sync/note_tests.rs`, 6, two stores through a stand-in that keeps
the server's rules: a note arriving approved and cited, its history with it, nothing echoed, a
candidate and a rejected note staying, a candidate approved later going; two edits apart, the same
conflict on both, agents getting the standing text, and two settles at once settled once; a revoke
winning over an edit made meanwhile, the edit in the history, the revoke recorded there, and a
restore going everywhere; linking taking the same text as one note and sending the rest; a
project that does not sync sending nothing; and the list of methods that write notes. Three more
from the review: two offline edits never undoing a revoke made meanwhile, nor meeting each other
rather than the other host's edit, and a published conflict that meets a newer text keeping this
host's; the first two fail without one op per record, the third without its fix. Each of the
settle-once, revoke-wins and same-text rules was checked to fail its test without it.
`crates/remote-host/tests/sync.rs` gains a link sending this host's notes in the same pass, which
fails without the second push. `yardsort-host`'s control test pins every memory ask as guarded. The Memory view's tests, 2 more: both texts shown apart from Approved, each settle
sent as chosen, Revoke, and the view reading again when another computer settles it.

**Its smoke test.** `.yardsort/tests/sync-memory.yaml`, two throwaway `yardsort-host` profiles on the
stand-in, each through a cable (`support/cable.ts`, a TCP proxy that unplugging kills): a note
written on one, refused first from input that is not a terminal, arrives approved on the other,
"from the user, on here", and is in the first message of an agent started there
(`support/record-harness.toml` keeps it); that agent's proposal stays there, with nothing waiting
to go up, and a shell that is not a terminal cannot approve it. Both unplugged, each edits the
note, each has one op waiting; plugged back in, the same text stands on both with the other beside
it; settled on the second with `--other`, both end with that text and no conflict. Then the first
revokes while the second, unplugged, edits: plugged back in, revoked on both, and nothing left to
go up. Passed on 2026-11-15 with its steps run by hand, all 24, and again after its review, all 25 with the step it added, and no note's text was in the
stand-in's log or either host's. Like the others, it waits for the owner's approval in the app
before `ys test run` runs it.

**The owner run**, this computer and the box through staging. It needs no deploy, since the server
reads nothing in a record, but a build of this branch on both, and a way to take each offline from
staging while its host runs: on the box, a firewall rule, as root; here, the app with the network
off. That is the gate's run (§5), and the owner's to make.

## 12 · Slice 4: workspaces, seen elsewhere

**Built on 2026-11-15.** A synced project's worktree workspaces sync, as records only: each other
host lists them and makes nothing for them.

**What it adds.**

- **Core migration 45**: `synced_workspaces`, the other hosts' workspaces as their records say them,
  cleared with the rest of sync's tables and with a project that stops syncing. This host's own
  stay in `workspaces`, mapped in `sync_ids`.
- **The outbox in the workspace methods**, each now one transaction: `add_worktree` (and
  `add_worktree_if_new` through it), `link_task`, and `rename_workspace`, `set_workspace_archived`,
  `forget_workspace`, `revive_workspace` and `remove_worktree` through one `change_workspace`.
  Turning a project on and linking one queue its workspaces as they do its notes. A test reads
  `store.rs` and fails when a method that writes `workspaces` or `workspace_tasks` is not on its
  list.
- **A workspace's body**, `{name, branch, baseBranch, archived, tasks, origin, host}` at schema 1,
  each task its source, repo, key, link and title. Never its path, preparation, sessions or outcomes.
- **On the desktop**, **on other computers (n)** under each project in the sidebar, folded, from a new
  `project_sync_workspaces` command, read again on `SyncChanged`: each one's name, computer, branch
  and tasks, and _archived_, with no menu and nothing to open. **`ys sync status`** lists them under
  the projects. The turn-on preview counts the workspaces that go up.
- **The loop looks at the outbox here every five seconds** while it waits after a good pass: one
  read of the database, nothing on the network. So a workspace `ys workspace new` makes, in a process
  of its own that tells the host nothing, goes up within seconds rather than at the next five
  minutes.

**What building it settled, beyond §3.**

- **Only the host a workspace lives on writes its record.** So two hosts never edit one, and the
  conflict §3 allowed for cannot arise between hosts: a conflict on this host's own record is one
  that went on without it hearing, and what it has here is put again against the revision there.
  Nothing is renamed, moved or ended for a record, on any host.
- **Only worktrees.** The project's own checkout is not a workspace elsewhere: every host has its
  own.
- **Gone is deleted.** A workspace removed or forgotten here is deleted there, leaving a tombstone.
  One brought back while its delete waits to go up is a new record; one that never reached the
  server is only forgotten here.
- **A record of this host's that no workspace here maps to**, after sync was stopped and the project
  linked again, is deleted rather than shown as another host's: this host's workspaces go up anew.
- **Renaming and archiving have no command line.** `ys` makes and deletes workspaces; renaming and
  archiving are the window's. So the smoke test makes and deletes on a computer with no screen, and
  the core's tests cover renaming, archiving and task links.

**What its review settled** (#265):

- **What an earlier version left unsent goes up.** A project already syncing before this slice has
  workspaces with no record, and one syncing before slice 3 has approved notes with none. Each pass
  first queues them (`Store::backfill_sync`); one queued or sent is not queued again, so after the
  first pass it queues nothing. A project just linked waits for its snapshot, which does the same.
- **A full snapshot drops the other hosts' workspaces it no longer has.** A host away past retention
  never sees a deleted workspace's tombstone, which is pruned from snapshots; without this, the
  workspace stayed listed for good.
- **`sync-memory.yaml`'s proposal step** now checks that the proposal is not on the first host once
  the second's outbox is empty, rather than that nothing is waiting to go up: the agent's workspace
  syncs now, and waits a moment like any other record.

**Its tests.** `crates/core/src/sync/workspace_tests.rs`, 7 with two from the review (a profile as
slice 3 left it sending its old workspace and note once, and a full snapshot dropping a workspace
whose tombstone was pruned, which fails without the fix), two stores through the notes' stand-in,
which now applies deletes too: made with a task, renamed, archived and deleted on one, seen each
time on the other with nothing made there and nothing echoed; the project's own checkout and a
forgotten workspace not sent, and one revived before its delete went up becoming a new record;
turning on and linking sending each host's own, and stopping hiding the others'; a record of this
host's it no longer maps deleted, not shown; and the list of methods that write workspaces. The
revive and stray-record rules were each checked to fail their tests without them.
`crates/remote-host/tests/sync.rs` gains a workspace written by another process going up while the
loop waits, which fails without the look at the outbox. `ys`'s sync test lists another computer's
workspace and checks no workspace row was made. The sidebar's test: the group folded, only under
the project that has some, its rows with nothing to press, and gone when a pass says so.

**Its smoke test.** `.yardsort/tests/sync-workspaces.yaml`, two throwaway `yardsort-host` profiles
on the stand-in: a workspace made on the first with `ys workspace new` is listed on the second by
`ys sync status` with its computer and branch; the second's workspaces, branches, worktrees and
worktree folder are byte for byte what they were before; deleted on the first, it is gone from the
second, and the second is still unchanged. Passed on 2026-11-15 with its steps run by hand, all
15, and no workspace's name was in the stand-in's log or either host's. Like the others, it waits
for the owner's approval in the app.

**The owner run**, this computer and the box through staging: needs no deploy, only a build of this
branch on both, and the owner's look at the box afterwards (§5).

## 13 · Slice 5: checking a branch out here

**Built on 2026-11-15.** Another computer's workspace can be made a workspace here too, of this
computer's own, and only when the person asks.

**What it adds.**

- **`yardsort_core::sync::checkout`**: `prepare_branch` fetches the branch from the project's remote
  into its remote-tracking ref and has a local branch at it; `check_out_here` finds the other
  computer's record, prepares its branch and opens a workspace on it with `Workspaces::open_branch`,
  as any other. `Git::branch_move` moves a branch that nothing has checked out.
- **On the desktop**, **Check out here** on each row of **on other computers**, shown on hover and
  focus, through a new `project_sync_check_out` command; the projects are read again and the new
  workspace entered. A refusal is said under the row.
- **`yardsort-host sync checkout <project> <workspace>`**, by the name `ys sync status` shows or the
  start of its id, behind `pair`'s guard. `ys` gains nothing: a checkout is a person's act at that
  computer, so no agent makes one there (§1, the third check of the gate).

**What building it settled, beyond §3.**

- **The branch comes from the remote, never the server.** The server keeps records, not commits, so
  a branch the other computer has not pushed is refused, saying to push it from there first, with
  nothing changed here.
- **The remote** is the one a push would go to: `origin`, or the only one.
- **A branch of that name here** is used as it is when at the remote's, moved up when only behind,
  and refused with a count of its own commits when it has any: moving it would lose them. Also
  refused: a branch checked out in a worktree, and a workspace here on it already.
- **The new workspace is this computer's own**, synced as its own, and the other computer's record is
  untouched: both are listed on a third computer, each as its computer's.

**Its tests.** `crates/core/src/sync/checkout/tests.rs`, 4, with a bare remote, the other
computer's clone that pushes and this computer's clone: a pushed branch checked out as a workspace,
at the pushed commit, tracking the remote's, the other's record still listed, and a second attempt
refused; a branch here with a commit of its own refused, left where it was, no worktree made, which
fails without the check; a branch here that is only behind moved up; a branch never pushed refused
with nothing made. `yardsort-host`'s control test pins `SyncCheckout` as guarded. The sidebar's
test: one button on each row and nothing done before it is pressed, a refusal shown under the row,
and the new workspace listed after.

**Its smoke test.** `.yardsort/tests/sync-checkout.yaml`, two throwaway `yardsort-host` profiles
on the stand-in, each with a clone of one bare repository in the run's folder: the first makes a
workspace, commits and pushes; the second lists it, with no branch or workspace made for it; checking
out is refused from a shell that is not a terminal, then done at one, at the first's commit and
tracking the remote's, with the first's still listed as its; with that workspace deleted and a commit
of the second's own on the branch, checking out again is refused and the branch is where it was.
Passed on 2026-11-15 with its steps run by hand, all 18. It waits for the owner's approval in the
app.

**The owner run**, this computer and the box through staging, with a repository both can push to
and fetch from: needs no deploy, only a build of this branch on both, and the owner's look at the
box after (§5).
