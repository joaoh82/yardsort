# Open source and the managed service

**Status:** decided on 2026-10-28 by the owner: the server is AGPL-3.0-only from this change on,
and the rest of the repository stays GPL-3.0-only (§1). The rule for the managed service (§2) and
where things go (§3) hold from now on. The entitlements seam (§4) and the private repository are
built at R7, the managed launch. This is the project's own reading of the licenses, not legal
advice. Have it checked by counsel before R7.

The proposal says "Keep the project open source. Sell managed operation and convenience" and
"Managed and self-hosted servers expose the same core protocol"
([25 §1](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md#1-outcome-and-settled-decisions)).
This document says how. The server is public because self-hosting it is the promise. What is
sold is running it. What must stay private is a short, specific list, and none of it exists yet.

## 1 · Licenses

| Part                                                                                                             | License       | Where it says so                        |
| ---------------------------------------------------------------------------------------------------------------- | ------------- | --------------------------------------- |
| The server: `crates/server` (`yardsort-server`), its image, its Fly configs and the self-host recipe             | AGPL-3.0-only | `crates/server/LICENSE`, its Cargo.toml |
| Everything else: the app, the core, `ys`, the PTY crates, `remote-proto`, `remote-client`, the website, the docs | GPL-3.0-only  | `LICENSE`, the workspace's Cargo.toml   |

**Why the server is AGPL.** The GPL's obligations follow distribution: whoever gives someone a
copy must give them the source too. A hosted service gives nobody a copy, so under the GPL anyone
could change the server, run it as a competing service, and keep the changes closed. The AGPL adds
one condition, its section 13: whoever runs a _modified_ version for users over a network must
offer those users its source. Someone self-hosting the server unchanged owes nothing beyond what
the GPL asks. Someone who changes it and lets others use it shares the changes. The identity
provider the recipe uses, ZITADEL, made the same choice in 2025 ([27 §1](27-remote-r2.md)).

**Why nothing else is.** The network condition matters only for software others use as a
service. The app, `ys` and the phone run on the person's own device. `remote-proto` and
`remote-client` are linked into the app, so AGPL there would reach into the app for nothing.

**Why it could be done now.** Changing a license needs the agreement of everyone who holds
copyright in the code. On 2026-10-28 every commit touching `crates/server` was the owner's, with
an AI assistant as co-author. Once others contribute to it, a change would need each of them.

**Mixing the two.** The server links `remote-proto`, which is GPL-3.0-only. Section 13 of each
license allows exactly that combination. Each part keeps its own license, and the server's network
condition applies to the server's part. The reverse would be a problem: anything outside
`crates/server` that depended on `yardsort-server` would bring the AGPL into the app or `ys`. So
nothing may. `crates/server/tests/license.rs` checks that, and that the crate says AGPL and ships
the license's text.

## 2 · The managed service runs the public server, unmodified

Production runs `yardsort-server` built from this repository's `main`, the same as a self-hosted
one. No private patches, no private fork, and no private code compiled into it.

- **It keeps the proposal's promise.** A self-hosted server is the managed one, minus the
  operator.
- **It keeps the AGPL simple for the project itself.** A licensor is not bound by their own
  license. But once others hold copyright in the server, a modified managed build would owe its
  source to every user, like anyone else's. Running it unmodified owes nothing either way.
- **No contributor agreement is needed.** Contributions come in under the AGPL, and the managed
  service uses them under the same AGPL as everyone. The project never needs more rights than
  that, so it never has to ask for them.

So anything only the managed service has is **a separate program the server talks to**, over an
interface documented in this repository. It is switched on by configuration. With it unset, the
server works fully, as a self-hosted one does.

## 3 · What goes where

| Public: this repository                                                                     | Private: `yardsort-cloud`, created at R7                                            | In no repository                                            |
| ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| The protocol, the client, the server: accounts, devices, pairing, and later the relay, sync | Billing: the payment provider's checkout, webhooks and invoices                     | Secrets and credentials of any kind                         |
| The self-host recipe, and the server's image                                                | The entitlements service: plans, quotas and grace periods, answering §4's interface | Customer data, and production database dumps                |
| Fly configs for staging and production, with their non-secret settings only                 | The support and admin console                                                       | `.env` files, and anything copied out of a provider's panel |
| The deploy workflows, the operator commands (`revoke-account`), the design docs             | Abuse and fraud rules, which stop working once they can be read                     |                                                             |
| The entitlements _interface_, and its "everything is allowed" default                       | Runbooks and incident notes with internal detail; pricing and business analytics    |                                                             |

A public production config is not a risk: like `crates/server/fly/staging.toml` it holds names
and settings anyone could see from outside. Secrets go straight from the provider into Fly or
GitHub, and never pass through a file, a chat or a log ([28 §5](28-remote-infrastructure.md)).
Production's deploy uses a GitHub environment that requires a reviewer and deploys from `main`
only, so a fork or a pull request never reaches its token.

## 4 · The entitlements seam: a sketch for R7

The proposal
([§12](Yardsort-Remote-Mobile-Sync-Spec-2026-10-21.md#12-managed-service-and-self-hosting)):
"Entitlements are checked on the server, never trusted from the client. Subscription changes
affect future managed service access according to a documented grace period. They must not
delete local work, terminate local agents, or prevent export of user-owned synced records."

- **The question the server asks.** Before something that costs the operator, it asks whether
  this account may do it: signing in another device past a limit, opening a relay stream,
  storing sync data. It asks nothing for reading what it already holds, or for revoking, removing
  or exporting.
- **Unset, everything is allowed.** With no entitlements service configured (a setting such as
  `YARDSORT_ENTITLEMENTS_URL`), the server allows everything up to its own built-in limits, such
  as 50 devices an account. That is how self-hosted servers run.
- **Set, it asks the private service.** It sends the account's id (the server's own id, not the
  provider's subject), the action, and the counts the decision needs. It sends no email address,
  name or credential. Answers are cached briefly.
- **What an unreachable service means** is decided at R7, within the proposal's rule. Lean: what
  was allowed stays allowed for the grace period, and only new things wait.
- **Billing never touches the server.** The payment provider's webhooks go to the private
  service. The server holds no payment data and does not know prices.

## 5 · Before the server is published as an image or a binary

Today the image is built on Fly's builder and never published, so nobody is given a copy. When the
self-host recipe offers a prebuilt image or binary, each copy is a distribution, and it must carry:

- the AGPL's text for the server and the GPL's for `remote-proto`;
- where its source is: the OCI labels `org.opencontainers.image.source` and
  `org.opencontainers.image.licenses`.

The server could also name its source in `/v1/server`. That helps self-hosters who change it to
offer theirs, but it is not required of an unmodified server.

## 6 · The phone app: a question for R3

The phone app (R3) will be published through Apple's App Store and Google Play. The Free Software
Foundation's view is that the App Store's terms add restrictions the GPL forbids. VLC was
withdrawn from it in 2011 over exactly that. As the only copyright holder, the owner can publish
anyway. Code others contribute under the GPL alone would not allow it. Decide before anyone else
contributes to the phone app: [open question 43](06-open-questions.md#remote).
