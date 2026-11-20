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
2. Point the server at the restored database, or restart it if the restore was in place. Each
   instance checks the journal before it serves, and finds the records behind it. Its log says
   `the journal holds revocations or deletions these records do not`, and `GET /health` answers
   503 from then on. That is expected.
3. Run `yardsort-server reconcile` where `DATABASE_URL` names the restored database and the
   journal's settings are set, the same environment the server runs with:
   - On a server of your own: `docker compose exec server yardsort-server reconcile`.
   - On Fly: the server's own image, as a one-off command in the app, so it has the app's secrets.
     The exact command is written here after the first drill on staging.

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

Staging's drill is the proof of R7's gate (46 §5), and has not been run yet:

1. Deploy `main` to staging with the journal on (its bucket's settings are already among the app's
   secrets).
2. Sign in a throwaway device and a throwaway account. Note the time.
3. Revoke the device, and delete the throwaway account.
4. Restore staging's database to the noted time as a Neon branch, and point staging's
   `DATABASE_URL` at the branch.
5. See `/health` answer 503 and a signed-in device refused with 503.
6. Run `reconcile`, and see it replay both.
7. See the device stay signed out, the deleted account's device told that its account was
   deleted, and every other device working. Point staging back at its own database afterwards, and
   delete the branch.

Write down here what was run, and what was found.
