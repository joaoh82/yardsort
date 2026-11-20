# Running the Yardsort server yourself

The Yardsort server signs people in and keeps track of the devices they sign in with. Later it will
also carry their terminals between their machines and their phone. This recipe runs it on one
machine of yours, with everything it needs:

- `server`: `yardsort-server`, built from this repository.
- `zitadel-api` and `zitadel-login`: [ZITADEL](https://zitadel.com), the identity provider people
  sign in with. It holds their passwords and passkeys; the server holds none.
- `postgres`: PostgreSQL 17, with one database for ZITADEL and one for the server.
- `proxy`: [Caddy](https://caddyserver.com), the only part reachable from outside. It gets the
  certificates from Let's Encrypt and renews them.

Nothing in it calls Yardsort's own servers.

**License.** The server is under the [AGPL-3.0](../LICENSE). Running it as it is asks nothing
more of you. If you change it and let other people use it, you must offer those people the
source of your changed version
([docs/design/30](../../../docs/design/30-open-source-and-managed.md)).

**What works today.** Signing in, listing and revoking devices, pairing a device with a host
(approved at the host), and the relay, through which a paired device reaches its host's
workspaces and terminals with neither of them listening for connections
([docs/design/27-remote-r2.md](../../../docs/design/27-remote-r2.md)). A computer can also leave a
notice for the phone: a task, a review, a failure, a desktop update, or a workflow question or
approval. The phone opens the item. Answering and installing stay on the computer
([docs/design/40-mobile-notifications.md](../../../docs/design/40-mobile-notifications.md)).
Notices wait on the server until you set Apple or Google push credentials. The Yardsort app can be a host, in
Settings → Remote, which is still under development. The phone app is in `apps/mobile`, and is not in
the app stores yet, so you build it yourself; the repository's pairing tool also stands in for one. So
for now this is for trying the server and getting ready for what comes next.

## What you need

- A Linux machine with Docker and Docker Compose, with ports 80 and 443 reachable from the
  internet.
- Two DNS names pointing at it, such as `auth.example.com` for ZITADEL and `remote.example.com` for
  the server.
- A clone of this repository on that machine. The server's image is built from it.

## Setting it up

All commands run in this directory, `crates/server/self-host`.

1. **Settings.** Copy `.env.example` to `.env`, make it yours alone (`chmod 600 .env`), and fill
   it in. The file says how to make each secret.

2. **Start ZITADEL and Caddy.**

   ```sh
   docker compose up -d proxy
   ```

   This starts PostgreSQL, ZITADEL and Caddy. ZITADEL sets itself up the first time, which takes
   a minute or two. Caddy then asks Let's Encrypt for both names' certificates.

3. **Sign in to ZITADEL** at `https://auth.example.com/ui/console` as `ZITADEL_ADMIN_EMAIL`, with
   `ZITADEL_ADMIN_PASSWORD`. It asks you for a new password straight away. Add a passkey or
   two-factor authentication to that account.

4. **Create the server's application in ZITADEL.**
   1. Under **Projects**, create a project named `Yardsort server`.
   2. In it, create an application named `Yardsort`, of type **Web**.
   3. **Authentication method:** **Code**. The server authenticates with a client secret, and
      sends PKCE as well.
   4. **Redirect URI:** `https://remote.example.com/v1/login/callback`, with your own name. Leave
      the post-logout URIs empty, and development mode off.
   5. Create it, then copy the **client id** and the **client secret** into `.env` as
      `YARDSORT_OIDC_CLIENT_ID` and `YARDSORT_OIDC_CLIENT_SECRET`. ZITADEL shows the secret only
      once; if you lose it, generate a new one from the application's page.

   While you are there, decide who may sign in. In ZITADEL's login settings, turn
   self-registration off unless anyone who finds the page should get an account, and add people
   under **Users** instead.

5. **Build and start the server.**

   ```sh
   docker compose up -d
   ```

   The first build takes a few minutes. The server creates its own tables the first time it
   starts.

6. **Check it.** `https://remote.example.com/health` answers `ok`, and
   `https://remote.example.com/v1/server` names `https://auth.example.com` as its `issuer`.

   To sign in as a device would, run this on any computer with a clone of this repository and
   Rust, and sign in through the browser that opens:

   ```sh
   cargo run -p yardsort-server --example sign-in -- https://remote.example.com
   ```

   It lists the account's devices, refreshes its credentials once, and then revokes itself.

## Running it

**Logs:** `docker compose logs -f server`. The server never writes a credential, a sign-in code,
its client secret or the database password to its log.

**Signing someone out everywhere.** Disabling a person in ZITADEL stops them signing in again,
but devices they already signed in keep working until they are revoked. To revoke all of them at
once, take their **user id** from their page in ZITADEL's console and run:

```sh
docker compose exec server yardsort-server revoke-account --subject <user id>
```

It says how many devices it revoked. Each one stops on its next request, and the running server
closes their live connections at once.

## Backing up

What to keep, and where:

- **`.env`.** Above all `ZITADEL_MASTERKEY`: ZITADEL encrypts its signing keys and secrets with it,
  so a database backup without it cannot be used.
- **Both databases.** Run these from cron, and keep the files somewhere else:

  ```sh
  docker compose exec -T postgres pg_dump -U postgres -Fc yardsort > yardsort.dump
  docker compose exec -T postgres pg_dump -U postgres -Fc zitadel > zitadel.dump
  ```

Caddy's certificates (the `caddy-data` volume) need no backup: Caddy gets new ones.

- **The journal**, if you keep it in a directory (below). Never restore it to an earlier point:
  it only grows.

## The journal

A backup of the database holds what was open when it was taken. Restored, a device revoked since
can sign in again, a host removed since is back, and an account deleted since is back. The server
writes each revocation and deletion outside the database as it happens, to a journal, so that a
restore can be brought up to date. Without one, the server warns as it starts.

Put it somewhere that is not restored with the database: a bucket of an S3-compatible store, or a
directory on another disk. Add to the `server` service's `environment` in `compose.yaml` either

```yaml
YARDSORT_JOURNAL_BUCKET: yardsort-journal
AWS_ENDPOINT_URL_S3: https://s3.example.com
AWS_REGION: auto
AWS_ACCESS_KEY_ID: ${JOURNAL_ACCESS_KEY_ID}
AWS_SECRET_ACCESS_KEY: ${JOURNAL_SECRET_ACCESS_KEY}
```

or `YARDSORT_JOURNAL_DIR` with a volume mounted there that the server's user (`nonroot`, uid 65532)
can write. The bucket is named in the host (`https://yardsort-journal.s3.example.com/…`), or in
the path for an endpoint at an IP address or `localhost`; set `YARDSORT_JOURNAL_ADDRESSING` to
`path` or `virtual` if your store wants the other. Its key needs to list, read and write; nothing
in the server deletes from it. As the server starts, its log says where it journals.

The journal holds ids and times only: which device, grant, host or account, never a name or an
email. A deleted account's id stays in it, which is what keeps a restore from bringing the account
back.

## Restoring

The server's database, with the server stopped:

```sh
docker compose stop server
docker compose exec -T postgres pg_restore -U postgres -d yardsort --clean --if-exists < yardsort.dump
docker compose start server
```

With a journal, the server then finds the restored database behind it and refuses every request
with 503, until you bring it up to date:

```sh
docker compose exec server yardsort-server reconcile
```

It revokes, removes and deletes again everything the journal has that the database does not,
says what, and the server serves again within moments. Running it twice does no harm. The
[restore runbook](../../../docs/operations/restore.md) says more, and what a restore still loses.

ZITADEL's database restores the same way, with `zitadel-api` and `zitadel-login` stopped instead.

## Upgrading

Take a backup first. Neither the server nor ZITADEL can go back to an older version once a newer
one has changed its database.

- **The server:** check out the newer version of this repository, then
  `docker compose up -d --build server`. It brings its tables up to date as it starts.
- **ZITADEL:** read its release notes for every version in between, then change both ZITADEL
  images in `compose.yaml` to the new version, and `docker compose up -d`. It migrates its
  database as it starts.
- **PostgreSQL:** within 17, change the image's version and `docker compose up -d`. A new major
  version needs a dump, a new volume and a restore.
- **Caddy:** change its version and `docker compose up -d`.

## Using your own proxy

If the machine already runs a reverse proxy (nginx, Traefik, a cloud load balancer), leave Caddy
out and point that proxy at the containers. `compose.own-proxy.yaml` publishes them on this
machine's loopback address:

| Container       | Published at     |
| --------------- | ---------------- |
| `zitadel-api`   | `127.0.0.1:8081` |
| `zitadel-login` | `127.0.0.1:3000` |
| `server`        | `127.0.0.1:8787` |

Use that file in every command, and never start `proxy`:

```sh
docker compose -f compose.yaml -f compose.own-proxy.yaml up -d zitadel-login
docker compose -f compose.yaml -f compose.own-proxy.yaml up -d server
```

`ACME_EMAIL` goes unused, but `compose.yaml` still asks for it: any value does.

Your proxy has to do what the `Caddyfile` does:

1. **TLS for both names**, with certificates browsers trust.
2. **For the auth name**, send `/ui/v2/login` and everything under it to `zitadel-login`, and
   everything else to `zitadel-api` **over HTTP/2 without TLS** (h2c), which ZITADEL's gRPC
   needs; ZITADEL's documentation on reverse proxies has examples for the common ones. Pass the
   original `Host` on: ZITADEL answers only for its own name, and answers `404` to any other.
3. **For the server's name**, send everything to `server`, and set a header to the client's
   address, replacing any the client sent (in nginx,
   `proxy_set_header X-Real-IP $remote_addr;`). The server counts its rate limits by it. If the
   header is not `X-Real-IP`, set `YARDSORT_CLIENT_IP_HEADER` to its name in `compose.yaml`.
   Never name a header the client can set itself, such as `X-Forwarded-For` passed on as it came.
   Pass WebSocket upgrades on too: hosts and devices hold one open at `/v1/relay` for as long as
   they are connected (in nginx, `proxy_http_version 1.1;` with `Upgrade` and `Connection`
   headers set from the request's). It pings every 20 seconds, so a proxy may close a connection
   quiet for a minute, but not sooner (nginx's `proxy_read_timeout` is 60 seconds already).
   Run one `server` ([One instance](#one-instance)).
4. **Let the server reach the auth name.** The server fetches ZITADEL's settings and keys at
   `https://<auth name>`. The override sends that name to this machine (`host-gateway`), so your
   proxy has to listen on 443 on an address the containers can reach, not on `127.0.0.1` alone.

## One instance

Run one `server`. Each instance holds its own relay sockets, and a device's connection has to
reach the instance holding its host's. Only a proxy that can send a request on to a named
instance does that: Fly.io's, with `YARDSORT_ROUTING=fly-replay`. Behind any other proxy, several
instances would leave devices unable to reach hosts connected to another.

The server keeps one more connection to PostgreSQL than its pool, to hear what other processes
sharing the database did, such as the operator's `revoke-account` ([Running it](#running-it)). `DATABASE_URL`
has to be a direct connection, as this recipe's is, not a pooler that hands out a connection per
transaction. Behind one, what another process revokes reaches live connections only at the
server's next five-second check, and its log warns about it about once a minute.

## What each account uses

The server counts what each account uses, a UTC day at a time: how many of its devices made a
request, the most connections its devices had open to its computers at once, and the bytes the
relay carried for it and its computers sent to sync. Counts only, never what passed through.
Each day is kept 400 days; the account's export includes them, and deleting the account deletes
them. To see them:

```sh
docker compose exec server yardsort-server usage --from 2026-11-01 --to 2026-11-30
```

It prints a line a day for each account, by the server's own account id, tab-separated; the last
seven days unless you give dates. On several instances, the most connections is the most through
one instance.

## Plans and entitlements

Unset, the server allows everything up to its own limits, such as 50 devices an account: that is
how a server you run yourself is meant to run, and you need nothing here. A managed service can
set `YARDSORT_ENTITLEMENTS_URL` to a service of its own, which the server asks before signing in
another device, opening a connection from a device to a computer, or storing more synced data. It
`POST`s the account's id (the server's own, never the identity provider's subject), the action
(`device`, `stream` or `sync`) and the count that action needs, such as `{"devices": 2}`, and
nothing that names a person. The answer is `{"allowed": true}`, or `{"allowed": false,
"message": "…"}` with words to show the person. `YARDSORT_ENTITLEMENTS_TOKEN` (or `_FILE`) is sent
with each question as a bearer token. The address must be HTTPS, or plain HTTP to a loopback
address.

Answers are kept a minute for each account and action. When the service cannot be reached, or
does not answer within three seconds, what an account was allowed in the last 24 hours stays
allowed, and anything else waits, with a message that the plan cannot be checked right now. That
memory is each instance's own, so an instance started while the service is down knows of nothing
allowed yet. Nothing is asked before reading, signing out, removing a computer, exporting or
deleting, and devices already signed in keep working whatever the answer.

## What it keeps for sync

Sync between a person's computers is being built
([design 42](../../../docs/design/42-remote-r6.md)). The server already has its tables, and keeps
what a computer syncs there in plain text, so whoever runs the server can read it: a project's
name and Git remote address, its approved memory notes, its workspaces' names and branches. No
computer syncs anything yet. A deleted record leaves a tombstone, without its contents; after 90 days it shrinks to a marker of
its id and revision, kept so that a computer away for months cannot bring the record back.
A sync push is up to 256 KiB, so a proxy of your own must allow request bodies at least that large
(nginx allows 1 MB unless told otherwise).

## Trying it on one machine first

`compose.localhost.yaml` runs the same recipe under `auth.localhost` and `remote.localhost`, with
certificates from Caddy's own authority instead of Let's Encrypt, and the ports open to this
machine alone. Set those two names in `.env`, and add the file to every command:

```sh
docker compose -f compose.yaml -f compose.localhost.yaml up -d proxy
```

Your browser warns about the certificates; that is expected. To make the sign-in tool trust them,
copy Caddy's authority out, build the tool, and run it with that authority alone:

```sh
docker compose -f compose.yaml -f compose.localhost.yaml \
  cp proxy:/data/caddy/pki/authorities/local/root.crt /tmp/caddy-root.crt
cargo build -p yardsort-server --example sign-in
SSL_CERT_FILE=/tmp/caddy-root.crt ../../../target/debug/examples/sign-in https://remote.localhost
```

Remove it all afterwards with `docker compose -f compose.yaml -f compose.localhost.yaml down -v`,
which also deletes its databases.
