# Remote: where it runs, and how to set it up

**Status:** staging was set up and deployed on 2026-10-24, and has answered at its own name since
2026-10-25 (§5); production is not set up. Prices were checked on 2026-10-22 and change often; check them again
before relying on them. The design this hosts is
[27](27-remote-r2.md). Setting the accounts up is the owner's to do (proposal §16: production
choices, credentials and paid provisioning are owner-supplied). Section 5 is the checklist for it.

## 1 · Environments

| Environment    | Runs on                                 | Identity provider                                     | Database                  | Who uses it                     |
| -------------- | --------------------------------------- | ----------------------------------------------------- | ------------------------- | ------------------------------- |
| **Local**      | Docker Compose on a developer's machine | ZITADEL in a container, throwaway                     | PostgreSQL in a container | Development and tests           |
| **Staging**    | Fly.io `fra`, one small Machine         | ZITADEL Cloud, the free instance                      | PostgreSQL (see §2)       | The team, and beta testers      |
| **Production** | Fly.io `fra`, two Machines              | ZITADEL Cloud Pro, its own instance and custom domain | Fly Managed Postgres      | Everyone on the managed service |

Self-hosters run the same server image with the self-host recipe (R2's third slice): their own
ZITADEL, their own PostgreSQL, behind their own reverse proxy. Nothing in it calls our
infrastructure.

Each environment has its own identity provider instance or application, its own database and
its own secrets. Staging never reads production data, and a staging credential opens nothing in
production.

Every environment runs the same server, built from this repository: production runs the public
`yardsort-server` unmodified, and anything only the managed service has is a separate, private
program behind a documented interface ([30 §2](30-open-source-and-managed.md#2--the-managed-service-runs-the-public-server-unmodified)).

## 2 · Where each part runs

**The Yardsort server on Fly.io.** One binary serves sign-in, the device routes and, from slice
6, the relay. The relay holds WebSocket connections for as long as hosts and phones stay
connected, which rules out request-scoped serverless platforms such as Cloud Run or Lambda. Fly
runs a container as a long-lived Machine and proxies WebSockets, terminates TLS and issues
certificates. It bills by the second, and its private network reaches its managed PostgreSQL.
`flyctl` is already installed here. A deploy restarts Machines and so drops live connections:
hosts and devices reconnect, which the protocol is built for (a fresh snapshot, never a replay).

**PostgreSQL.** Two choices:

- **Fly Managed Postgres**: high availability, backups and connection pooling on every plan, and
  reachable over Fly's private network. Basic, with shared CPUs and 1 GB, is US$38 a month plus
  US$0.28 per GB of storage. This is the choice for production.
- **Neon**: usage-based, with a free plan, and US$0.106 per compute-hour on Launch. It is cheaper
  for a staging database that is mostly idle, but outside Fly's network, so it is reached over
  the internet with TLS.

Staging uses Neon's free plan (settled 2026-10-24), and production will use Fly Managed Postgres.

**Identity on ZITADEL Cloud.** Staging uses the free plan: one instance, 100 daily active users,
and a `*.zitadel.cloud` domain. Production needs Pro (US$100 a month): a second instance needs
it, and so do a custom domain and the uptime guarantee. With sign-in only (27 §2), each device
counts on the days its person signs in, not every day it is used. Data locations are the United
States, the European Union, Switzerland and Australia, all on Google Cloud. ZITADEL's own pages
disagree on whether the free plan may choose one. Put the instance in the same part of the world
as the server.

**Everything else:**

- **DNS:** `yardsort.sh` is on Cloudflare, already the owner's.
- **The website:** stays on Vercel.
- **Email:** for now, ZITADEL sends its own verification and invitation emails. Our own sender
  comes later, for branding.
- **Push:** Apple's and Google's push services, in R4.
- **Monitoring:** Fly's built-in metrics and logs to begin with. Error tracking and a status page
  come before the public beta.

**Considered and not chosen:**

- **A plain VPS** (Hetzner and similar) is cheapest per CPU. But we would run PostgreSQL, its
  backups and failover, TLS and deploys ourselves. Hetzner's prices also rose sharply in 2026: a
  CPX22 was about €19.50 a month in August, and the cheapest CX line was not orderable.
- **Render** (the owner chose Fly.io over it on 2026-10-23) would also work, and is simpler to
  operate: deploys from a Git push, a clear
  dashboard, preview environments. It loses on three things this server needs (checked
  2026-10-23):
  - **Relay traffic costs more.** Render's Pro workspace includes 25 GB of outbound traffic,
    then US$0.15 per GB. Fly charges US$0.02 per GB in Europe and North America. Relay traffic
    is the cost that grows with use.
  - **No routing to a chosen instance.** Render's load balancer gives each new connection to a
    random instance. Once the relay runs on more than one instance, a host and the phone
    reaching it may land on different ones, and the instances would need a message bus between
    them. On Fly, the server can answer `fly-replay` to send a connection to the Machine that
    holds the host's.
  - **One region per service, and one in Europe.** Render has Frankfurt alone in Europe, and
    each region is its own private network. Fly offers Managed Postgres in Frankfurt,
    Amsterdam and London, and one app can run Machines in several regions.

  With high availability, Render's PostgreSQL needs a Pro plan (US$55 a month for 1 CPU and
  4 GB) and bills the standby at the same price again. That makes production on Render about
  US$150 a month before ZITADEL, against about US$50 on Fly. Both status pages, and outside
  trackers, show regional incidents on both platforms through 2026: neither record is clean.
  Either way, local processes survive the server going away (proposal §18, "Important invariants").

- **Railway** was ruled out by the owner after its instability in October 2026.
- **AWS or Google Cloud directly** give the most control and need the most operations, which a
  team this size should not spend yet.

## 3 · What it costs to start

| Item                                               | Staging          | Production at launch                          |
| -------------------------------------------------- | ---------------- | --------------------------------------------- |
| Fly Machines (shared-cpu-1x, 512 MB, US$3.69 each) | 1 → ~US$4        | 2 → ~US$7.50                                  |
| Fly outbound traffic (US$0.02/GB in NA and EU)     | ~US$0            | Relay traffic: measure in beta (proposal §12) |
| Dedicated IPv4                                     | US$0 (shared)    | US$2                                          |
| PostgreSQL                                         | Neon free: US$0  | Fly Managed Postgres Basic: US$38 + storage   |
| ZITADEL Cloud                                      | Free: US$0       | Pro: US$100                                   |
| Domain and DNS                                     | the owner's      | the owner's                                   |
| **About**                                          | **US$5 a month** | **US$150 a month**, before relay traffic      |

## 4 · Names

The domain is `yardsort.sh`. The website keeps `www.yardsort.sh` on Vercel.

| Name                         | What                                                                                                                |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `remote.yardsort.sh`         | The production server, which the app and the phone connect to: sign-in, devices, and later the relay                |
| `remote-staging.yardsort.sh` | Staging                                                                                                             |
| `auth.yardsort.sh`           | Production's ZITADEL custom domain (Pro): the sign-in pages people see. Staging keeps its `*.zitadel.cloud` address |
| `<server>/v1/login/callback` | The one redirect URI registered at ZITADEL for each environment (27 §2)                                             |
| `yardsort://signed-in`       | The app's redirect, allowed by the server, never registered at ZITADEL                                              |
| `http://127.0.0.1/signed-in` | A computer's redirect, on any port                                                                                  |

Fly app names are global, so the ones below may need a suffix.

## 5 · Setting it up: the owner's checklist

**Staging, done on 2026-10-24.** Each service has its own Yardsort organization, apart from
anything else its owner runs there, and Yardsort's own project or app inside it:

| Service       | What exists                                                                                                                                                               |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ZITADEL Cloud | Instance `yardsort-staging` in the EU (`https://yardsort-staging-pegnzh.eu1.zitadel.cloud`), organization Yardsort, project `Yardsort server`, Web application `Yardsort` |
| Neon          | Organization Yardsort, project `yardsort` in `aws-eu-central-1` on PostgreSQL 17, database and role `yardsort`                                                            |
| Fly.io        | Organization `yardsort`, with its own card; apps `yardsort-remote-staging` (one Machine in `fra`, since 2026-10-24) and `yardsort-remote` (empty)                         |
| Fly secrets   | On staging: `YARDSORT_OIDC_CLIENT_SECRET` and `DATABASE_URL` (Neon's direct connection, `sslmode=require`)                                                                |
| GitHub        | Environment `staging`, holding `FLY_API_TOKEN`: deploy token `github-staging`, for the staging app alone                                                                  |
| Cloudflare    | In `yardsort.sh`, DNS only: `A remote-staging` to `66.241.124.66` (Fly's shared address) and `AAAA remote-staging` to `2a09:8280:1::1cb:3339:0` (staging's own)           |

The issuer and the client id are in `crates/server/fly/staging.toml`. No secret passed through a
file, an issue or a chat: each went from its provider straight into Fly or GitHub. The two DNS
records were added by hand in Cloudflare's dashboard on 2026-10-25, and Fly's certificate was
issued a minute later.

Do these in order. Nothing secret goes into git, an issue or a chat. Each secret goes straight
from the provider's screen into Fly's secrets or a GitHub environment secret.

**A. Decide**

Settled on 2026-10-23:

- **Region: Europe.** Fly `fra` (Frankfurt), where Managed Postgres is offered
  (`fly platform regions`), and ZITADEL's European Union location.
- **Hosting: Fly.io**, over Render (§2).
- **Domain:** `yardsort.sh`, with the names in §4. Its DNS is on Cloudflare.
- **ZITADEL:** the Cloud free plan to begin with, and Pro once there is something to launch.

**B. ZITADEL Cloud, for staging first**

1. Sign up at zitadel.com. That creates a Customer Portal account and a team. Turn on MFA or a
   passkey for it.
2. **Create Instance**: name it `yardsort-staging`, in the chosen region if offered. Create its
   first administrator, with a passkey.
3. In the instance console, create a **project**, `Yardsort server`.
4. In the project, create an **application**:
   - **Type:** **Web**.
   - **Authentication method:** **Code**, so the server authenticates with a client secret.
     The server sends PKCE as well, which ZITADEL accepts alongside the secret: confirmed against
     ZITADEL v4.19.5 in the local environment on 2026-10-23 ([27](27-remote-r2.md) §16).
   - **Redirect URI:** `https://remote-staging.yardsort.sh/v1/login/callback`.
   - **Development mode:** off.
   - **Refresh tokens:** leave them off. The server never asks ZITADEL for one.
5. Copy the **issuer**, exactly as the instance shows it (`https://yardsort-staging-xxxx.zitadel.cloud`),
   and the **client ID**. These two are not secret. Put the **client secret** straight into Fly
   (step C3).
6. **Login settings:** turn off self-registration for the beta, and invite testers from the
   console. Allow passkeys.

**C. Fly.io**

1. Create or choose the organization for Yardsort, with a payment card. Every member uses
   two-factor authentication.
2. Create the apps without deploying them: `fly apps create yardsort-remote-staging`, and
   `yardsort-remote` for later.
3. Set staging's secrets. `fly secrets import` reads them from stdin, ending with Ctrl-D, so
   they stay out of shell history:

   ```sh
   fly secrets import -a yardsort-remote-staging   # then paste, one per line:
   # YARDSORT_OIDC_CLIENT_SECRET=…
   # DATABASE_URL=…
   ```

4. Create a deploy token for CI that can deploy that app and nothing else:
   `fly tokens create deploy -a yardsort-remote-staging`.

**D. Database**

- **Staging:** create a Neon project in Frankfurt (AWS `eu-central-1`), with one database
  `yardsort`. Use its direct connection string, not the pooled one, as `DATABASE_URL`, keeping
  `sslmode=require`: the server keeps its own pool of eight connections per instance, listens for
  its instances' signals on one more, which a pooler would not carry, and makes its tables itself
  when it first starts.
- **Production, later:** `fly mpg create`, on the Basic plan, in the primary region.

**E. DNS**

`fly certs add remote-staging.yardsort.sh -a yardsort-remote-staging`, then add the records it
prints in Cloudflare, set to **DNS only** (the grey cloud), not proxied:

- Fly issues and renews the certificate itself, and its check has to reach Fly.
- Proxied, every connection would come from Cloudflare's addresses. The server's per-address
  rate limits would then count Cloudflare, not the client.

`fly certs show remote-staging.yardsort.sh -a yardsort-remote-staging` says when the certificate
is issued.

**F. GitHub**

Create a `staging` environment on the repository, with the deploy token as `FLY_API_TOKEN`.
Later, create a `production` environment that requires a reviewer.

**G. Hand back**

Pass these back to whoever builds the deploy (slice 3):

- the Fly app names, if a suffix was needed;
- the ZITADEL issuer and client ID.

None of them is a secret.

**Production, at launch:**

- Do B again on Pro: a `yardsort-production` instance with `auth.yardsort.sh` as its custom domain.
- Do C, D and E again for `yardsort-remote`.

## 6 · What gets built, and when

Section 5 is the owner's. The rest is code, in R2's next slices, and nothing is deployed without
the owner's go-ahead:

- **Slice 2, done on 2026-10-23** ([27](27-remote-r2.md) §5): the `yardsort-server` binary,
  its PostgreSQL store, configuration from the environment and secret files, and the local
  Docker environment with `just` recipes (CONTRIBUTING, "Working on the server").
- **Slice 3, built on 2026-10-24** ([27](27-remote-r2.md) §6): the container image, staging's Fly
  config (`crates/server/fly/staging.toml`), a workflow that deploys it on request
  (`.github/workflows/server-deploy.yml`), and the self-host recipe (`crates/server/self-host/`).
  Staging was first deployed from a workstation, with the workflow's arguments, before the
  workflow reached `main`. Step E and staging's smoke test followed on 2026-10-25, and the test
  passed ([27](27-remote-r2.md) §16). The workflow first deployed it from Actions on 2026-10-25.
  Production's Fly config and its protected environment come with its ZITADEL instance.
- **Slice 4, built on 2026-10-25** ([27](27-remote-r2.md) §7): pairing, on the server and in the
  app behind `YARDSORT_REMOTE_PREVIEW=1`. Its migration `0002_hosts_and_grants.sql` reaches
  staging with its next deploy; nothing else about where it runs changes.
- **Slice 6, built on 2026-10-28** ([27](27-remote-r2.md) §9): the relay, on the same server.
  It holds a WebSocket for every connected host and device, so a connected host keeps staging's
  Machine busy. It keeps its tickets and sockets in memory, so it runs on **one Machine**:
  staging already does, and production's two need the relay to send a connection to the Machine
  holding the host's link (`fly-replay`) first. No migration, no new secret, no new setting.
- **Slice 7, built on 2026-10-29** ([27](27-remote-r2.md) §10): resuming a stream, and telling a
  device of control and the exit. All of it is on the host, and the server relays it unchanged:
  the server itself does not change. Staging needs slice 6 deployed for its smoke test.
- **Slice 8, built on 2026-10-29** ([27](27-remote-r2.md) §11): following activity. Also all on
  the host; the server does not change, and staging needs slice 6 deployed for its smoke test.
- **Slice 9, built on 2026-10-29** ([27](27-remote-r2.md) §12): the relay on more than one
  Machine, which production's two need. Each instance notes which host links it holds, in the
  migration `0003_relay_links.sql`, and a device's ticket request and socket that reach another
  Machine are answered `fly-replay`, so Fly's proxy sends them to the right one. Staging's config
  turns it on (`YARDSORT_ROUTING=fly-replay`) and still runs one Machine; its smoke test adds a
  second for a while, which needs the owner's go-ahead. No new secret. The self-host recipe stays
  one instance.
- **Slice 10, built on 2026-10-29** ([27](27-remote-r2.md) §13): a host told over its link that
  a device asks to pair. One more query per instance every five seconds; no migration, no new
  secret, no new setting.
- **Slice 11, built on 2026-10-29** ([27](27-remote-r2.md) §14): signals between instances, as
  PostgreSQL notifications, so a revocation reaches every instance at once. Each running server
  holds one more database connection, to listen, beside its pool of eight. Listening needs a
  direct connection, which `DATABASE_URL` already is on staging: through a pooler that hands out
  a connection per transaction, as Neon's pooled string does, signals would never arrive, and
  only the five-second rounds would remain. The server notices within a minute and logs "records:
  not listening for signals: its own probe did not come back…". No migration, no new secret, no
  new setting.
- **Slice 12, built on 2026-10-29** ([27](27-remote-r2.md) §15): rate limits shared across
  instances, in a new table, `rate_limits` (migration `0004`, applied as the server starts), with
  one more write per limited request; each instance sweeps its full buckets every minute. A host
  link replaced on another instance closes at once, by one more kind of signal. No new secret, no
  new setting.

## 7 · Open decisions

- When "something to launch" is, for ZITADEL Pro: the public beta, or launch (R7).
- Whether beta testers sign up for themselves or are invited.
- Whether DNS records stay a hand step in Cloudflare's dashboard or a script does them, with an
  API token limited to editing `yardsort.sh`'s DNS. By hand for staging; production adds
  `remote` and `auth`.
