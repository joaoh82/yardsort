# Remote, R2: identity and relay

**Status:** R2 started on 2026-10-22. Three slices are built. The first (2026-10-22): signing in
through the server's identity provider, and the server's own credentials for each device after
that. The second (2026-10-23): the `yardsort-server` binary, which serves them over HTTP from
PostgreSQL, and a local environment with a real ZITADEL in Docker. The third (2026-10-24): the
server's container image, its Fly.io configuration and deploy workflow, the self-host recipe, an
operator's command to sign an account out everywhere, and a sign-in against a real ZITADEL in CI.
Staging was deployed on 2026-10-24 ([28](28-remote-infrastructure.md) §5), and passed the third
slice's smoke test (§10) on 2026-10-25 at `https://remote-staging.yardsort.sh`. Nothing in the app or
`ys` uses the server. The proposal is
[25](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md); R0 and R1 are recorded in
[26](26-remote-r0-r1.md).

## 1 · The identity provider

The proposal settled the protocol: OpenID Connect, the authorization-code flow with PKCE through
the system browser, an established provider, and no password database in Yardsort (§5.1, §16).
The server accepts any compliant issuer. What was open was which provider the self-host recipe
documents and tests. The owner chose **ZITADEL** on 2026-10-22, after these were checked that day:

| What                        | Found                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Licence                     | AGPL-3.0 since v3 (2025-03-31). Its FAQ: running it unmodified is generally fine; the copyleft is about modifying it and offering that as a service; talking to it over HTTP APIs "likely" makes separate programs. SDKs, Helm chart and examples stay Apache-2.0. Yardsort is GPL-3.0-only, which is compatible either way. The FAQ does not speak to a paid hosted product running it unmodified: a question for counsel before the managed launch (R7), not before. |
| Maintenance                 | v4.20.0 released 2026-10-22, patches every week or two. Some patch releases needed configuration changes (a login-cookie secret in 4.19.x, a consent screen for the device flow in 4.19.5), so the recipe must pin exact versions and carry upgrade notes.                                                                                                                                                                                                             |
| What runs                   | The Go API, a separate Next.js login UI (since v4), PostgreSQL 14–18 and a reverse proxy that speaks HTTP/2 upstream; Redis is optional. The master key cannot change after the first start, so the recipe generates it up front.                                                                                                                                                                                                                                      |
| Cost to self-hosters        | No licence fee. One or two vCPUs and about 2 GB of memory beside the server; it can use the server's PostgreSQL, in a database of its own.                                                                                                                                                                                                                                                                                                                             |
| Cost to the managed service | Running it ourselves: no licence fee, only the machine and the operations. Its cloud: Free covers 100 daily active users a month; Pro is US$100 a month with 25 000; the overage rate is not published. A "daily active user" is summed over the days of the month, and **refreshing a token counts as activity**.                                                                                                                                                     |

The last row is why the design below exists.

## 2 · Sign-in only

A person signs in at the provider once per device. After that the device holds the server's own
credentials and never talks to the provider again.

```text
device ──GET /v1/login?challenge&redirect──▶ server ──302──▶ provider /authorize
                                                 (state, nonce, the server's own PKCE)
person signs in at the provider
provider ──302──▶ server /v1/login/callback?code&state
server ──POST /token (code, verifier, client secret)──▶ provider ──▶ ID token
server checks the ID token, finds or creates the account (issuer, subject)
server ──302──▶ device redirect?code=<one-time code>
device ──POST /v1/token {signIn, code, verifier, device}──▶ server ──▶ access + refresh
later: device ──POST /v1/token {refresh}──▶ server ──▶ new access + refresh   (provider not asked)
```

Why the server is the provider's client, and not the app:

- **A provider that bills by active user counts sign-ins, not refreshes.** A test signs a device
  in and uses it for thirty days: the provider receives one code exchange and one key fetch.
- **Revoking a device is the server's decision and takes effect on the next request.** Every
  access credential is checked against the records, so there is no window while a provider-issued
  token runs out.
