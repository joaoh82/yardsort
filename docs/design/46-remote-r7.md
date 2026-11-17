# Remote, R7: the managed launch

**Status:** planned on 2026-11-16, with the questions in §3 open for the owner. Slice 1, exporting
an account, built on 2026-11-17 (§9), on question 8's lean. Slice 2, deleting an account, built the
same day (§10), on question 5's lean.

This is the plan for the last phase of the proposal ([25](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md)
§13): what it has to show, what it starts from, the decisions it needs, and the slices in the order
they can be built. Like [42](42-remote-r6.md) for R6, it becomes the record of the slices as they
are built. R0 to R6 are done ([26](26-remote-r0-r1.md) §8).

R7 differs from the phases before it: most of what it needs is the owner's to choose or provide,
not code. Production accounts, paid plans, prices and quotas, a payment provider, store listings
and submissions, terms and a privacy policy, and counsel's reading of the licenses are all the
owner's (proposal §16, "owner-authorized only"). So the slices are in two groups: those that can be
built and proved on staging now, and those that wait on a decision in §3.

## 1 · What the phase has to show

**The gate** (proposal §13):

- **a backup restore drill**: a database restored from a backup, and nothing it brings back
  reopens what was closed after it was taken: no revoked device signs in again, no deleted account
  comes back;
- **load and isolation tests**: the relay and sync at a known load, within the proposal's budgets
  (§14), and no route answering one account with another's records;
- **a deployment rollback**: production put back on the release before, on request, in minutes;
- **the store distribution gates**: the phone app published on the App Store and Google Play, with
  what the stores require of an app with accounts.

In R7, a person can:

- sign up for the managed service, or be invited to it, and see what their plan allows;
- export everything the server holds of their account, as a file they keep;
- delete their account, from the desktop, the phone or a web page, and have every device signed
  out and every record gone;
- install the phone app from the App Store or Google Play;
- see whether the managed service is up, and what is being done when it is not, at
  `status.yardsort.sh`, from outside the service, linked from the app and the phone when they
  cannot reach it;
- keep everything local working, whatever happens to their account or their payment.

**Not in R7**: teams and shared accounts, moving an account to another server (export is one half
of it; import is later), end-to-end encryption of synced records ([42](42-remote-r6.md) §8), and
buying from inside the phone app (§3, question 4).

## 2 · Where it starts

Checked against `main` at `c6958a1` on 2026-11-16.

**The server** (`crates/server`, AGPL-3.0): accounts by identity-provider subject and never an
email (`migrations/0001_accounts_and_devices.sql`), devices and their hashed credentials, hosts and
grants, the relay with routing across Machines (`fly-replay`, [28](28-remote-infrastructure.md) §6),
push routes to APNs and FCM (`push.rs`), notices, and sync's records, change log and markers
(`0010_sync.sql`). Ten migrations, run as it starts. Every per-account table cascades from
`accounts`, and every query filters by account ([42](42-remote-r6.md) §2).

- **Limits it has**: 50 devices an account (`MAX_DEVICES_PER_ACCOUNT`), devices per host on the
  relay, queued frames, pending tickets, the body limits (16 KiB, 256 KiB for sync), and rate gates
  shared across instances (`0004_rate_limits.sql`).
- **The one operator command**: `yardsort-server revoke-account --subject …`, which signs every
  device of one account out and signals every instance (`main.rs`). There is no route for a person
  to export or delete their account, and nothing meters use.
- **Backups**: staging's database is Neon's free plan; production's is planned as Fly Managed
  Postgres ([28](28-remote-infrastructure.md) §2). Neither has been restored, and nothing outside
  the database records what was revoked or deleted, so a restore would bring both back.

**Where it runs**: staging, one Machine in `fra`, ZITADEL Cloud's free instance, deployed from
Actions on request ([28](28-remote-infrastructure.md) §5). Production is an empty Fly app
(`yardsort-remote`) and a planned ZITADEL Pro instance; its Fly config and its protected GitHub
environment come with that instance. There is no rollback in the deploy workflow beyond Fly's own
release history.

