# Restoring the server's database

What to do when the Yardsort server's PostgreSQL has to be put back to an earlier point: a bad
migration, a deleted table, a lost volume. The part that is easy to miss is the last step. A
database restored from a backup has everything that was closed after the backup open again: a
device its person revoked can sign in, a host removed is back, an account deleted is back. The
server's journal records every one of those outside the database, and `yardsort-server reconcile`
makes each of them again. Until it has run, every server on that database refuses every request
with 503, its health check included.

The design is [46 §12](../design/46-remote-r7.md#12--slice-3-a-restore-that-cannot-resurrect).
Running the server yourself? The short version is in
[the self-host recipe](../../crates/server/self-host/README.md#restoring).

## Before

- **The journal is on.** The server's log says `revocations and deletions are journaled in …` as
  it starts. If it says `no journal`, a restore cannot be reconciled: what was closed since the
  backup stays open, and each person has to revoke their devices again. Turn it on first
  (`YARDSORT_JOURNAL_DIR`, or a bucket: see `crates/server/src/config.rs`).
- **The journal is not restored.** Never put the bucket or the directory back to an earlier
  point with the database. It only ever grows, and nothing of the server deletes from it.
- **Note the time** to restore to, in UTC.

## Restoring

1. Restore the database to that time with the provider's own tool: a point-in-time restore on Fly
   Managed Postgres, a branch from that time on Neon, `pg_restore` of a dump on a server of your
   own. Keep the database it replaces until the restore is checked.
2. Point the server at the restored database, or restart it if the restore was in place. On Fly,
   `fly secrets import` with the new `DATABASE_URL` (from stdin, so it is not in your shell's
   history) restarts the Machine, then waits for its health check, which answers 503 until step 3:
   it times out after five minutes and says the update failed. The Machine is running the new
   secret all the same; carry on. Each
   instance checks the journal before it serves, and finds the records behind it. Its log says
   `the journal holds revocations or deletions these records do not`, and `GET /health` answers
   503 from then on. That is expected.
3. Run `yardsort-server reconcile` where `DATABASE_URL` names the restored database and the
   journal's settings are set, the same environment the server runs with:
   - On a server of your own: `docker compose exec server yardsort-server reconcile`.
   - On Fly: on the running Machine, which has the app's secrets. The image has no shell, so name
     the binary:

     ```sh
     fly ssh console -a yardsort-remote-staging -C "/usr/local/bin/yardsort-server reconcile"
     ```

   It prints what it sent and replayed:

   ```text
   Reconciling the records with the bucket yardsort-journal-staging at https://fly.storage.tigris.dev/.
   Replayed 3 entries the records did not have: revoked 1 device again, revoked 0 grants again, removed 1 host again, deleted 1 account again.
   Running servers serve again within moments.
   ```

   Run it again if it stopped part of the way: it replays only what is left, and a second run says
   `Nothing to replay`.

4. Check: `GET /health` answers `ok` again within seconds, and the log says
   `the records are reconciled: serving`.

If `reconcile` says that some entries were **written by a newer server**, the restore went back past
an upgrade. Run the newer server's `reconcile`. Until then the records stay behind and every
request is refused.

## What a restore still loses

What happened after the backup and was not a revocation or a deletion: devices signed in since
(they sign in again), hosts registered since (registered again from the computer), grants given
since (paired again), and synced records written since, until a device that has them pushes them
again. A note revoked through sync after the backup is open on the server again. See 46 §12, under
"What it does not do yet".

## The drill

Staging's drill is the proof of R7's gate (46 §5). Run on 2026-11-22, and passed:

1. `main` deployed to staging (`e222688`) with the journal on; its log named the bucket.
2. A throwaway user made in staging's ZITADEL console (staging does not let anyone register), and
   two throwaway hosts signed in as it from this computer, each with its own `--data-dir`. The
   restore point noted: 2026-11-22T00:55:49Z.
3. One host signed out (`yardsort-host sign-out`): a host removed and its device revoked. The
   account deleted from the other (`yardsort-host account delete`, which needs `serve` running,
   run by the owner in a terminal of their own). Three rows in `journal`, all written.
4. Staging's database branched in Neon at the restore point
   (`neon branches create --parent <time>`), and `DATABASE_URL` pointed at the branch with
   `fly secrets import`. It timed out waiting for the health check, as step 2 above says.
5. `/health` and an authenticated route both answered 503 with `Retry-After: 60` and the restore
   message.
6. `reconcile` over `fly ssh console` replayed three entries: a device revoked, a host removed and
   an account deleted again. The log said `the records are reconciled: serving` within a second,
   and `/health` answered `ok`.
7. In the branch: the account, its host and its devices gone, three journal rows. A second
   `reconcile` said `Nothing to replay`. Staging pointed back at its own database (Neon's
   connection string for `main`), the owner's host reconnected and nothing was signed out, and the
   branch deleted.

Next time, keep a throwaway device's credentials aside before revoking it, so it can be seen to be
refused after the restore: signing out and deleting the account remove them from the computer.
