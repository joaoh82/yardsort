# Remote, R2: identity and relay

**Status:** R2 started on 2026-10-22. Seven slices are built. The first (2026-10-22): signing in
through the server's identity provider, and the server's own credentials for each device after
that. The second (2026-10-23): the `yardsort-server` binary, which serves them over HTTP from
PostgreSQL, and a local environment with a real ZITADEL in Docker. The third (2026-10-24): the
server's container image, its Fly.io configuration and deploy workflow, the self-host recipe, an
operator's command to sign an account out everywhere, and a sign-in against a real ZITADEL in CI.
Staging was deployed on 2026-10-24 ([28](28-remote-infrastructure.md) §5), and passed the third
slice's smoke test (§14) on 2026-10-25 at `https://remote-staging.yardsort.sh`. The fourth
(2026-10-25): pairing. A host registers on its account and shows a code; a device of the same
account redeems it and asks; the host's person approves it at the host, which keeps its own record
of what it gave. The app does this from Settings → Remote, shown only with
`YARDSORT_REMOTE_PREVIEW=1`, because nothing can reach a paired host until the relay exists. The
fifth (2026-10-28): the daemon's protocol 2, with one controller and one query responder per
terminal, and numbered output and control leases over its socket (§8). The sixth (2026-10-28):
the relay, and the host's outbound connector to it, so a paired device reaches its host through
the server (§9). The seventh (2026-10-29): a device that reconnects resumes its stream where it
left off, and is told when control changes hands and when the program ends (§10). The proposal is
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
- **The provider's secrets and tokens stay on the server.** Devices never hold a provider
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
    the device when a copy is presented (§13).
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
  signing in again: that is the provider's to refuse (§13).
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

## 7 · What the fourth slice adds

**Pairing on the server** (`remote_proto::pairing` for the messages, `src/pairing.rs`, migration
`0002_hosts_and_grants.sql`):

| Route                                      | Who                                | What                                                                |
| ------------------------------------------ | ---------------------------------- | ------------------------------------------------------------------- |
| `POST /v1/hosts`                           | a device signed in as a host       | Register, or register again under the id the server gave it before  |
| `GET /v1/hosts`                            | any device of the account          | The account's hosts, as the phone will list them                    |
| `POST /v1/hosts/{host}/offers`             | the host                           | A pairing offer, in place of any it still had open                  |
| `POST /v1/pairings`                        | another device of the account      | Redeem an offer and ask for scopes: a pending grant                 |
| `GET /v1/hosts/{host}/grants`              | the host                           | Grants waiting for its person, and the ones in force                |
| `POST /v1/grants/{grant}/approve`          | the host                           | Give some or all of what was asked, on all projects or named ones   |
| `POST /v1/grants/{grant}/deny`             | the host                           | Refuse                                                              |
| `GET /v1/grants`, `GET /v1/grants/{grant}` | the device it is for, and its host | Where a grant stands: pending, approved, denied, expired or revoked |
| `POST /v1/grants/{grant}/revoke`           | the device it is for, or its host  | Take it back                                                        |
| `POST /v1/hosts/{host}/remove`             | the host                           | Stop being one: its offers go and its grants are revoked            |

- **Every check is the account first.** Another account's host, offer or grant is `not_found`,
  like one that does not exist. Within the account, only the device a host is signed in as now
  acts for it, and a grant is seen by its host and the device it is for: not by the account's
  other devices.
- **An offer** is two things, either of which redeems it, once, within five minutes
  (`OFFER_TTL_MS`): a secret for a QR code (`ys1.po.…`, 256 bits) and a short code for a person to
  type (eight characters of Crockford's base32, 40 bits, shown `ABCD-EFGH`; typed in any case,
  with or without the dash, `O` read as `0` and `I`/`L` as `1`). Both are kept as SHA-256
  hashes. One host has one offer open at a time.
- **The short code only finds an offer.** It is looked up within the caller's own account, so a
  stranger would first have to be signed in to it. Each miss is counted against every open offer
  of that account, and five misses (`MAX_OFFER_MISSES`) spend them all; the host shows a new
  one. One address may ask for ten offers and try ten pairings a minute. Then the host's person
  still has to say yes to a device they can see by name.
- **Redeeming asks; it gives nothing.** The pending grant waits five minutes for the host's
  person. An approval gives at most what was asked for. Approving a device on a host revokes any
  grant it had there before, so a device holds one grant per host; answers on one host take
  turns, so two approvals at once cannot both stand. A host cannot pair with itself.
- **Signing a device out ends its grants**, as revoking them or removing the host does. A host
  that signs in again keeps its id and its grants, and the device it was before stops acting
  for it.

**The host's own record** (`yardsort-core`, `remote::host`, migration `0018_remote_host.sql`):

- `remote_host` is this machine as a host on one server, written when Remote is turned on and
  deleted when it is turned off. A new server's host does not inherit an earlier server's grants.
- `remote_grants` holds the devices this host's person let in, and is written **only when they
  approve one here**, with what they chose. The server's list can take a grant away, never add
  one (`Store::retain_remote_grants`). `remote::host::grant_for` lets a device in only when both
  agree: the grant is in force at the server and this host approved it. A server that lied could
  not add a device.
- The host independently checking its grants is proposal §5.3's rule. It matters once the relay
  carries requests: the facade will be handed only what `grant_for` returns.