**The managed service's own code** does not exist. [30](30-open-source-and-managed.md) settles how
it will: the public server runs unmodified, and anything only the managed service has (billing,
plans, the support console, abuse rules) is a private program, `yardsort-cloud`, behind an
interface documented here; with it unset, the server allows everything up to its own limits, as a
self-hosted one does (30 §4).

**The phone app** (`apps/mobile`, MPL-2.0): internal TestFlight builds by a workflow, signed by
hand ([31](31-remote-r3.md) §1); Android in an emulator only, with generated permissions still to
block before any store build (31 §16). No store listing, privacy label or data safety form, and no
account deletion in the app.

**Local operation** is account-free and stays so: a billing failure or a deleted account never
stops a local agent or deletes local work (proposal §9, §12).

## 3 · Decisions

**Settled before this plan**, kept as they are: the public server runs unmodified in production
and the private parts live in `yardsort-cloud` (30 §1, §3); entitlements are checked on the server,
never trusted from a client, and a lapse never deletes local work, ends a local agent or blocks an
export (proposal §12, 30 §4); store submission, paid services and production deployment each need
the owner's own go-ahead (proposal §15).

**For the owner to settle.** Each has the plan's lean, which the slices assume until told
otherwise.

1. **When production starts, and ZITADEL Pro with it** (28 §7). Lean: at slice 5, for an invited
   beta, before billing exists. Not chosen: at launch only, which leaves the drills on staging
   alone.
2. **Who can sign up** (28 §7). Lean: invited only until billing works; then sign-up open, with
   ZITADEL's self-registration and email verification. Not chosen: open from the start, which
   needs abuse rules first.
3. **Plans, quotas and the payment provider.** The proposal chooses none (§12) and says to measure
   first. Lean: slice 4 meters relay traffic, streams, sync storage and active devices on staging
   and the beta; the owner sets plans from that, and the provider is one with hosted checkout and
   webhooks to `yardsort-cloud`. The server never holds payment data or prices.
4. **Buying in the phone app.** Lean: no. The phone app shows no prices and sells nothing; plans
   are bought on the web. Store rules on linking out to a purchase change and differ by country;
   read them, with counsel, before the app says anything about payment.
5. **Confirming a deletion.** Lean: a sign-in completed within the last five minutes on the device
   that asks, with the provider asked to sign in again (`prompt=login`), then a typed confirmation.
   Not chosen: a confirmation alone, which anyone holding an unlocked phone could give.
6. **Where the record of revocations and deletions lives**, so a restore cannot undo them (slice
   3). Lean: an object-store bucket in the same Fly organization, append only, written as each one
   happens and read by `yardsort-server reconcile` after a restore. Not chosen: a second database,
   which is backed up and restored the same way.
7. **When the entitlements service cannot be reached** (30 §4). Lean: what was allowed stays
   allowed for a grace period of 24 hours, and only new things wait.
8. **What an export holds.** Lean: everything the server keeps of the account that a person put
   there or that names them: the account's own row, devices (names, kinds, when signed in, never a
   credential), hosts, grants, push routes (platform only, never the token), and every live synced
   record with its body. Not tombstones or markers, which hold no content, and not the change log.