- **The provider's secrets and tokens stay on the server** (§12). Devices never hold a provider
  token. Only `openid` is asked for, never `offline_access`, so the provider issues no refresh
  token, and the ID token's issuer and subject are all that is kept: not the email, not the name.
- **One redirect URI is registered at the provider**: the server's own `/v1/login/callback`.
  The app's scheme and a computer's loopback address are the server's business, so a self-host
  operator does not set up the provider anew for each kind of device.
- **An account is `(server, issuer, subject)`** (§5.1). Two subjects are two accounts whatever
  their email addresses say.

PKCE is used twice: between the server and the provider, and between the device and the server.
The one-time code that reaches the device's redirect is worth nothing without the verifier the
device kept, so another app that claims the same scheme gets nothing usable.

## 3 · Credentials

| Credential        | Looks like | Lifetime                        | Kept as | Use                                                                                                         |
| ----------------- | ---------- | ------------------------------- | ------- | ----------------------------------------------------------------------------------------------------------- |
| Access            | `ys1.at.…` | 15 minutes                      | SHA-256 | `Authorization: Bearer`, checked against the records on every request                                       |
| Refresh           | `ys1.rt.…` | 90 days unused, then signed out | SHA-256 | Once. The answer carries the next one. Presenting a spent one revokes the device: only a copy could hold it |
| Sign-in code      | `ys1.hc.…` | 60 seconds                      | SHA-256 | Once, with the device's verifier. A wrong verifier spends it too                                            |
| `state`, `nonce`  | random     | 10 minutes (the sign-in)        | SHA-256 | Sent to the provider; the answer must bring them back                                                       |
| Provider verifier | random     | 10 minutes (the sign-in)        | As is   | Sent to the provider with the code exchange, so it cannot be a hash                                         |

Each has 256 random bits, so a plain SHA-256 is enough: there is nothing to guess, and a slow hash
would only slow every request. The prefix says which kind a credential is, and one kind is never
accepted as another. `Secret`, the type credentials cross the wire in, prints as `Secret(…)`.

An ID token is accepted only if all of these hold (`oidc.rs`):

- It is signed with `RS256` or `ES256` by a key in the provider's published set. The algorithm
  is the one the key's own type allows, so a token cannot pick `none`, an HMAC keyed with the
  public key, or a key of another type.
- `iss` is exactly the configured issuer, which discovery also had to state exactly.
- `aud` includes the client id. `azp`, when present, is the client id, and must be present when
  there are several audiences (ZITADEL lists its project beside the client).
- `exp` has not passed, and `iat` and `nbf` are not in the future, each within 60 seconds.
- `nonce` is the one this sign-in sent.
- It names its key with `kid`. A token without one is refused, even from a provider that
  publishes a single key. ZITADEL always sends it.

The provider's endpoints must be HTTPS. Plain HTTP is accepted only on a loopback address, for a
provider running beside the server in development, and only one written as an address
(`127.0.0.1`), since a name such as `localhost` can resolve anywhere. That is why the local
ZITADEL is served at `http://127.0.0.1:8080`. There is no switch to skip certificate checks.

A key id the server has not seen fetches the key set again, at most once a minute, so a stream of
tokens naming unknown keys cannot make it hammer the provider. The key set and the discovery
document are also fetched again once they are an hour old, even when the key is known, so a key
the provider withdraws stops signing anyone in within the hour.

A sign-in goes back only to a redirect the operator allows, compared exactly. A loopback rule
(`http://127.0.0.1/signed-in`) matches any port, since a computer's client listens on whichever
port is free (RFC 8252 §7.3). Any other redirect is refused before anything is kept or anyone
is sent anywhere, so the server is not an open redirector.

## 4 · What the first slice adds

- `remote-proto::account`: the sign-in and device messages (`ServerDescription`, `LoginParams`,
  `TokenRequest`, `Credentials`, `DevicePage`), their bounds, and `Secret`. Five fixtures under
  `fixtures/v1/account-*.json` pin them. `Platform` gains `ios` and `android` (additive: an older
  peer reads them as `other`).