**Where a host keeps its credential** ([open question 35](06-open-questions.md#remote), settled):
`remote::host::CredentialStore`, a small trait in the core. The app puts the OS credential store
behind it (the same `keyring` entry service Assist uses, as `remote-host-refresh-<device>`). The
entry is named for the device the host signed in as because that store is per OS user while the
host is per profile: a development build or a throwaway profile turning Remote on or off leaves
the installed app's alone.
`FileCredentials` is the other: one file per secret in a directory only its owner can read
(`0700`, files `0600`, and a directory others could read is made private), for a machine with no
credential store. The app uses it when `YARDSORT_CREDENTIALS_DIR` names one, and the headless
host (R5) will by default, from a mount kept apart from its data. Neither the database, a
settings file nor a terminal's environment ever holds a credential. Each new refresh credential
replaces the spent one in the session before it is kept, since presenting the spent one would
sign the host out (§3); if keeping it fails, as with a locked keyring, the session carries on
with it until the app quits, and the error says so.

**`remote-client`**, a new crate: the device's side of the server's API. It signs in through the
system browser to a loopback listener, refreshes (`Session`), and makes the calls above. It speaks
only HTTPS, or HTTP to a loopback address written as one. It is apart from the core because `ys`
never talks to a server, and the headless host will use it without the app.

**The app** (`src-tauri/src/remote.rs`, Settings → Remote): shown, and its commands answering,
only when the app starts with `YARDSORT_REMOTE_PREVIEW=1`.

- **Turn on**: the server's address and a name for this computer. It signs in through the
  browser as a host (Cancel stops waiting), and registers.
- **Pair a device**: a QR code (the server's address and the offer's secret, drawn by the
  `qrcode` crate, MIT or Apache-2.0) and the short code, with how long they are good for.
- **Waiting for you**: each device that redeemed it, by name, with what it asked for. The
  answers are _Let it watch_ (workspaces, activity, approved memory and terminals, read only),
  _Let it watch and type_ when it asked to type, which the section says is anything the person
  could do at this computer, and _Refuse_. Either way on all projects, including ones added
  later, or on the ones ticked. `device:manage` is never given.
- **Devices let in**, each with what it may do and a **Revoke**, which is refused here at once
  and then told to the server.
- **Turn off** removes the host at the server, signs its device out there, and forgets both here.
  If the server cannot be reached it is still turned off here, and the error says so. The server's
  address and the name are kept, in the settings file's `[remote]` section, and offered the next
  time Remote is turned on.

The section asks the server every three seconds while it is open and Remote is on. The webview
never sees a credential or the QR code's secret, only its image. Nothing in `ys`, and nothing an
agent's environment can reach, approves a device: as with memory, only the desktop does.

**The pairing tool** (`examples/pair.rs`, `just server-pair`) stands in for a phone (`phone
[SERVER] CODE`, asking to watch and type, then waiting for the answer and optionally for a
revocation), or for a host (`host [SERVER]`, which shows a code and approves the first device
that redeems it). `just server-e2e`, and with it CI's "Server end to end", now also runs both
against the image: the host shows a code, the phone redeems it, the host lets it watch and type
and then takes that back, and the phone sees each step. The run fails if the server's log holds
anything starting `ys1.`, which every credential, code and pairing secret does.

## 8 · What the fifth slice adds

**The daemon, `pty_ipc::PROTOCOL` 2** (2026-10-28): one controller and one query responder per
terminal, decided in `pty-host` and so in the daemon, where the app and `ys attach` cannot get
round them; and numbered output and leases over the daemon's socket. Nothing outside this
computer reaches a terminal yet. What a person can notice is local: a terminal shown in the app
and in `ys attach` at once now answers a program's questions once.

**One controller** (`pty_host::control`):

| Who                                 | How they get control                                                                                                              | When it ends                                            |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| The person at the computer, `Local` | By typing or pasting, in the app or `ys attach` (`write`, `paste`): at once, from a lease holder too                              | 30 s after the last thing typed here                    |
| A lease holder, by the name it gave | `acquire`, refused while `Local` has it, and from another holder unless `take_over`. A lease lasts 30 s and is renewed every 10 s | `release`, 30 s without renewal, or someone typing here |

Every change of hands bumps the terminal's generation, and every call under a lease names its id
and generation, so a holder that lost control types nothing more even on a connection still
open (`lease_lost`). Input under a lease is numbered from 1: a repeat is dropped, so a client
unsure whether an input arrived may send it again, and a gap is refused (`input_out_of_order`)
with nothing after it written. The holder's size stands while it holds control. A resize from
here meanwhile is kept, and put back when control returns: on release, or with the next thing
typed here after a lapse. The daemon does not decide who may ask: anything that reaches its
socket is already this computer's user. Whether a device may is its grant's `terminal:control`,
checked by the remote side before it asks.

**One responder** (`pty_host::queries`). A program can ask its terminal where the cursor is,
what it is, which colours it uses, whether a mode is set, and the terminal types the answer
back. With two terminals attached both answered, and the program read the second answer as
typing. Now exactly one viewer is shown the questions: the terminal attached most recently, or,
while a lease holder controls the terminal, none, and the host answers where the cursor is
itself, as it always did when nobody watched. Every other viewer, numbered ones included, is
sent the output with the questions taken out. They are xterm's list (device status and
attributes, mode and setting reports, window reports, colour and clipboard queries, XTVERSION,
XTGETTCAP) and kitty's keyboard and colour queries. A question draws nothing, so every viewer
still draws the same screen. A question cut across two batches has already gone out half-sent
when it turns out to be one, so a quiet viewer gets `CAN` for the rest, which abandons it in the
viewer's parser; xterm.js, `vte` and real terminals all treat it so. Nothing is held back or
delayed. The app's webview and `ys attach` needed no change: they are simply not shown what
they should not answer. Like the cursor answer before it, this reads only the terminal
protocol, never what a program printed for a person.

The responder's answer comes back as a write from here, which would take control. Windows'
ConPTY asks where the cursor is whenever a terminal starts, so every launch would have shut
devices out for 30 seconds. So for two seconds after a terminal is shown a question, a write
made only of answer-shaped sequences is written without taking control.

The app also types on its own: a workflow's `send_to_session` step. Nobody here typed that, so
it goes through `type_unattended`, which takes no control and is refused (`controlHeld`) while a
lease is held; the step waits and tries again at the next tick, as it does while the agent is
busy. Once the message is pasted, its Enter goes even if a lease arrived in between, so a
message is never typed twice. A request the person here makes in the app, such as asking the
agent to resolve a pull request's conflicts, is theirs, and takes control as typing does. A
launch's first message is typed by the host whoever controls the terminal: it is part of the
launch the person asked for.

**Over the socket** (`pty-ipc`). New requests: `watch` (numbered viewing), `controller`,
`acquire`, `renew`, `release`, `input`, `resizeLeased` and `typeUnattended`, with the errors `leaseLost`,
`controlHeld`, `inputOutOfOrder`, `invalidHolder` and `controlUnavailable`. Numbered output has
a frame kind of its own, 5: `[u32 stream][u8 item][u64 number]`, then the size for a snapshot
or a resize, then the bytes. `DaemonClient` speaks all of it, and `TerminalHost` has it with
defaults that refuse, for the tests' stand-ins.

**Upgrades.** Protocol 2 adds and changes nothing of 1, so this build still runs terminals in a
daemon of 1 (`understood`), and refuses the new requests itself (`shares_control`): a daemon of 1
would drop the connection on a request it cannot read, and with it every terminal the app shows.
At start the app uses a daemon of its own protocol; replaces an older one that is idle; keeps
an older one with agents in it, so sharing their terminals waits until it is idle and
replaced; and runs terminals in-process only beside a daemon it does not understand, as every
protocol change did before. With a daemon of 1, which has no leases, the app's own typing goes as
`paste` and `write`, as before.

**Not yet.** Nothing calls the leases but tests and the example below; the host facade still
answers every terminal operation `unsupported` until the relay's connector uses them (§12). A
lease holder's own terminal does not answer questions: while one controls a terminal, only the
cursor question is answered, by the host. Making the controller's numbered viewer the responder
is for R3. Nobody is told when control changes hands; a holder finds out at its next input or
renewal, within ten seconds.

**The lease tool** (`crates/pty-ipc/examples/lease.rs`) takes a terminal the app is showing, the
way the connector will: it watches it numbered, takes a lease, types under it, sends an input
twice and one out of order, then waits for a person to type in the app and shows the lease lost.
It ends saying whether every item came, once and in order.

## 9 · What the sixth slice adds

The relay, and the host's connector to it: a paired device now reaches its host through the
server, with neither of them listening. The app keeps the link while Remote is on, still only
with `YARDSORT_REMOTE_PREVIEW=1`. There is no phone app yet (R3); the pairing tool stands in for
one (§14).

**Tickets** (`POST /v1/relay/tickets`, `remote_proto::relay`). A signed-in device asks with its
access credential for a ticket, good once for 60 seconds. A host asks for its own link, and gets
one only as the device the host is signed in as; any other device asks for its way to one host,
and gets one only with a grant on it the server has in force. Another account's host is
`not_found`, exactly as one that does not exist; a host of the same account without a grant is
`forbidden`. The ticket is the first message on the WebSocket (`GET /v1/relay`), never part of a
URL, so no proxy logs it; the access credential never goes on the socket. Tickets are kept in
memory as hashes (`ys1.rl.`), at most 10 000 waiting, and asking is limited to 60 a minute per
address.

**Sockets.** A host's socket carries every device's traffic, each in a channel the relay numbers:
a five-byte header (kind, channel) on every binary message. The relay opens a channel with the
device and the grant it found; the host closes one when its own record does not agree. A device's
socket carries the remote protocol itself, untouched. Close codes say why a socket closed
(`revoked`, `host offline`, `overloaded`, `replaced`, `refused`, `restarting`, …). The relay
pings every 20 seconds and takes 60 seconds of silence for a dead socket. One host link per
host: a second replaces the first, and its devices reconnect. At most 16 devices per host at
once.

**Bounded.** What waits for each socket is bounded at 2 MiB. A device that falls that far behind
is closed (`overloaded`) and its host told; the host and its other devices carry on, and the
relay never stops reading anyone to wait for a slow one. On the host, each device's terminal
output is bounded at 1 MiB (`MAX_QUEUED_OUTPUT`): past it, that stream is detached with
`resnapshot_required`, and the daemon never waits.

**Revocation reaches live sockets.** The relay checks every connected account against the
records every five seconds, and at once when this server revokes a device, a grant or a host, or
an approval replaces a grant. A socket whose device, grant or host is no longer in force closes
(`revoked`), and the host drops the channel, releasing whatever that device controlled. On the
host, every request looks its grant up again, so revoking in the app stops the next request; the
connector checks every channel every five seconds and at once after a revoke in the app.

**The host's side** (`yardsort-core`'s `remote::serve`, and the new `remote-host` crate). The
connector (`remote_host::Connector`) asks for a ticket with the host's credential, opens the
relay, and answers each channel on a thread of its own, in order: the greeting, then requests.
Read requests go to the facade, as before. The terminal operations are answered now:

