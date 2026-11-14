# Remote, R6: durable sync

**Status:** planned on 2026-11-13. The owner took the defaults for the four questions in §3 the same
day. Slice 1, the server's records, was built on 2026-11-14 (§9); its tests and its local smoke test
passed, and its staging run waits for the owner's go-ahead to deploy.

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

| #   | Slice                                | What it adds                                                                                                                                                                                                                                                                                                                                                          | Its smoke test                                                                                                                                                                                                                                                          |
| --- | ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | The server's records (built, §9)     | `remote_proto::sync`: logical ids, op ids, revisions, cursors, ops and their answers, with fixtures. The server's tables, `Records` methods on both stores, the three routes, the pruner, the relay hint (minor 9). No client                                                                                                                                         | The server's own: two accounts, duplicate push, stale base, revoke over a stale edit, a tombstone refusing a create, cursor past the floor answered with "take a snapshot", a body over the limit, a phone's token refused. Run on PostgreSQL in CI and on the stand-in |
| 2   | Turning a project on, and linking it | The local tables: the logical-id map, the outbox, the cursor. The core's sync loop in the host runtime. Projects only: **Sync** in Project settings with its preview, **Link a synced project** on another machine, **Stop syncing**, **Remove from the server**; `yardsort-host sync on`, `link`, `status`, behind the operator's guard. `ys sync status`, read-only | `sync-link.yaml`: two throwaway profiles on the stand-in, one turning a project on, the other linking it to a clone of the same repository; the hint offered first, nothing linked until chosen; stop and remove                                                        |
| 3   | Approved notes                       | The outbox written in `add_memory` and `change_memory`'s transactions for a synced project's approved and revoked notes; import without echo; revisions; conflicts kept locally with both texts; the Memory view's **Conflicts** section; "from Build box", "source unavailable on this device". Agents' launches get the synced notes as their own                   | `sync-memory.yaml`: two profiles; approve on one, launch an agent on the other and find the note in its first message; both offline, edit the same note on each, reconnect, settle the conflict; revoke on one while the other edits, reconnect: revoked on both        |
| 4   | Workspaces, seen elsewhere           | The outbox in the workspace methods, each in a transaction. On a linked machine, the other host's workspaces under the project, named with their host, branch and task, and never made here. A conflict shown, nothing renamed                                                                                                                                        | `sync-workspaces.yaml`: make, rename and archive on one profile, see each on the other; no worktree, branch or directory appears on the second                                                                                                                          |
| 5   | Checking a branch out here           | **Check out here** on another host's workspace: fetches its branch with Git and makes a workspace for it through `Workspaces::open_branch`, refusing when a local branch of that name has work the remote does not. The record of the other one is untouched                                                                                                          | `sync-checkout.yaml`: push a branch from one profile's workspace, check it out on the other; a second attempt with a diverged local branch refused                                                                                                                      |

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

- Choosing where a task runs from the composer, and handing a task to another machine (35 §9).
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