9. **Terms, a privacy policy and counsel.** The owner's, with counsel: the terms of the managed
   service, the privacy policy the stores link to, and the license questions 30 and 27 left for
   R7 (ZITADEL inside a paid product, and the server's AGPL). The plan's part is to say exactly what
   data exists and where, for those documents (slice 8).
10. **Backup retention and the restore point** of production's database, and whether logs are kept
    and for how long. Lean: the provider's default point-in-time window, written down in the
    runbook, and request logs kept seven days.
11. **Where the status page runs** (slice 10). It has to stay up when what it reports on is down,
    so not on Fly beside the server, and not on the website's Vercel project, which cannot check
    anything by itself. Lean: a hosted status service, with checks from outside, incidents the
    owner posts, and subscriptions by email, at `status.yardsort.sh` by a CNAME on Cloudflare.
    Not chosen: a page built by GitHub Actions on a schedule, which is free but checks every few
    minutes at best and goes down with GitHub.

## 4 · Slices

| #   | Slice                                    | What it adds                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Waits on               | Its proof                                                                                                                                                                                       |
| --- | ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Exporting an account                     | `GET /v1/account/export`, paged, for a phone or a computer the person signed in; on a headless host, `yardsort-host account export` behind the operator's guard. **Export my account's data…** in Settings → Remote, and in the phone's Settings, saved or shared as a JSON file, with what it holds said before it is made                                                                                                                                                            | Question 8's lean      | The server's tests on both stores: everything of the account in it, nothing of another account, no credential or push token; a smoke test on the stand-in exporting from each client            |
| 2   | Deleting an account                      | `POST /v1/account/delete` after a fresh sign-in: the account's rows cascade, every instance closes its links and drops its devices, push routes go, sync's records and markers go. **Delete my account…** on the desktop and the phone, saying what goes and what stays (everything local); a page on the website for the stores. Every client told, kept working                                                                                                                      | Question 5's lean      | Tests that every per-account table is empty after it and another account untouched; a smoke test with a host, a phone and a computer signed in, each told, the host's agents running throughout |
| 3   | A restore that cannot resurrect          | Each revocation and deletion appended to a journal outside the database as it happens; `yardsort-server reconcile` replays it after a restore, and refuses to serve until it has. The runbook's restore section                                                                                                                                                                                                                                                                        | Question 6             | **The restore drill** on staging: a branch of its database from before a revoke and a deletion, restored, reconciled; the device stays out and the account stays gone                           |
| 4   | Entitlements and metering                | The seam of 30 §4: `YARDSORT_ENTITLEMENTS_URL`, asked before a device past the plan, a relay stream, sync storage; cached; unset allows everything. Per-account daily counters of relay bytes, peak streams, sync bytes and active devices, and `yardsort-server usage`. A stand-in entitlements service for tests                                                                                                                                                                     | Question 7's lean      | Tests of allow, refuse, cache and an unreachable service, and that no refusal ever blocks reading, revoking, exporting or deleting; counters checked against a known load                       |
| 5   | Production, and rollback                 | `crates/server/fly/production.toml`, a `production` GitHub environment with a reviewer that deploys from `main` only, two Machines with `fly-replay`; the deploy workflow's **rollback** input, putting back a release by its image; health checks and an uptime check                                                                                                                                                                                                                 | Questions 1 and 10     | **The rollback drill**, on staging then production: deploy, roll back, every device and host back within a minute                                                                               |
| 6   | Load and isolation                       | A load tool (`crates/server/examples/load.rs`) driving hosts and devices through the relay and sync at a set rate, reporting the proposal's budgets; a test that walks every route and asks it with another account's ids                                                                                                                                                                                                                                                              | Slice 5 for production | **The load test** on staging, with the owner's go-ahead, within §14's budgets; **the isolation test** in CI on both stores                                                                      |
| 7   | Runbooks                                 | `docs/operations/`: deploying, rolling back, restoring and reconciling, revoking or deleting an account for someone, what to look at first in an incident. Internal detail stays in `yardsort-cloud` (30 §3)                                                                                                                                                                                                                                                                           | Slices 3 and 5         | Each drill run by following its runbook, by someone who did not write it                                                                                                                        |
| 8   | The stores                               | Release builds for both stores from the workflows; Android's extra permissions blocked; the privacy label and data safety form, written from a list of exactly what each part keeps; deletion in the app and on the web (slice 2); an accessibility pass; store assets                                                                                                                                                                                                                 | Questions 4 and 9      | **The store gates**: both apps accepted, after the owner submits them                                                                                                                           |
| 9   | Sign-up and billing, in `yardsort-cloud` | Invitations, then sign-up; plans and their quotas; the payment provider's hosted checkout and webhooks; the grace period; a support console. All private, answering slice 4's interface                                                                                                                                                                                                                                                                                                | Questions 2, 3 and 9   | A paid plan on the beta changes what slice 4 allows, and a lapsed one changes nothing local                                                                                                     |
| 10  | The status page                          | `status.yardsort.sh` on the service question 11 chooses: checks from outside of `remote.yardsort.sh`'s `/health`, its relay's WebSocket handshake and `auth.yardsort.sh`'s sign-in page, and incidents posted by the owner. The server's `YARDSORT_STATUS_URL`, given in `GET /v1/server` and remembered by each client, so a self-hoster names their own or none; the desktop and the phone link to it when they cannot reach the server. The incident runbook (slice 7) starts there | Question 11, slice 5   | Staging's own checks first: a stopped Machine shown down on the page within the check interval and up again after, the page answering throughout; a client cut off naming the remembered page   |

Slices 1, 2, 4 and 6's isolation test can start now, on the stand-in and staging, and each is
useful to a self-hoster too. Slice 3 waits only on where its journal lives. Slice 5 starts
production, and everything that runs there waits on it. Slice 9 is the only one outside this
repository, and slice 10's page is the only part that runs on a service of its own.

Each slice brings its tests, failing without it, its guide, its status lines here, in 26 and in
05, and a smoke test; those that touch the server also a staging run, with the owner's go-ahead
for each deploy.

## 5 · What the gate needs

- **Backup restore drill**: slice 3, on staging, then once on production before launch.
- **Load and isolation tests**: slice 6, the load test on staging with the owner's go-ahead and the
  isolation test in CI.
- **Deployment rollback**: slice 5's drill, on staging and on production.
- **Before sign-up opens**, though not part of the proposal's gate: slice 10's status page
  (28 §2 puts it before the public beta).
- **Store distribution gates**: slice 8, and the owner's submissions; slice 2, since both stores
  require deletion of an account from the app or the web.

## 6 · What only the owner can provide

| What                                                                    | For           |
| ----------------------------------------------------------------------- | ------------- |
| The answers to §3                                                       | Every slice   |
| ZITADEL Pro, `auth.yardsort.sh`, production's database and Fly Machines | Slice 5       |
| The journal's bucket, and its credentials straight into Fly             | Slice 3       |
| Go-ahead for each staging and production deploy, and the load test      | Slices 1 to 6 |
| Store accounts' listings, submissions and review answers                | Slice 8       |
| Terms, privacy policy and counsel's reading                             | Slices 8, 9   |
| The payment provider's account, plans and prices                        | Slice 9       |
| The status service's account, and the `status` record on Cloudflare     | Slice 10      |

**Not needed** for slices 1, 2, 4 and 6's isolation test: no new service, secret or paid plan.

## 7 · Risks

- **A deletion that misses a table.** A later migration could add a per-account table without the
  cascade. A test lists every table with an `account` column and fails when one does not cascade
  from `accounts` or is not named in the deletion.
- **A restore before reconciling.** A restored server that serves before replaying the journal
  lets a revoked device back for minutes. `reconcile` marks the database as reconciled; a server
  that finds the journal newer than its last reconcile refuses to serve until it has.
- **An entitlements refusal that strands someone.** Every refusal is for something new; reading,
  revoking, exporting and deleting are never asked about, and the clients say which plan limit it
  was, not "error".
- **Store rules.** They change, and differ by country, in particular on payment links; question 4
  keeps payment out of the app until they are read.
- **The export as a leak.** It holds every synced note and workspace name. It is made only for a
  device the person signed in, said before it is made, never logged, and never includes a
  credential or a push token.
- **Cost.** Production is about US$150 a month before relay traffic (28 §3); slice 4's counters
  say what traffic costs before plans are priced.

## 8 · Later, not in R7

- Importing an export into another server, so an account can move (proposal §10.2).
- Teams, shared projects, and an admin for them.
- End-to-end encryption of synced records.
- Buying a plan inside the phone app.
- Published uptime targets, and credits when they are missed. The status page itself is slice 10.

## 9 · Slice 1, exporting an account

Built on 2026-11-17, on question 8's lean, which the owner has not answered yet.

**The server.** `GET /v1/account/export` (`remote_proto::export`), listed as `account.export` in
`GET /v1/server`'s capabilities, for any device the account has signed in, phone, computer or host,
and for its own account only: every read is by the caller's account, never by an id a request
names. The first page carries the account: its issuer, subject and when it was made; its devices,
signed out ones included, as `GET /v1/devices` shows them; its hosts, as `GET /v1/hosts`; every
grant, as the grant pages show them, whatever became of it; its push routes, by device and
platform with when each was saved, never the token; and its notification settings. Every page
carries synced records by id, those with a body only, a revoked note's included with its flag:
tombstones, pruned markers and the change log stay out, as do notices on their way to a phone,
which name runs and workspaces by id and lapse within days. A page holds at most 200 records and
about 512 KiB of their bodies, at least one, since a page of 200 at the 16 KiB a record may hold
would be over the phone client's 1 MiB answer. Pages are under the per-device `sync` limit. Three
store methods, `account_row`, `push_routes` and `exported_records`, and `GrantFilter::All`, on both
stores; the memory store now keeps when each account was made, and when each push token was saved.

**The clients.** Each puts the pages together as one `AccountExport`, the format 1 file, with the
server's id and name and when it was made, and checks that each page moves on. A page the
server refuses as asked too often, its limit shared with sync, is asked for again after a wait
that doubles each time, six times at most, and the pages before it are kept.

- The desktop: **Settings → Remote → Your account → Export my account's data…** says what the file
  holds; **Choose where to save…** opens the system's save dialog (the new `dialog:allow-save`
  capability), and only then is the server asked (`remote_export`). It is offered while this
  computer is signed in.