- `terminal.attach` is a numbered watch on the daemon (slice 5). The reply comes first, then the
  snapshot as frames numbered with the reply's position, then each later item with the next
  number: output as frames, a resize as a `terminal.resized` event. Output over 64 KiB is split
  into frames with the same number, so a client sees numbers that never skip and never go back.
  The epoch is always 0, and `resumeAfter` always gets a fresh snapshot: nothing is kept to
  replay (until the seventh slice, §10).
- `terminal.acquire`, `renew`, `release`, `input` and `resize` are the daemon's lease, held as
  `device:<id>`. A lease belongs to the channel that took it: another device naming it is told
  `lease_lost`, as for any lease it does not hold. The controller a device is shown is another
  device by the name this host knows it by, or the person here.
- Which terminals a grant reaches: a terminal of a workspace in a project the grant reaches.
  Anything else, a terminal with no workspace included, is `not_found`.

Closing a channel, for any reason, detaches what its device watched and releases what it
controlled. When the link fails, every channel closes that way, the connector waits (1 second,
doubling, up to a minute) and connects again. **The terminals are the daemon's and are never
touched**: the programs run on, and the person here types into them as before. Nothing a device
typed is sent again after a reconnect: the lease it typed under is gone.

**Protocol 1.1.** The minor goes to 1: a `HelloReply` (`welcome` or `error`), `Event`s
(`terminal.resized`, `terminal.detached`), and the terminal operations, which a host lists in its
capabilities only when its daemon shares control. The daemon's protocol is told apart with
`TerminalHost::shares_control`. Behind a busy daemon of protocol 1 (slice 5), devices can read the
lists but not watch or type.

**The app.** Settings → Remote says where the link stands: connecting, connected and how many
devices, waiting to try again and why, or stopped (signed out, or the host removed). The link
starts with the app when Remote is on, and once it is turned on; it stops before turning off.

**The pairing tool** goes through the relay with `--relay`: it greets the host, finds a running
terminal it may see, watches it from a snapshot checking the numbers, types with
`--type TEXT`, and with `--wait-for-revoke` watches until the relay closes the socket and says
why.