- `crates/server` (`yardsort-server`), a library:
  - `oidc`: discovery, the authorization URL, the code exchange and the ID token checks above.
  - `tokens`: minting, hashing, PKCE S256 and constant-time comparison.
  - `records`: the `Records` trait, one method per step that must happen whole (taking a one-time
    value, rotating a refresh credential), and `MemoryRecords`, which forgets everything on
    restart and is for tests.
  - `ControlService`: begin and finish a sign-in, redeem a code, refresh, authenticate, list and
    revoke an account's devices. Another account's device is `not_found`, exactly as one that
    does not exist.
- New dependencies, all under permissive licences: `jsonwebtoken` 11.1 (with its pure-Rust
  crypto, so no C toolchain on any platform), `reqwest` with rustls and `ring` as the app already
  has, `sha2`, `subtle`, `getrandom`, `url`. For tests only: `axum` 0.8 and `tokio`.

## 5 · What the second slice adds

- **`yardsort-server`, the binary** (`src/main.rs`), configured from its environment
  (`config.rs`): the public URL, the provider's issuer, client id and secret, `DATABASE_URL`, the
  listen address and the allowed device redirects. The client secret and the database URL can
  come from files instead (`…_FILE`), as container platforms mount them, and never from the
  command line. On `SIGTERM` it stops accepting and finishes what is in flight.
- **The HTTP routes** (`http.rs`): `GET /v1/server`, `GET /v1/login`, `GET /v1/login/callback`,
  `POST /v1/token`, `GET /v1/devices`, `POST /v1/devices/{id}/revoke`, and `GET /health` for a
  platform's checks.
  - The two browser routes answer with `303` redirects, or a short plain-text page when there is
    nowhere to send the browser. The rest answer JSON, and a failure is a `RemoteError` with the
    status its code implies (`401` with `WWW-Authenticate: Bearer`, `404`, `429`, …).
  - Every response is `Cache-Control: no-store` and `Referrer-Policy: no-referrer`, since codes
    and credentials pass through.
  - Bodies over 16 KB are refused with `413`.
  - A request is logged by method, path, status and time, never its query (where codes travel)
    or headers (where credentials do). A records failure is logged on the server and reaches the
    device only as "could not read its own records".
- **Rate limits** (`limit.rs`): 20 sign-ins started and 60 token requests a minute per address,
  with a burst of as many, answered with `429` and `Retry-After`. An IPv6 address counts with the
  rest of its /64. Behind a proxy, the address is taken from the header it names
  (`YARDSORT_CLIENT_IP_HEADER`, `Fly-Client-IP` on Fly.io) and otherwise from the connection.
  At most 10 000 addresses are counted one by one. Once that many are, full buckets are swept
  out at most once a second, and a newcomer that still finds no room shares one overflow bucket
  with every other, so a burst from many addresses costs bounded memory and time, and addresses
  already counted keep their own allowance.
- **At most 50 devices per account** (`MAX_DEVICES_PER_ACCOUNT`). A device counts while it is not
  revoked and holds a refresh credential that is neither spent nor lapsed, however it last used
  it (`Records::signed_in_devices`). The 51st sign-in is refused with
  `403` and a message saying to sign one out; the code it used is spent. Two sign-ins at the
  same moment can both get in, so this bounds what one account can make the records hold
  rather than being an exact quota.
- **`PgRecords`** (`postgres.rs`), the store a running server uses:
  - Each `Records` step is one statement or one transaction. One-time values are taken with
    `DELETE … RETURNING`. A rotation locks the refresh credential's row and its device's, so of
    two racing rotations exactly one gets through and the other is reuse.
  - Rotations also delete the device's lapsed access credentials, and its spent refresh
    credentials once they would have lapsed anyway, never sooner: a spent one must still revoke
    the device when a copy is presented (§9).
  - `last_seen_at` is written at most once a minute (`SEEN_RESOLUTION_MS`); `MemoryRecords`
    follows the same rule.
  - The server's id is made the first time any instance starts, and kept.
  - `tokio-postgres` with a pool, and TLS through rustls against the public web roots, as the
    connection string's `sslmode` says. Not `sqlx`: its SQLite driver's native library collides
    with the app's, even with that driver turned off.
