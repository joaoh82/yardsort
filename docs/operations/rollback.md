# Rolling back the server

What to do when a release of `yardsort-server` misbehaves and the one before it should run again,
in minutes, without building anything. The design is
[46 §14](../design/46-remote-r7.md#14--slice-5-production-and-rollback).

## When

- A deploy finished, and something that worked before it does not: sign-in fails, devices cannot
  reach their hosts, sync refuses what it took before, the server's log fills with errors.
- Fix forward instead when the fix is small and sure, or when the release before it is the one
  with the fault.

A rollback puts back the **image**: the server's binary. The Fly config (its environment, Machine
size and checks) is the branch's the workflow runs from, as for any deploy. Secrets stay as they
are.

## The database

Nothing to do. Migrations are append-only, and an older server leaves the ones a newer server made
as they are (`crates/server/src/postgres.rs`, `migrate`): it runs on the database as it stands. What
the newer release added to the database stays there, unused, until it runs again.

What the newer release journaled stays journaled: the journal is outside the database, and an
older server that knows the journal reads the entries it knows (46 §12). One from before the
journal (before 2026-11-19) neither writes nor checks it, so revocations made while it runs are
not journaled.

## Rolling back

1. **Actions → Deploy the server → Run workflow**, from `main`. Choose the environment, and in
   **Roll back instead** put:
   - `previous`: the image before the one running now (releases that failed are skipped);
   - a release's number, `12` or `v12`, as `fly releases -a <app> --image` lists them;
   - or an image of the app's own, `registry.fly.io/<app>:deployment-…`.

   From a terminal with `gh`:

   ```sh
   gh workflow run server-deploy.yml --ref main -f environment=staging -f rollback=previous
   ```

2. The workflow says which image is running and which it puts back, deploys that image to every
   Machine without building, checks `/health` and `/v1/server`, and then lists each Machine's
   image. It fails if a started Machine runs anything else.
3. Check that devices are back: hosts reconnect within a minute, as after any deploy (a host backs
   off for up to a minute), and the server's log shows `relay: host connected`. Phones reconnect
   when next opened.

`previous` run twice goes back and forth: after a rollback, the image running is the older one,
and the one before it is the newer one.

## Afterwards

- Write down here what was rolled back, from what to what, and why.
- Fix forward on a branch; a normal deploy of `main` replaces the rolled-back image.

## The drill

Staging's drill is half of the proof of R7's gate (46 §5): deploy, roll back, and every device and
host back within a minute. Production's follows once production exists. Not run yet on staging.