**Not yet:** a phone app (R3). Telling a device when control changes hands, or when a terminal's
program exits: it finds out at its next request. A replay window for resuming. (Those two came
with the seventh slice, §10.) More than one
relay instance (§13). The entitlements question before opening a stream ([30](30-open-source-and-managed.md)).
Telling the host of a pairing request over its link: Settings → Remote still asks every three
seconds while open.

## 10 · What the seventh slice adds

A device that loses its connection carries on from where it was, instead of being sent the whole
screen again, and a watching device is told who controls the terminal and when its program ends,
without asking. Still only with `YARDSORT_REMOTE_PREVIEW=1`, and still no phone app (R3); the
pairing tool stands in for one (§14).

**The replay window** (`pty_host::replay`). From the first numbered viewer on, a session keeps
the last 256 KiB of its numbered stream: output, without the program's terminal queries, and
resizes. A viewer that already shows everything up to an item asks to resume after it
(`PtyHost::resume_numbered`). When the session still keeps every item since, the viewer is sent
`Resumed`, with that item's number and the size the terminal had then, and those items, then the
live stream; otherwise a snapshot, as before. Either way nothing is lost or doubled. A session
nobody has watched numbered keeps nothing, and the window is the session's, in the daemon's
memory: it goes with the session.

**Control and the exit, told.** A numbered viewer is sent who controls the session right after
its first item, then each time that changes: a lease taken, released or taken over, the person
here typing, and a hold lapsing. A hold lapses lazily, when someone next asks, so the session's
pump now wakes when the hold in force would lapse, and says so though nothing else happened; an
operation after which a hold lapses sooner wakes it. Once the program has ended, and every item
it printed has been sent, the viewer is sent `Exited`. Neither carries a number. Tests shorten
the holds with `PtyHost::with_holds`.

**The daemon, protocol 3.** `Watch` takes `after`, and the three new items have frames of their
own: a resume with its size, and who controls the session and how it exited as JSON. The daemon
sends those two only to a client that said 3 in its hello. A client of 3 with a daemon of 2 does
not ask to resume and gets a snapshot, and no control or exit items: the visitor asks the daemon
who controls a terminal for its reply, as before. A daemon of 2 is replaced once nothing runs in
it, as for every protocol change.

**Protocol 1.2.** The minor goes to 2. `terminal.attach` honours `resumeAfter`: when the host
resumes, the reply says `resumed` with the position the device gave, and the items after it
follow, numbered as they would have been. A position in another epoch, or past the end of the
stream, gets a snapshot. Two events: `terminal.control` (`stream`, `controller`) and
`terminal.exited` (`stream`, `exit`: `code`, `success`). A resumed stream is not sent the control
changes it missed; its reply says who controls the terminal now. The controller a device is
shown is named as before, from a copy of this host's names for its devices that the channel reads
again on each attach and lease and at each grant check, every five seconds: the daemon's items
arrive where the store must not be asked.

**The pairing tool** with `--relay`, after typing, lets go of its stream and attaches again from
the last item it drew, and says whether the host resumed from there. It says when control changes
hands and when the program ends. CI's end-to-end round checks that it resumed.

**Not yet:** the phone app (R3). More than one relay instance (§13). The entitlements question
before opening a stream ([30](30-open-source-and-managed.md)). Telling the host of a pairing
request over its link. Typing is never replayed, by design: after a reconnect the device's lease
is gone (§9).

## 11 · Verification

On Linux, 2026-10-29, the seventh slice:

- `pty-host`: 3 new unit tests of the window: what came after a kept item comes back in order
  with the size at that item; the oldest go first once it is full, sizes with them; output larger
  than the window leaves only the present to resume from. 3 new tests against real PTYs: a viewer
  that resumes is sent exactly what one that stayed was sent after the same item, output and
  resizes, starting at the size it had then and told who controls the session now; a position
  the stream has not reached gets a snapshot; a viewer away for more than the window holds gets
  a snapshot (Unix); a viewer is told control changing hands, and a lease and then a local hold
  lapsing with nothing else happening (holds of 600 and 300 ms). The first numbered test now
  also checks the exit comes after every item, and that a late viewer gets the snapshot, who
  controls the session and the exit.
- `pty-ipc`: frames for the three new items round trip and refuse every truncation. The daemon,
  over a raw socket: a client of 3 hears who controls the session right after the snapshot and of
  the exit last, and a client of 2 hears neither. Through the client: a watch resumes where it
  left off and gets what one that stayed got.
- `remote-proto`: new fixtures pin `terminal.control`, `terminal.exited` and a resumed attach;
  the minor is 2.
- `yardsort-core`, `remote::serve` (1 new test): local typing reaches a watching device as a
  `terminal.control` event; the device detaches, the program prints, and attaching with
  `resumeAfter` is answered `resumed` at that position, with only what it missed, numbered on
  from it, and the controller now; a position in another epoch gets a snapshot; killing the
  program sends `terminal.exited`.
- The gate test (`relay_host`) now also lets go of its stream and attaches again through the
  real relay, and is answered `resumed` at the last item it drew.
- Changes made on purpose, one at a time, each failing a test: the pump not telling of a lapse
  when it wakes for one; the daemon sending the new items to a client of 2; output not kept in
  the window (it failed only the visitor's test at first, so the resume test now prints while the
  viewer is away).
- Found on the way and fixed before it shipped: waiting for a lapse inside an `if let` on the
  view's lock held the lock for the whole wait, which hung every session; and a lease taken while
  the pump already waited with no deadline was never timed, until operations that make a hold
  lapse sooner woke the pump.