- **Migrations** in `crates/server/migrations/`, applied in order when the server connects,
  under an advisory lock so that instances starting together take turns. Append-only, like the
  core's: a shipped file is never edited.
- **The local environment**, all on `127.0.0.1` with well-known throwaway secrets:
  - `crates/server/dev/compose.yaml` runs PostgreSQL 17 and ZITADEL v4.19.5 (API and login,
    behind Caddy), following ZITADEL's own Compose example (Apache-2.0).
  - `scripts/server-dev.mjs`, behind `just server-up`, creates the server's project and app in
    that ZITADEL through its API, with the settings 28's checklist gives a real instance, and
    writes the server's settings to a git-ignored file. Running it again changes nothing.
  - `just server-run` runs the server on the host, so a change rebuilds in seconds.
  - `just server-sign-in` (`examples/sign-in.rs`) is a pretend device: it signs in through the
    browser to a loopback listener, lists devices, refreshes, and revokes itself.
- **New dependencies**, all under permissive licences: `tokio-postgres`, `deadpool-postgres`,
  `tokio-postgres-rustls` and `webpki-roots` for PostgreSQL; `axum` (until now for tests only),
  `tokio`, `tracing` and `tracing-subscriber`.
- **CI** starts a PostgreSQL container on its Linux runner and requires the PostgreSQL tests to
  run there. The macOS and Windows runners have no containers, so those tests skip there.

## 6 · What the third slice adds

- **The image** (`crates/server/Dockerfile`), built from the repository root: `yardsort-server`
  alone on a distroless Debian 13 base, as a user with no shell, listening on 8080. Both bases
  are pinned by digest. The build context is an allowlist (`.dockerignore`): the Cargo workspace
  and nothing else, so no `node_modules`, `target/` or local `.env` file goes to a builder. It is
  52 MB, 14 MB compressed. The server only colours its log at a terminal, so a platform's log
  collector gets plain text.
- **Staging on Fly.io** (`crates/server/fly/staging.toml`): the settings that are not secret (the
  public URL, the issuer, the client id, `YARDSORT_CLIENT_IP_HEADER=Fly-Client-IP`), a health
  check on `/health`, one `shared-cpu-1x` Machine with 512 MB in `fra`. It stops when idle and
  Fly's proxy starts it on the next request; that changes once the relay holds long connections.
  The client secret and `DATABASE_URL` are Fly secrets, set from the providers' screens and never
  through a file or this repository.
- **The deploy workflow** (`.github/workflows/server-deploy.yml`), run by hand. It builds on Fly's
  builder, deploys one Machine (`--ha=false`), then checks the app's `fly.dev` address answers
  `/health` and names the configured issuer. It runs in the `staging` GitHub environment, whose
  `FLY_API_TOKEN` is a deploy token for that one app. Fly's setup action is pinned by commit, since
  it runs with the token in reach. Production is not offered: it arrives with its own ZITADEL
  instance, a Fly config and a `production` environment that requires a reviewer.
- **`yardsort-server revoke-account --subject SUBJECT [--issuer ISSUER]`**: revokes every device
  of one account that is not revoked yet, in one statement (`Records::revoke_account`), with a
  new reason, `byOperator`. The issuer defaults to the server's own. It needs only
  `DATABASE_URL`, and says how many devices it revoked; a subject that never signed in is an
  error rather than a quiet zero, and does not create an account. It does not stop the person
  signing in again: that is the provider's to refuse (§9).
- **The self-host recipe** (`crates/server/self-host/`): Compose with PostgreSQL 17, ZITADEL
  v4.19.5 (API and login), the server built from source, and Caddy, which gets certificates from
  Let's Encrypt. Secrets come from a `.env` the repository ignores. The server has its own
  database role. Caddy names the client's address in `X-Real-IP`, which it sets itself. Caddy
  answers for ZITADEL's public name inside the Compose network too, so the server reaches ZITADEL
  without leaving the machine, where many hosts cannot reach their own public address. Its README
  covers setting up, signing someone out everywhere, backups, restores and upgrades.
  `compose.localhost.yaml` runs the same recipe under `*.localhost` with Caddy's own certificate
  authority, to try it on one machine. `compose.own-proxy.yaml` leaves Caddy out for a machine that already runs a
  proxy, and the README lists what that proxy must do.
