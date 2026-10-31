# @yardsort/remote-client

A device's side of the Yardsort server and remote protocol, in TypeScript: signing in, pairing,
and reaching a computer's workspaces, activity and terminals through the server's relay. It is the
twin of the Rust crates `remote-proto` and `remote-client`, which are the source of truth, and it
is what the phone app (`apps/mobile`, R3) will use for everything it says to a server. Screens never
build a request themselves.

It has no dependencies. It runs wherever there are `fetch` and `WebSocket`: on a phone, in Bun, in
a browser. Randomness and SHA-256 come from Web Crypto where there is one, and from what the app
passes as `crypto` where there is not (React Native). Addresses are read without `URL`, which React
Native implements only in part.

The plan, and why each decision went the way it did, is [docs/design/31](../../docs/design/31-remote-r3.md).

## What is in it

| File              | What                                                                                                                                                                                                                                                                                                                                                       |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/protocol.ts` | The remote protocol's types and decoders: ids, `Hello`, requests, replies, events, terminals, grants' scopes.                                                                                                                                                                                                                                              |
| `src/account.ts`  | Signing in, credentials, devices, computers, pairing, grants; pairing codes and `yardsort://pair` links.                                                                                                                                                                                                                                                   |
| `src/relay.ts`    | Tickets, the relay's greeting and notices, and its close codes.                                                                                                                                                                                                                                                                                            |
| `src/frame.ts`    | Terminal output frames, the binary half of the wire.                                                                                                                                                                                                                                                                                                       |
| `src/client.ts`   | `Client`: one server over HTTPS. `beginSignIn`, `signIn`, `refresh`, the device, host, pairing and grant calls, tickets. `codeFromRedirect` reads what the browser brings back.                                                                                                                                                                            |
| `src/session.ts`  | `Session`: credentials refreshed before they lapse, one refresh at a time, each new refresh credential kept before its access is used.                                                                                                                                                                                                                     |
| `src/channel.ts`  | `openChannel`: one computer through the relay. Requests matched to replies; `attach` (with resume) checking every item's number; `acquire`, giving a `Control` that renews its lease and numbers its input; `subscribe` to activity. A channel that sends nothing for 20 seconds asks `host.describe`, and one with no answer in 10 is closed as `silent`. |
| `src/revoked.ts`  | `explainRevoked`: what a 4003 took away, asked of the server: this computer's grant (`unpaired`), this device (`signedOut`), or nothing it could tell (`undetermined`).                                                                                                                                                                                    |
| `src/decode.ts`   | The decoding primitives. A refusal names where, never what: what arrives may be a credential or a person's own text.                                                                                                                                                                                                                                       |
| `tools/pair.ts`   | A pretend phone through this package alone, the twin of `just server-pair phone`: `just client-pair [SERVER] CODE`, and `just client-pair devices [SERVER] [--revoke DEVICE]`.                                                                                                                                                                             |

Everything from a computer is decoded as untrusted, though it comes from one the person paired. Ids
are checked, and names, labels, notes, links and lists are held to the protocol's limits. Strings
the protocol does not bound are held to the message's own size, 64 KiB. Numbers past 2^53 − 1,
which a JavaScript number cannot hold exactly, are refused rather than rounded. Fields a newer peer
adds are dropped, and an event or a frame of a kind this build does not know is passed over. A
credential, ticket or code is a `Secret`, which prints as `Secret(…)`.

## Tests

```sh
just test-client     # or, here: bun test
```

- `test/fixtures.test.ts` reads every file in `crates/remote-proto/fixtures/v1/`, decodes it,
  writes it back and requires the same JSON, and repeats what the Rust test of each asserts. A
  fixture this table does not read fails the run, as one the Rust tests do not read fails theirs.
  `frame-output.json` and `frame-refused.json` pin the binary frames on both sides.
- `test/server.test.ts` and `test/pair-tool.test.ts` run against the real server, its sign-in
  through a stand-in identity provider, and a real host with a program in a real terminal:
  `crates/server/examples/stand-in.rs`, started with Cargo. The first run builds it.
- `test/channel.test.ts` puts the channel against a computer that misbehaves on purpose, over a
  real WebSocket: going quiet, skipping a number, closing with each code.

## License

[MPL-2.0](LICENSE), unlike the rest of the repository: see
[docs/design/31 §6](../../docs/design/31-remote-r3.md#6--the-license-43). Anyone who ships a changed
file of it has to publish that file. It can be used beside closed code, and an app built with it
can be distributed under other terms, an app store's among them, while its own source stays under
the MPL.