- The phone: **Settings → Your account → Export my account's data**, a screen saying what the file
  holds, and **Export**. On iOS the share sheet, from a copy in the cache folder deleted once it
  closes; on Android a folder the person picks. No new native module.
- A computer with no screen: `yardsort-host account export [--out FILE] [--yes]`, behind the guard
  as `pair` is, since the file holds every synced note. It says what the file holds and asks; an
  existing file is kept.

The desktop and the host write through `remote_host::export::save`: beside its place, then put
there whole, readable by its owner only: mode 600 on Unix, and on Windows a protected access
list with one entry, this user's, set before anything is written, so a folder others may read
gives the file nothing. A server too old to export answers `not_found`, which each
client words as the server not exporting yet.

**Proof.** The store test (`tests/records.rs`), on memory and PostgreSQL: one account's row, push
routes, grants and records, by id and a page at a time, a revoked note in and a deleted workspace
out, nothing of another account's. The HTTP tests (`tests/export.rs`), on both: an account of 251
records over two pages, the first with the account and the second without; no credential and no
push token in either; the host may export too; another account's records never; a signed-out
device and a request without one refused; a page of large notes kept under the byte bound. The
desktop's, the phone's and the TypeScript client's own tests, and `remote_host::export`'s: whole,
kept unless replacing, and mode 600. The smoke test `account-export`: one account on the real
window, a computer with no screen and a pretend phone against the stand-in; the host's export
refused from a shell that is not a terminal and saved from one; the window's section read, then the command
the save dialog's answer goes to asked with a place, since a page cannot answer that dialog; the phone's every page; each file with the
account, both computers and a synced note, readable by its owner only, and none of the sign-ins
those computers keep.