- **End to end against a real ZITADEL** (`just server-e2e`, and a CI job, "Server end to end"):
  the image runs against the local environment's ZITADEL and PostgreSQL. The sign-in tool signs
  in without a browser: it makes the two calls ZITADEL's own login pages make (a session from the
  username and password, then that session for the sign-in request) and follows the redirects as
  a browser would. It then lists devices and refreshes, and waits while the image's own
  `revoke-account` signs the account out, until its access and refresh credentials are both
  refused. The run fails if the server's log holds the client secret or a bearer credential.
- **The sign-in tool** (`examples/sign-in.rs`) gains `--zitadel-api` for that, and
  `--wait-for-revoke`, which a person can use too, against staging.
- An HTTP client the server cannot build now says why, instead of reqwest's "builder error". The
  self-host trial found it: an `SSL_CERT_FILE` the server could not read.

## 7 · Verification

On Linux, 2026-10-24, the third slice:

- `just server-test`: 61 tests, all passing. New: the contract test for `revoke_account` on both
  stores (two of three devices revoked, an earlier revocation and its reason kept, the same
  subject at another provider untouched, asking again changes nothing, an unknown subject is
  `None` twice so none was created); the built binary revoking an account in PostgreSQL, refusing
  another provider's subject and a misspelt one; and its errors for a missing subject, value,
  database and issuer.
- Two changes to the new query were made on purpose: without "not revoked yet" an earlier
  revocation was overwritten, and without the issuer another provider's account was revoked. A
  test failed each time.
- `just server-e2e`, the CI job's steps, passed: the image served the local ZITADEL, signed in,
  refreshed, and was revoked from inside its container. With the revoke pointed at a subject that
  does not exist, it failed in 32 seconds and left no process behind.
- **The self-host recipe**, under `compose.localhost.yaml`, with a throwaway `.env`: ZITADEL and
  Caddy came up healthy, ZITADEL's discovery answered over HTTPS, the server was built by
  Compose, reached ZITADEL by its public name, and answered `/health` and `/v1/server` through
  Caddy. The sign-in tool signed in through ZITADEL's API, from this machine, through Caddy;
  `docker compose exec server yardsort-server revoke-account` revoked it, and both its
  credentials were refused. Both databases were dumped as the README says; the server's was
  restored over itself with the server stopped, kept its tables' owner and its server id, and
  served again. The server's log held neither the client secret, the database password nor a
  bearer credential. With `compose.own-proxy.yaml`, ZITADEL's API and login answered on their
  published ports; the API answered only with its public `Host` (`404` otherwise), and over h2c.
  No other proxy was tried. Two fixes came out of it: ZITADEL's health check ignores `--tlsMode`, so
  the recipe sets TLS by its settings, and the error above.
- **The image with a browser**, the second local smoke test (§10): the sign-in tool started a
  sign-in at the image, headless Chromium went through ZITADEL's own login pages (name, then
  password) and back to the tool, which listed, refreshed and waited; `revoke-account` run in the
  container signed it out, and both its credentials were refused.
- `fly config validate` accepts `staging.toml`, and `fly deploy --build-only --local-only` with
  the workflow's arguments built the image from the repository root without pushing it.
  `actionlint` passes on both workflows.
- Staging's accounts were checked from here, by name only: the Fly app and its two staged
  secrets, the GitHub environment and its secret, the Neon project, and the ZITADEL Cloud
  instance's discovery document and key set. ZITADEL accepts the staging callback as a redirect
  for the client id, and refuses another.

**Staging**, deployed from this machine on 2026-10-24 with the workflow's own arguments (the
workflow can only be run from Actions once it is on `main`): Fly's builder built the image, 12 MB,
and one Machine started in `fra`. The server connected to Neon over TLS, made its tables and its
id, and listened 2.5 seconds after the Machine started; Fly's first health check came a second
before that and failed, and every one since has passed. `https://yardsort-remote-staging.fly.dev`
answers `/health`, and `/v1/server` names the staging ZITADEL as its issuer. `fly ssh console -C`
runs `revoke-account` in the image, which has no shell, with the Machine's secrets: for a subject
that never signed in it answered "no account" and changed nothing.

