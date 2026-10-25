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

**What works today.** Signing in, and listing and revoking devices: the server's first slices
([docs/design/27-remote-r2.md](../../../docs/design/27-remote-r2.md)). The Yardsort app does not
connect to a server yet, so for now this is for trying the server and getting ready for what comes
next.

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

It says how many devices it revoked. Each one stops on its next request.

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

**Restoring** the server's database, with the server stopped:

```sh
docker compose stop server
docker compose exec -T postgres pg_restore -U postgres -d yardsort --clean --if-exists < yardsort.dump
docker compose start server
```

ZITADEL's restores the same way, with `zitadel-api` and `zitadel-login` stopped instead.

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
4. **Let the server reach the auth name.** The server fetches ZITADEL's settings and keys at
   `https://<auth name>`. The override sends that name to this machine (`host-gateway`), so your
   proxy has to listen on 443 on an address the containers can reach, not on `127.0.0.1` alone.

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
