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
host back within a minute. Production's follows once production exists. Run on staging on
2026-11-22, and passed:

1. `main` deployed (`57f4066`). Its image was built and started, then crashed: staging's Neon
   database had run out of the free plan's compute and refused every connection (28 §2). The
   owner moved Neon to its Launch plan, and the Machine, already on `main`'s image, started by
   itself. Fly kept that release as failed, which showed the workflow taking "what runs" from the
   newest release that finished: `previous` would then have gone one image too far. It now asks
   the Machines.
2. `rollback=previous`, at 18:09:52 UTC: `Running now: …01M826K6…`, `Putting back: …01M7V8YN…`
   (the image before). The Machine started on it at 18:10:13, the checks passed, and every
   started Machine ran the image put back. The owner's host reconnected at 18:10:13.7, 0.3 s after
   the server listened.
3. `main` deployed again at 18:11, a new image (`…01M82794…`); the host reconnected 0.14 s after
   the server listened.

Nothing was signed out: the only devices revoked that day were the load test's, by itself. A
`POST /v1/token` refused once a minute throughout is an older client's credential, from before.
