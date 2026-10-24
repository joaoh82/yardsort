# Remote, R2: identity and relay

**Status:** R2 started on 2026-10-22. The first slice is built: signing in through the server's
identity provider, and the server's own credentials for each device after that. It is a library
(`crates/server`): nothing listens, and nothing in the app or `ys` uses it yet. The proposal is
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
provider running beside the server in development. There is no switch to skip certificate checks.

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

## 5 · Verification

On Linux, 2026-10-22:

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

**Not run here:** PostgreSQL, since this machine has no server and no access to Docker; a real
ZITADEL; macOS and Windows, which CI covers; anything on a phone.

## 6 · The rest of R2

Each item is a later slice. The gate (proposal §13): two-account isolation, a revoked device
losing a live stream, and relay failure leaving PTYs alive.

1. ~~Sign-in and device credentials, as a library.~~ This slice.
2. **PostgreSQL records and the HTTP listener**: `/v1/server`, `/v1/login`, the callback,
   `/v1/token` and the device routes, with rate limits on starting sign-ins and on the token
   endpoint. The same tests are run against PostgreSQL. Needs a PostgreSQL to test against, here
   and in CI.
3. **The self-host recipe**: Compose with ZITADEL (pinned), the server, PostgreSQL and a reverse
   proxy, with TLS, backup and upgrade notes, and one end-to-end sign-in against a real ZITADEL
   in CI. A `yardsort-server` command revokes every device of an account (§7).
4. **Host enrollment and grants**: a host identity persisted when Remote is enabled, enrollment
   by QR secret or human code (hashed, five minutes, rate-limited), approval at the host, grants
   from that approval, and where a headless host keeps its credential
   ([open question 35](06-open-questions.md#remote)).
5. **The daemon, `PROTOCOL` 2**: one controller lease and one query responder enforced in the
   daemon, and numbered output over its IPC.
6. **The relay and the host's outbound connector**: WSS, single-use 60-second connection tickets,
   bounded queues, revocation closing live streams, and relay failure leaving every PTY running.

## 7 · Risks

- **Nothing limits how fast sign-ins are started or codes redeemed.** Pending sign-ins lapse
  after ten minutes, and `MemoryRecords` drops lapsed ones as it adds new ones, but a flood is
  unbounded until the listener (slice 2) limits by source.
- **Refresh reuse signs a device out even when the device is honest**, when the answer carrying
  its new credentials was lost in transit after the server rotated them. That is the standard
  trade, and a sign-in fixes it. A short grace window would trade some of the protection away;
  [open question 38](06-open-questions.md#remote).
- **An access credential is a bearer credential.** A copy works until its quarter hour is up,
  or until the device is revoked, which takes effect on the next request.
- **`last_seen_at` is written on every authenticated request.** The PostgreSQL store should
  write it at most once a minute.
- **Disabling a person at the provider does not sign their devices out.** The provider is asked
  once per device, at sign-in, and each refresh starts the 90 days again, so a device in use
  keeps working for as long as it stays in use. That follows from asking the provider only at
  sign-in (§2), which is what keeps a provider that bills by daily active user cheap. Until it
  is answered, the operator's path is revoking that account's devices; slice 3 adds a
  `yardsort-server` command for it, by issuer and subject, beside the self-host recipe. A
  maximum session length, after which a device has to sign in again however often it is used, is
  the other answer; it costs a sign-in every so often and is not chosen yet
  ([open question 39](06-open-questions.md#remote)).
- **Spent refresh credentials must outlive their rotation.** A store that dropped a spent one at
  once would answer a stolen copy with "unknown" and leave its holder signed in, instead of
  revoking the device. The `Records` contract says so; `MemoryRecords` keeps everything, and the
  PostgreSQL store (slice 2) may drop one only after its own expiry.
- **Account deletion and export** (§9) are not built.
- **`jsonwebtoken`'s pure-Rust backend brings the `rsa` crate**, which has an open timing
  advisory (RUSTSEC-2023-0071) on operations with a private key. The server only verifies, with
  the provider's public keys; only the tests sign. Nothing in CI audits dependencies yet.
- **The provider's own `error` text** comes through the browser. It is bounded to 64 characters
  and only reaches the server's log, never the device.