## 10 · Slice 2, deleting an account

Built on 2026-11-17, on question 5's lean, which the owner has not answered yet.

**Confirming.** A sign-in started with `purpose=confirm` (`remote_proto::account::LoginPurpose`)
sends the browser to the provider with `prompt=login` and `max_age=0`, so the person signs in
there and then, whatever session the browser has. Its ID token must carry `auth_time` within the
last five minutes (`FRESH_SIGN_IN_MS`), or the browser goes back with an error and no code. It
makes no account: a person with none here gets an error too. Its handoff is marked `confirm`, lapses
five minutes after that sign-in, and is good for a deletion only: `POST /v1/token` refuses it, and a
deletion refuses an ordinary sign-in's. Clients ask for the typed confirmation, `delete`, before
they start that sign-in, so the code comes back to a deletion that goes at once.

**The server.** `POST /v1/account/delete {code, verifier}` (`remote_proto::delete`), listed as
`account.delete`, with the caller's access: the code is taken in the same step it is read, a wrong
verifier spends it, and it has to be the caller's own account (`forbidden` otherwise, and nothing
goes). It answers how many devices, hosts and synced records went. In one transaction, with the
account's row locked: the hashes of its devices' access and refresh credentials go into
`deleted_credentials` with when each would have lapsed, linked to nothing, then the row is deleted
and everything else cascades. Then every instance rechecks its links, so the account's relay
sockets close at once. A device that asks afterwards, with access or a refresh credential, is told
`unauthenticated` with the new `reason: accountDeleted` (`remote_proto::ErrorReason`, absent from
an older server, read as `unknown` from a newer one), and words saying nothing on it was deleted.
Migration 0011 adds the `confirm` columns and `deleted_credentials`. Two store methods,
`find_account` and `delete_account`, and `deleted_credential`, on both stores. The log says that
an account was deleted, with its counts and nothing that names it.