**Staging's smoke test** (§10), on 2026-10-25, once its DNS records were in: Fly's certificate
from Let's Encrypt was issued within a minute, and `https://remote-staging.yardsort.sh` answered
`/health`, with `/v1/server` naming the staging ZITADEL. The sign-in tool, run from this machine,
sent the browser to ZITADEL Cloud and back; it listed its device, refreshed, and waited. The
account's subject was read from the staging database (the one account, with one live device),
and `revoke-account` over `fly ssh console` revoked that one device. Within a second the tool
reported both its credentials refused. Fly's log of the run has every request by method, path
and status, none with a query string, and no secret, database URL, code or credential.
Scanners found the name within minutes of its certificate (certificate transparency) and got
`404` for everything they tried.

**Not run here:** ZITADEL's login pages in the self-host recipe or
in CI, which both use the API the pages call; the recipe on a machine reachable from the
internet; macOS and Windows for any of it.

On Linux, 2026-10-23, the second slice, with the review's additions of 2026-10-24:

- `just server-test`: 57 tests, all passing.
  - `tests/records.rs`: the same thirteen checks of what a store must do, run on `MemoryRecords`
    and on `PgRecords` against PostgreSQL 17, each test in a fresh schema. They include eight
    rotations of one refresh credential racing (exactly one gets through), a copy of the first
    refresh credential still revoking the device three rotations later, which devices count as
    signed in (revoked, rotated, lapsed and another account's), names with non-ASCII
    text, and every device kind and platform. Two more are PostgreSQL's alone: the server's id
    and schema survive a restart, and four instances starting together on an empty schema
    migrate once and agree on one id.
  - `tests/http.rs`: a whole sign-in through the router on a real port, on both stores: start,
    the provider's answer, redemption, the device list, refresh, an unknown device's `404`,
    revocation and the `401` after. Also the status, body and headers of each kind of wrong
    request, and the rate limits.
  - The first slice's tests, and two more for the device limit: 50 lapsed devices, then 50 in
    use, then a refused 51st, another account unaffected, room again after a revocation; and 50
    devices that only ever refreshed, across the 90 days, still counting.
  - Unit tests for the settings and the limiter, including a burst from 20 000 addresses at one
    instant: 10 000 tracked, one sweep, the rest sharing one allowance.
- Seven changes to `PgRecords` were made on purpose, one at a time: no row locks in a rotation,
  revocation not scoped to the account, a sign-in not removed when taken, a second revocation
  overwriting the first, reuse not revoking the device, lapsed handoffs kept, and spent refresh
  credentials deleted at the next rotation. A test failed each time. So did one with the device
  limit taken out, with lapsed credentials counted, with devices counted by when they were last
  seen (which missed the ones that only refresh: found in review), with the limiter sweeping on
  every request (the burst took 2.1 s instead of 0.02 s), and with it tracking every address.
- **Against a real ZITADEL** (v4.19.5, in the local environment): `just server-sign-in` signed in
  through ZITADEL's login pages, which headless Chromium filled in. The device was listed,
  refreshed, revoked, and then refused. This also settled two things:
  - ZITADEL accepts the server's PKCE together with its client secret. The **Code**
    authentication method in 28's checklist works as written.
  - ZITADEL accepts `127.0.0.1` as its domain, so its issuer is a loopback address.
- The server's log from that run holds no code, credential, `state`, client secret or database
  password.

On Linux, 2026-10-22, the first slice:

- `cargo test -p yardsort-server`: 5 unit tests and 11 integration tests (12 after review). The
  integration tests run against a stand-in provider served over real HTTP by `axum`, which
  checks what a real one does: client authentication (Basic, with a secret containing `%` and
  `+`, and a public client), the code, the redirect URI and the PKCE verifier. It signs real
  RS256 and ES256 tokens with keys generated for the tests (`tests/fixtures/`). They cover:
  - signing in once and using the device for thirty days, with one code exchange and one key
    fetch at the provider;
  - two subjects are two accounts: each lists and revokes only its own devices, and a revoked
    device stops on the next request;
  - a sign-in code works once, for a minute, with its verifier only;
  - the provider's answer counts once and only within ten minutes; a cancelled sign-in goes back
    to the device with an error and no code;
  - only allowed redirects, and a loopback one on any port;
  - thirteen ways an ID token can be wrong (issuer, audience, `azp`, several audiences without
    `azp`, nonce changed or missing, expired, issued in the future, signed with another key under
    a published id, an unpublished key id, `alg: none`, HMAC keyed with the public key, an RSA
    signature under an EC key's id), none of which creates an account;
  - a rotated provider key is fetched, and unknown ones are fetched at most once a minute;
  - a key the provider withdraws is refused once the key set is an hour old (added after review,
    2026-10-24; it fails with the maximum age taken out);
  - a provider whose discovery names another issuer is not used, nor one on plain HTTP;
  - a refresh credential works once, and presenting a spent one signs that device out.
- Ten checks were broken on purpose, one at a time: nonce, `azp`, several audiences, `iat`, the
  key refetch limit, the device's verifier, the redirect allowlist, refresh reuse, the account
  scope of revocation, and a revoked device's access. A test failed each time.
- `cargo test -p remote-proto`: the account fixtures round-trip, a `Secret` never prints, and the
  challenge, verifier and device-name bounds hold.

**Not run here:** macOS and Windows, which CI covers, without the PostgreSQL half (above); ZITADEL
Cloud, as opposed to ZITADEL in a container; anything on a phone; any deployment.

## 8 · The rest of R2

Each item is a later slice. Where each runs, and the accounts to set up, are in
[28](28-remote-infrastructure.md). The gate (proposal §13): two-account isolation, a revoked device
losing a live stream, and relay failure leaving PTYs alive.

1. ~~Sign-in and device credentials, as a library.~~ Done (§4).
2. ~~PostgreSQL records, the HTTP listener, rate limits, and the local environment.~~ Done (§5).
3. ~~Deploying it, and the self-host recipe.~~ Done (§6): staging deployed and its smoke test
   passed (§7). Production's deploy comes with its ZITADEL instance.
4. **Host enrollment and grants**: a host identity persisted when Remote is enabled, enrollment
   by QR secret or human code (hashed, five minutes, rate-limited), approval at the host, grants
   from that approval, and where a headless host keeps its credential
   ([open question 35](06-open-questions.md#remote)).
5. **The daemon, `PROTOCOL` 2**: one controller lease and one query responder enforced in the
   daemon, and numbered output over its IPC.
6. **The relay and the host's outbound connector**: WSS, single-use 60-second connection tickets,
   bounded queues, revocation closing live streams, and relay failure leaving every PTY running.

## 9 · Risks

- **Rate limits are per instance and per address.** Two instances allow twice the rate, and many
  addresses together are not limited as a whole. Once more than 10 000 are in play, newcomers
  share one allowance, so during such a burst a person signing in from a new address may be
  told to wait a minute. The callback is not limited: it only takes a sign-in that
  was started, which is.
- **Refresh reuse signs a device out even when the device is honest**, when the answer carrying
  its new credentials was lost in transit after the server rotated them. That is the standard
  trade, and a sign-in fixes it. A short grace window would trade some of the protection away;
  [open question 38](06-open-questions.md#remote).
- **An access credential is a bearer credential.** A copy works until its quarter hour is up,
  or until the device is revoked, which takes effect on the next request.
- **Disabling a person at the provider does not sign their devices out.** The provider is asked
  once per device, at sign-in, and each refresh starts the 90 days again, so a device in use
  keeps working for as long as it stays in use. That follows from asking the provider only at
  sign-in (§2), which is what keeps a provider that bills by daily active user cheap. Until it
  is answered, the operator's path is to disable the person at the provider and then run
  `yardsort-server revoke-account` (§6). A maximum session length, after which
  a device has to sign in again however often it is used, is the other answer; it costs a sign-in
  every so often and is not chosen yet ([open question 39](06-open-questions.md#remote)).
- **Spent refresh credentials must outlive their rotation.** A store that dropped a spent one at
  once would answer a stolen copy with "unknown" and leave its holder signed in, instead of
  revoking the device. The `Records` contract says so; `MemoryRecords` keeps everything, and
  `PgRecords` drops one only after its own expiry, which a record test checks.
- **A device that never comes back leaves its rows**: its last access and refresh credentials'
  hashes stay until it is revoked or its account is deleted. They open nothing, but nothing
  sweeps them either.
- **Account deletion and export** (proposal §9) are not built.
- **`jsonwebtoken`'s pure-Rust backend brings the `rsa` crate**, which has an open timing
  advisory (RUSTSEC-2023-0071) on operations with a private key. The server only verifies, with
  the provider's public keys; only the tests sign. Nothing in CI audits dependencies yet.
- **The provider's own `error` text** comes through the browser. It is bounded to 64 characters
  and only reaches the server's log, never the device.
- **The image's bases do not update themselves.** They are pinned by digest, so a fix in Debian's
  glibc or in Rust reaches the server only when someone moves the pins (the Dockerfile says how).
  Nothing watches for that yet.
- **CI's end-to-end run does not render ZITADEL's login pages.** It makes the calls they make, so
  a change in the pages themselves would get past it. A person signing in on staging is the check
  for them (§10).
- **Staging sleeps.** The first request after it stopped waits for a Machine to start, and Neon's
  free compute to wake: a few seconds, not more, but enough to look like a failure.
- **The deploy token lasts until 2046,** Fly's default. It deploys one app and nothing else, and
  lives only in the GitHub environment; replacing it is `fly tokens create deploy` again.
- **The self-host recipe runs ZITADEL's containers as root**, as ZITADEL's own Compose example does,
  so that the API can write the login client's token where the login container reads it.

## 10 · Smoke tests

From the third slice on, each slice ends with a smoke test a person runs: on their own machine,
and on staging once the slice is deployed there. They are short on purpose. The tests above check
the details; these check that the whole thing works where it will run.

**Slice 2: the server, on this machine.** `just server-up`, `just server-run`, and in another
terminal `just server-sign-in`. Sign in as `developer@yardsort.test` with `Yardsort-dev-1`. It
lists the device, refreshes, revokes itself, and shows the `401` after.

**Slice 3, on this machine** (Linux, with Docker):

1. `just server-up`, then `just server-e2e`. It ends with "End to end: passed".
2. The image with a browser: `just server-image`, then
   `docker run --rm --name yardsort-smoke --network host --env-file crates/server/dev/server.env.local -e YARDSORT_LISTEN=127.0.0.1:8787 yardsort-server:dev`,
   and `just server-sign-in --wait-for-revoke` in another terminal. Sign in in the browser. While
   it waits, revoke the account from the container:
   `docker exec yardsort-smoke /usr/local/bin/yardsort-server revoke-account --subject <id>`, with
   the id of `developer@yardsort.test` from ZITADEL's console at `http://127.0.0.1:8080/ui/console`.
   The sign-in tool reports both its credentials refused.
3. Optionally, the self-host recipe under `compose.localhost.yaml`, as its README describes.

**Slice 3, on staging**, once it is deployed:

1. `https://remote-staging.yardsort.sh/health` answers `ok`, and `/v1/server` names
   `https://yardsort-staging-pegnzh.eu1.zitadel.cloud` as its issuer.
2. `just server-sign-in https://remote-staging.yardsort.sh --wait-for-revoke`, and sign in on
   ZITADEL Cloud's pages in the browser with a staging account.
3. While it waits, revoke that account. Take the user's id from the staging ZITADEL console, then
   `fly ssh console -a yardsort-remote-staging -C "/usr/local/bin/yardsort-server revoke-account --subject <id>"`.
   The sign-in tool reports both its credentials refused.
4. `fly logs -a yardsort-remote-staging` holds no client secret, database URL, code or credential.