- `just check` with the `YARDSORT_*` variables unset: `pty-host` 29 + 29, `pty-ipc` 14 + 13,
  `remote-proto` 40, `remote-client` 6, `remote-host` 1, `yardsort-core` 601, `yardsort-server`
  13 + 81, the app crate 150, frontend 669 (on a second run: `Summary.test.tsx`'s Markdown test
  timed out under the full run's load, as it has on other branches). `just lint-windows` and
  `just bindings-check` passed.

**The smoke test on this machine** (§14), on 2026-10-29, run by a person: passed.

**Not run here:** the end-to-end run, which needs Docker access this session still does not have,
and which CI runs, now checking that the pretend phone resumed; Windows and macOS, where CI runs
the same tests; staging's smoke test, which waits for slice 6 to be deployed there. No phone.

On Linux, 2026-10-28, the sixth slice:

- `remote-proto`: 40 tests, 5 new. A link frame's header is pinned byte by byte, frames round
  trip, and short, unknown, malformed and oversized ones are refused; close reasons keep their
  codes, and an unknown one reads as unknown. New hand-written fixtures pin the ticket request,
  the ticket, the presented ticket, the greeting, the hello reply and both events.
- The server, against both stores (`just server-test`, PostgreSQL in Docker):
  - Tickets: one works once and not once it lapses, another kind of credential is no ticket, and
    at most 10 000 wait.
  - Over a real socket (7 tests): a phone reaches its host and they talk on its channel; an
    unpaired phone of the same account is `forbidden`, another account's phone `not_found`, and
    neither another account's host nor a phone gets a host's link; a ticket works once, and an
    access credential is not one; nothing on one phone's channel reaches another. Revoking a
    grant, signing a device out and removing a host each close the live sockets within the
    three seconds the test allows, before the five-second check could, and the host is told to
    drop the channel. A phone that never reads is let go (`overloaded`) and its host told, and
    the other phone still hears its host. A host not yet connected is `host_offline`; one that
    connects again replaces its old link, whose phones reconnect; one that leaves takes its
    phones' sockets with it. A phone sending binary, and a host opening a channel, are closed.
  - **The gate, with a real host behind the relay** (`tests/relay_host.rs`): the connector, the
    core's records and a program in a real PTY. A phone types through the relay and the program
    answers, on the phone and in the host's own view. Its grant is revoked at the server: its
    socket closes in under five seconds and its control is let go. Another phone takes control,
    then the server fails: its socket closes, nothing holds control, the program runs on, and
    typed at the host it answers; the connector says it is waiting to reconnect.
- `yardsort-core`, `remote::serve` (5 tests, against real PTYs; PowerShell programs on Windows
  since the review): a device
  is let in only under a grant this host approved, greeted before anything else, and a grant
  revoked here stops its next request and closes the channel. Terminals outside the grant's
  projects, or in no workspace, are `not_found`; a watch-only grant cannot take control; another
  device naming a lease is told `lease_lost`, and cannot take control without taking over.
  Watching starts with the reply, then the snapshot numbered as the reply says; typed input
  reaches the program and its answer the device, numbered without a skip; a resize under the
  lease is an event numbered after the output before it; the controller shown is the device by
  the name approved here. A device that falls behind is detached and told, and the program never
  waits. Closing the channel lets go of its lease and its watch, and the program runs on.
- `remote-host` (1 test, a stand-in relay on a real socket): a device comes through the link,
  takes control and watches; the link drops, the lease is let go, the program runs and takes
  typing here; the connector comes back by itself; a grant this host never approved is refused
  whatever the relay says; a grant revoked here closes its channel at once; stopping closes the
  link.
- Settings → Remote (1 new test): what the link's state says, as the poll brings it.
- Changes made on purpose, one at a time, each failing a test: a closed channel keeping its
  leases; the attach reply sent after the output it starts; the connector's channel not letting
  go when the link drops; revoking a grant without nudging the relay (the five-second check
  alone is too late for the test); a device's queue without a bound.
- `just check` passed with the `YARDSORT_*` variables unset: `pty-host` 26 + 26, `pty-ipc`
  13 + 12, `remote-proto` 40, `remote-client` 6, `remote-host` 1, `yardsort-core` 600,
  `yardsort-server` 13 + 81 (its PostgreSQL half skipped there, run by `just server-test`), the
  app crate 150, frontend 669. `just lint-windows` passed. `just bindings-check` differs only by
  the new `RemoteLink` type, generated, as the change needs.

**Not run here:** the smoke test (§14), which needs a person to sign in at a browser, and the
end-to-end run (`just server-e2e`), which needs Docker access this session does not have; CI runs
it, and its pairing round now goes through the relay: the pretend host serves a program with the
connector, and the pretend phone types into it. Windows and macOS, where CI runs the same tests;
the relay test with a real host uses PowerShell on Windows. No phone.

On Linux, 2026-10-28, the fifth slice:

- `pty-host` unit tests: 26, 13 new.
  - Leases against a clock the test moves: renewed in time, they last; left alone, they lapse.
    Input is written once and in order, and never after a gap. Typing here takes control from a
    lease at once, and nobody may lease while the person here has it, until 30 s after their
    last keystroke. Another holder needs `take_over`, and the one it took from is out. A
    released or forged lease does nothing, and a holder's name must be printable.
  - Questions told from everything else: 29 kinds of question, and 22 sequences that look alike
    but are not questions (`CSI 6 m`, `CSI 8;24;80 t` that resizes a window, an OSC 8 link
    ending in `?`, a kitty image). Answers told from typing: 13 answer shapes, 13 kinds of
    keys and pastes. Controls carried out inside a question stay; a question too long, or
    abandoned, is left alone.
  - A stream with six questions among colours, a title, multibyte text and the alternate
    screen, cut at every byte. The copy without the questions draws the same screen and cursor
    as the whole, holds no question, and every cursor question is counted once. Leaving `CAN`
    out fails it.
- `pty-host` integration tests: 26, 7 new, real programs in real PTYs:
  - Two terminals that each answer: only the newer is asked, and the program hears one
    answer. Detach the newer, and the older answers.
  - With a lease held, no terminal is asked and the host answers; a numbered viewer is never
    sent the question.
  - A terminal's answer leaves the terminal free to lease.
  - Input under a lease is typed once, a gap refused, until typing here ends the lease.
  - A holder's size stands, a resize from here meanwhile comes back on release and on typing
    here, and numbered viewers hear every resize in order.
  - An exited terminal cannot be leased.

  Sending every viewer the questions fails two of these; counting a terminal's answer as typing
  fails a third.

- `pty-ipc`: 13 unit tests, 2 new, and 12 against a daemon in its own process, 3 new.
  - Numbered frames keep their place and size, and a truncated or unknown one is refused.
  - A client of a pretend protocol-1 daemon runs terminals in it, refuses the seven lease and
    watch requests itself, and sends the app's own typing as a `paste`: the daemon hears only
    `hello`, `list` and `paste`. Taking that guard out fails it. The test binds its socket in
    its own temporary directory, not the runtime one, which CI does not have.
  - Over the socket: a numbered viewer gets every item once and in order, the resize among
    them. A lease on a second connection types once, is refused out of order, renews, and is
    lost to typing on the first; the app's own typing is refused while the lease holds. With two connections each attaching a terminal, only the
    newer is asked.
- `yardsort-core`: two new tests.
  - What the app does with the daemon it finds: use its own kind; replace an older one that is
    idle; keep an older one with agents in it; and leave one it does not understand alone.
  - A workflow's message due while a lease is held waits, against a real script in a real PTY:
    nothing is typed, the lease keeps its generation, and once it is released the message goes
    and the terminal is left with nobody in control. Typing it the way the person here types
    fails it.
- The lease tool against a real daemon, with a script on the socket standing in for the app:
  the lease typed once, the out-of-order input never reached the shell, typing on the other
  connection took control and the lease was refused, and five items came numbered without a gap.
- `just check` passed with the `YARDSORT_*` variables unset: `pty-host` 26 + 26, `pty-ipc`
  13 + 12, `yardsort-core` 595, the app crate 150, frontend 668. The frontend run once failed
  `Summary.test.tsx`, which passed on its own run and again with the rest; this slice changes no
  frontend code. `just bindings-check` passed, the bindings unchanged, and so did
  `just lint-windows`.

**Not run here:** the daemon on Windows and macOS, where CI runs the same tests. The lease test
over the socket uses PowerShell on Windows to read lines, which has not run yet. No phone or
remote client was involved: nothing outside this computer reaches a terminal yet.

On Linux, 2026-10-25, the fourth slice:

- `just server-test`: 82 tests, all passing, the record and HTTP tests on both stores. New:
  - Seven record contract tests: a host registered, moved to a new device, kept from another
    account and removed for good; an offer redeemed once, only within its account, replaced by
    the next, lapsing, and gone with its host; misses spending only their own account's offers,
    one short of the limit and at it; a grant answered once, one per device per host, a denial
    revoking nothing, too late to answer; revoking and removing; eight approvals racing for one
    device, of which exactly one stands; and every scope, project and name kept as given.
  - Pairing over HTTP: the whole of it on both stores, from registering to removing, and what
    another account cannot do (redeem the code or the secret, take the host's id, make its offers,
    list, answer or revoke its grants), with the host's own pending request untouched afterwards
    and a signed-out device's grant gone from the host's list. Also: wrong codes spending the open
    offer, malformed pairings spending nothing, and the per-address limits.
- Eight changes were made on purpose, one at a time. Each failed a test: the per-host lock taken
  out of an approval (six or seven of the eight racing approvals then stood), any device acting
  as the host, an approval giving more than was asked, offers redeemed across accounts, a host
  pairing with itself, a phone registering as a host, a signed-out device keeping its grant,
  and another device of the account seeing a grant. That last one passed at first: no test had a
  third device on the account. One was added.
- `just server-e2e` passed with the pairing round: two pretend devices signed in through
  ZITADEL's API, against the image; the phone saw itself let in to watch and type on every
  project, then revoked. The server's log held nothing starting `ys1.`, nor the client secret.
- `yardsort-core`: seven tests of the host's record and its credentials. A device is let in
  only with the grant this host approved and the server names; a grant only the server knows,
  or another device naming it, lets nobody in; the server's list only takes away; another
  server's host starts with none; a row this build cannot read gives nothing; credential files
  are `0600` in a `0700` directory, rewritten whole, and never reached by a name that is a path;
  and each device's credential has its own entry.
- `remote-client`: six tests, among them the loopback listener taking the code from the right
  path only, past a connection that sends nothing; a refused sign-in's error text not echoed;
  and a refresh credential that could not be kept still being the one presented next.
  `remote-proto`: 35, with the pairing fixtures and code normalising.
- The frontend: nine tests of Settings → Remote (turning on and its error, the server and name
  it was last turned on with offered again, a failed poll's error taken down once the server
  answers, the pairing code and QR code, letting a device watch or watch and type on chosen
  projects, offering no typing to a device that did not ask, refusing, revoking, turning off)
  and one that the tab appears only with the preview switch.
- `just check`, `just bindings-check` and `just lint-windows` passed, and so did clippy of
  `remote-proto` for Windows. `remote-client` could not be checked for Windows from here: `ring`
  needs Microsoft's C tools to cross-compile. CI's Windows runner builds it.

**The smoke test on this machine** (§14), on 2026-10-27, run by a person: the app with a
throwaway profile and a credentials directory turned on against the local server, signed in
through ZITADEL's login pages, showed a code, and let the pairing tool in to watch and type on
every project; revoking it reached the tool; after a restart Remote was still on and paired
again; turning off emptied the credentials directory. It found one gap: once turned off, the
form was empty again, so the server and name had to be typed anew. They are now kept (§7).

**Staging** runs this slice since 2026-10-27: release v3, deployed from this machine with the
workflow's own arguments, since the branch was not yet pushed. The same Machine and server id;
the server made the new tables in Neon, `/v1/server` lists `host.list` and `host.pair`, and an
unauthenticated `POST /v1/hosts` is answered `401`.

**Staging's smoke test** (§14), on 2026-10-27, run by a person: the app turned on against
staging, signed in on ZITADEL Cloud's pages, and let the pairing tool in to watch and type;
revoking it reached the tool, and turning off removed the host. The first try looked like a
fault: the device appeared under **Devices let in** and vanished a poll later. Staging's log
showed why: the tool had been run without `--wait-for-revoke`, so it signed its own device out
42 ms after reading its approval, and the host's next poll rightly found the grant revoked. The
tool now says when it signs out. The log of the run holds every request by method, path and
status, none with a query string, and no `ys1.`, code, bearer credential or secret.

**Not run here:** the OS credential store on macOS and Windows.

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
- **The image with a browser**, the second local smoke test (§14): the sign-in tool started a
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
that never signed in it answered "no account" and changed nothing. Once the workflow was on
`main`, it deployed staging from Actions on 2026-10-25 (run 40233851558) in 42 seconds: release
v2 on the same Machine, the same server id, and both of its checks passing.

**Staging's smoke test** (§14), on 2026-10-25, once its DNS records were in: Fly's certificate
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

## 12 · The rest of R2

Each item is a later slice. Where each runs, and the accounts to set up, are in
[28](28-remote-infrastructure.md). The gate (proposal §13): two-account isolation, a revoked device
losing a live stream, and relay failure leaving PTYs alive.

1. ~~Sign-in and device credentials, as a library.~~ Done (§4).
2. ~~PostgreSQL records, the HTTP listener, rate limits, and the local environment.~~ Done (§5).
3. ~~Deploying it, and the self-host recipe.~~ Done (§6): staging deployed and its smoke test
   passed (§11). Production's deploy comes with its ZITADEL instance.
4. ~~Host enrollment and grants.~~ Built (§7), behind the preview switch; done once its smoke
   test (§14) passes here and on staging. The headless host's own way to approve a device comes
   with the headless host (R5): today only the desktop approves.
5. ~~**The daemon, `PROTOCOL` 2**: one controller lease and one query responder enforced in the
   daemon, and numbered output over its IPC.~~ Built (§8); done once its smoke test (§14) passes
   here. Nothing of it runs on the server, so staging has no part in it.
6. ~~**The relay and the host's outbound connector**: WSS, single-use 60-second connection tickets,
   bounded queues, revocation closing live streams, and relay failure leaving every PTY running.~~
   Built (§9); done once its smoke test (§14) passes here and on staging. With it, the gate's three
   checks have tests (§11): another account reaches nothing, a revoked device's live socket closes
   within seconds, and the relay failing leaves the program running and the person here in
   control. Production runs two Machines, which the relay cannot share yet (§13).
7. ~~**Resuming, and telling a device of control and the exit**: a replay window in the daemon,
   `resumeAfter` honoured, and the `terminal.control` and `terminal.exited` events.~~ Built (§10);
   done once its smoke test (§14) passes here and on staging.

## 13 · Risks

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
  for them (§14).
- **Staging sleeps.** The first request after it stopped waits for a Machine to start, and Neon's
  free compute to wake: a few seconds, not more, but enough to look like a failure. Neon also
  suspends its compute after a few idle minutes, which closes the connections the server keeps
  open: the log shows one `deadpool.postgres: Connection error: db error` warning when that
  happens (seen on 2026-10-25, with no request running), and the pool opens a new connection for
  the next request.
- **Pairing's short code is 40 bits.** It is only a way to find an offer, within one account,
  five tries at most, and an approval at the host still follows. A person reading it aloud near
  someone signed in to the same account is the case it does not cover; the QR code's secret is
  the stronger path.
- **The host learns of a request by asking.** Settings → Remote asks the server every three
  seconds while it is open; closed, nothing arrives until it is opened again. The relay link could
  tell the host instead; it does not yet.
- **A host's grant stays in force at the server if the host's last word is lost.** Revoking in
  the app drops the grant here first, so this host refuses the device at once whatever the
  server heard. Turning Remote off with the server unreachable leaves the host listed there until
  the person signs it out from another device; the app says so.
- **Grants, denials and expired requests are kept.** Nothing sweeps old rows of `grants` yet.
  They open nothing, and each needed an offer, which is rate-limited.
- **The deploy token lasts until 2046,** Fly's default. It deploys one app and nothing else, and
  lives only in the GitHub environment; replacing it is `fly tokens create deploy` again.
- **The self-host recipe runs ZITADEL's containers as root**, as ZITADEL's own Compose example does,
  so that the API can write the login client's token where the login container reads it.
- **The terminal attached last answers, which is a guess at where the person is.** Open in the
  app and in `ys attach` in another window, the latter answers, so an agent asking for the
  background colour learns that terminal's, not the app's theme, until it detaches.
- **A question not on the list reaches every viewer.** Two terminals that both answer it answer
  twice, as before. Kitty's graphics protocol is the known one, left alone because the same
  sequence draws the image.
- **Answers are told from typing by their shape, and only just after a question.** A key that
  sends the same shape in those two seconds (Shift+F3 is `CSI 1 ; 2 R`) is still typed, but does
  not take control.
- **A busy daemon of protocol 1 postpones sharing its terminals** until nothing runs in it and
  the app replaces it. Someone who always has an agent running would wait for a while; the
  remote side will have to say why a terminal cannot be shared.
- **A lease that lapses leaves its holder's size** until someone types or resizes here.
- **The relay is one process.** Tickets, host links and devices' sockets live in its memory, so a
  device and its host must reach the same instance. Staging runs one Machine; production's two
  need the relay to route a connection to the Machine holding the host's link (Fly's
  `fly-replay`, [28](28-remote-infrastructure.md) §2) before it relays anything.