**The web page**, for the stores and for a person with no device to hand: `/account/delete` on the
server itself (`crates/server/src/delete_page.rs`). It says what goes and what stays; **Sign in
again to delete** keeps a verifier in a `HttpOnly`, `SameSite=Lax` cookie scoped to those pages and
starts a confirming sign-in back to `/account/delete/confirm`, which asks for `delete` typed, and
the form deletes the account signed in as. A post from another site carries neither the cookie nor
this server's `Origin`, and deletes nothing. The pages load nothing and run no script. The website
has a guide page for the stores' links, [Deleting your account](../guide/delete-account.md).

**The clients.** Each says what goes and what stays, takes `delete` typed, then the browser:

- The desktop: **Settings → Remote → Your account → Delete my account…**
  (`remote_delete_account`, `remote_host::delete`), waiting for the browser until **Cancel**. It
  then forgets the account here: the link, the row and the kept sign-in, and nothing remembered
  for signing in again, since the host is gone too.
- The phone: **Settings → Your account → Delete my account**, a screen of its own. The profile goes
  from the phone, and the notice says so. A confirmation the server refuses does not sign the
  phone out.
- A computer with no screen has no command for it: the guide sends its person to the other two or
  the page.

Every client words `accountDeleted` as its account having been deleted, not as being signed out:
the desktop's and the host's link line, and the phone's notice. Local work is never touched.

**Proof.** The store tests (`tests/records.rs`), on both stores: a confirming sign-in and handoff
kept as such, `find_account` making nothing; an account with a phone, a push route, a host, a
grant, an offer, a link place, preferences and synced records deleted, nothing of it readable
afterwards, its credentials' hashes kept until each would have lapsed, and another account's
holdings the same before and after. The HTTP tests (`tests/delete.rs`), on both: deleted from a
phone after the provider was asked to sign in again; the phone, the host and a computer each told
why, by access and by refresh; another account untouched; signing in again starts empty. Refused:
an ordinary sign-in's code, a confirming code at `/v1/token`, a sign-in again as someone else, a
wrong verifier (which spends the code), a stale or missing `auth_time`, a person with no account,
a malformed request and one with no access. The page deletes in the browser that started it and
not from another, another site, or with the confirmation mistyped. And on PostgreSQL, every table
holding anything of an account cascades from it, and no row naming the account, its devices or its
hosts is left in any table after a deletion. The desktop's, the phone's and both clients' own
tests. The smoke test `account-delete`: the window, a computer with no screen running a shell and
a pretend phone on one account against the stand-in; deleted from the window through the browser;
the computer with no screen and the phone each told; the shell still running; and signing in
again finding an empty account.