- **Revocation from another process waits for the next check.** The operator's `revoke-account`,
  and a device signed out for reusing a refresh credential, reach live sockets within five
  seconds; revoking through this server's own routes reaches them at once.
- **A socket is let in by its ticket.** Its access credential lapsing later does not close it;
  only revocation does, which the checks catch.
- **The relay reads what it carries.** TLS ends at the server, which forwards terminal output and
  typing as it is. Nothing of it is logged or kept, and the app says it is not end-to-end
  encrypted.
- **Each check costs the records two queries per connected host**, every five seconds. Fine for a
  beta; a busy server would want the records to tell the relay instead.
- **The replay window is 256 KiB per watched session.** A device away for longer than that much
  output, or watching a full-screen program that repaints fast, gets a snapshot, which a slow link
  redraws in full. The window lives in the daemon's memory, so it costs up to 256 KiB for each
  session anyone has watched from another device, and goes with the session.
- **A device let in moments ago can be shown without its name** in `terminal.control`, for up to
  five seconds, until the channel reads the names again.
- **Whether an open relay socket keeps staging's Machine awake** is Fly's proxy's call, not yet
  watched. If it stops anyway, the host's next try wakes it within its minute of backing off.

## 14 · Smoke tests

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

**Slice 4, on this machine:**

1. `just server-up`, then `just server-e2e`. It ends with "End to end: passed", after the
   pairing round.
2. The app with a browser. `just server-run` in one terminal, and in another a throwaway profile
   with the preview switch and a credentials directory, so the real credential store is not
   touched:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys-credentials YARDSORT_DATA_DIR=/tmp/ys YARDSORT_WORKTREE_ROOT=/tmp/ys-wt just dev`.
3. Settings → Remote: server `http://127.0.0.1:8787`, a name, **Turn on and sign in**. Sign in as
   `developer@yardsort.test` with `Yardsort-dev-1`. It shows itself on.
4. **Pair a device**, then `just server-pair phone <code> --wait-for-revoke` and sign in again in
   the browser. Within three seconds the app shows the tool waiting. **Let it watch and type**:
   the tool reports being let in to watch and type on every project.
5. **Revoke** it: the tool reports the grant taken back.
6. Quit the app and start it again with the same line. Remote is still on, and **Pair a device**
   still works, from the kept credential.
7. **Turn off**. `/tmp/ys-credentials` is empty, and `just server-sign-in` lists the host's
   device as revoked.

**Slice 5, on this machine** (no part runs on the server, so there is no staging half):

1. A throwaway profile: `YARDSORT_DATA_DIR=/tmp/ys5 YARDSORT_WORKTREE_ROOT=/tmp/ys5-wt just dev`.
   Add a project and open a shell in a workspace.
2. In another terminal, `YARDSORT_DATA_DIR=/tmp/ys5 just cli attach`. In the shell, in either
   window, ask for the cursor position and show every answer that comes within a second:
   `stty raw -echo min 0 time 10; printf '\033[6n'; dd bs=1 count=64 2>/dev/null | od -c | head -2; stty sane`.
   One answer, `033 [ … R`. The released app answers twice.
3. Detach `ys attach` (`Ctrl-]`) and ask again in the app: still one.
4. `cargo run -p pty-ipc --example lease -- /tmp/ys5` and follow it. The app's terminal shows
   `typed-under-a-lease` once; typing in the app, then Enter in the tool, shows the lease lost,
   the controller `Local`, and the items numbered without a gap or a repeat.
5. Optionally, an update over a busy daemon of protocol 1: start the released app with
   `YARDSORT_DATA_DIR=/tmp/ys5-old`, open a shell, quit leaving it running, then
   `YARDSORT_DATA_DIR=/tmp/ys5-old YARDSORT_WORKTREE_ROOT=/tmp/ys5-old-wt just dev`. The shell is
   there and takes typing, no "still running under a previous version" appears, and
   `/tmp/ys5-old/daemon.log` is the old daemon's. `/tmp/ys5-old` is upgraded on the way: throw it
   away after.

**Slice 6, on this machine:**

1. `just server-up`, then `just server-run`.
2. The app, with the preview switch, a credentials directory and a throwaway profile:
   `YARDSORT_REMOTE_PREVIEW=1 YARDSORT_CREDENTIALS_DIR=/tmp/ys6-credentials YARDSORT_DATA_DIR=/tmp/ys6 YARDSORT_WORKTREE_ROOT=/tmp/ys6-wt just dev`.
   Add a project and open a shell in one of its workspaces.
3. Settings → Remote: server `http://127.0.0.1:8787`, a name, **Turn on and sign in** as
   `developer@yardsort.test` with `Yardsort-dev-1`. Within a few seconds it says "Connected to
   the relay. No device is connected now."
4. **Pair a device**, then
   `just server-pair phone <code> --relay --type 'echo hi from the phone' --wait-for-revoke`, and
   sign in again in the browser. **Let it watch and type.** The tool says it was welcomed, names
   the shell it watches, and types; the app's shell runs `echo hi from the phone`, and Settings →
   Remote says one device is connected. (Anything typed in the shell in the half minute before
   keeps control here, and the tool says it could not take it.)
5. Type in the app's shell. The tool keeps watching, and stops at nothing.
6. **Revoke** the tool's device in the app. Within five seconds the tool says the relay closed the
   socket because access was revoked, that it saw its items numbered without a gap, and what the
   shell drew after the snapshot it was attached with.
7. The relay failing: pair and run the tool again, without `--type`. While it watches, stop
   `just server-run` with Ctrl-C. The tool says the server is restarting; the app's shell still
   runs and takes typing; Settings → Remote says it is not connected and tries again. Start
   `just server-run` again: within a minute it says connected.
8. **Turn off**: the link closes first, and `/tmp/ys6-credentials` is empty.

**Slice 7, on this machine.** A fresh profile, so that a daemon of protocol 3 starts for it:
steps 1 to 3 of slice 6 with `/tmp/ys7`, `/tmp/ys7-wt` and `/tmp/ys7-credentials`. Then:

1. **Pair a device**, then
   `just server-pair phone <code> --relay --type 'echo hi from the phone' --wait-for-revoke`, and
   sign in again in the browser. **Let it watch and type.** The tool types, says "Control is now
   device" and then "Control is now nobody" as it lets go, and then "Came back after item N:
   resumed from there".
2. Type in the app's shell. The tool says "Control is now local"; about thirty seconds after the
   last key, with nothing else happening, "Control is now nobody".
3. Type `exit` in the app's shell. The tool says "The program ended, with code 0."
4. **Revoke** the tool's device. The tool says the relay closed the socket, and that it saw its
   items numbered without a gap, across the resume.

**Slice 4, on staging**, once it is deployed there: steps 3 to 7 with
`https://remote-staging.yardsort.sh` as the server, a staging account, and
`just server-pair phone https://remote-staging.yardsort.sh <code> --wait-for-revoke`; then
`fly logs -a yardsort-remote-staging` holds no `ys1.`, code or credential.

**Slice 6, on staging**, once it is deployed there: steps 3 to 6 and 8 with
`https://remote-staging.yardsort.sh` as the server, a staging account, and
`just server-pair phone https://remote-staging.yardsort.sh <code> --relay --type 'echo hi' --wait-for-revoke`.
The link holds over Fly's proxy for a few minutes with nothing happening (its pings keep it), and
`fly logs -a yardsort-remote-staging` holds no `ys1.`, code, ticket or anything typed.

**Slice 7, on staging**, once slice 6 is deployed there: its steps above with
`https://remote-staging.yardsort.sh` as the server and a staging account. Nothing of slice 7 runs
on the server, so this checks that resuming and the events come through Fly's proxy as they do
here.
