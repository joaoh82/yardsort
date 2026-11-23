# 50 · Which account

Built on 2026-11-23. Every Remote sign-in asks the person which account to sign in as, and each
device shows the account it signed in to. Server, desktop, headless host and phone.

## What happened

On 2026-11-22 the owner's phone showed their computer offline, and every pairing code from that
computer was refused as "wrong or has expired". Staging's records said why: the server had two
accounts. The load test ([46](46-remote-r7.md) §13) had signed devices in through this computer's
browser as a throwaway ZITADEL user, and the browser kept that session. The desktop's next
sign-ins, its client device at 18:16 UTC and two host turn-ons that evening, went to the identity
provider, which saw a session and answered at once, as the throwaway user. The phone signed in
through its own browser as the owner. A code is found only in the account of the device redeeming
it (`redeem_offer` takes the caller's account), and a device sees only its own account's hosts, so
the phone found nothing, and showed the owner's earlier host, which nothing linked to any more, as
offline. Neither side said which account it was in, so "both signed in, same server" looked right.

## What was decided

Asked of the owner on 2026-11-23:

- **Every sign-in asks which account** (`prompt=select_account`), from every client: the desktop's
  host and other-computers sign-ins, `yardsort-host`, the phone. ZITADEL then lists the browser's
  sessions and offers another. One rule at the server, no protocol change. A sign-in that confirms
  deleting the account keeps `prompt=login` with `max_age=0` ([46](46-remote-r7.md)). Not chosen:
  only the desktop's sign-ins, through a new `LoginParams` field and a protocol minor.
- **Each device shows the email it signed in as, and the server keeps none.** The server asks for
  `openid email` and reads `email` from the ID token, or, as ZITADEL leaves it out of the ID token by
  default when it also issues an access token (OIDC Core §5.4), from the userinfo endpoint, once,
  with that access token, which is then dropped. The userinfo answer counts only when its `sub` is
  the ID token's. The email rides in the handoff (`handoffs.email`, migration 0015), which is deleted
  when the device redeems it or after minutes, and goes to the device once, in the sign-in's
  `Credentials.email`. A refresh never carries it. Not chosen: a short code made from the account
  id on every device (no email anywhere, but nothing a person recognises), and both.

An email that is not one line of at most 254 bytes with an `@` in it is dropped, not cut, on the
server (`remote_proto::account::shown_email`) and on the phone (`shownEmail`), and the sign-in goes
ahead without one. It is for showing only: nothing decides anything by it, and two accounts may have
the same email when the provider has two users with it.

## Where it shows

- **Desktop**, Settings → Remote: **Account:** under this computer as a host, and under **Other
  computers** for its client sign-in, each from its own sign-in (`remote_host.email`,
  `remote_computer.email`, core migration 0047). They can differ, and that is what it is for.
- **`yardsort-host`**: `sign-in` prints it, and `status` says it. The dashboard does not yet.
- **Phone**: the account screen, under the server and device name, and the pairing screen, beside
  the code: "This phone is in the account … The computer must be signed in to the same account, or
  its code is not found." Kept in the profile (`Profile.email`).

A sign-in from before this has no email, and says the account is not known here; it shows from the
next sign-in. An older server sends none, an older client ignores the field: no protocol minor.

## What the server holds

Still an account is `(server, issuer, subject)`, and the records hold no email: export, deletion
and the journal are unchanged. The email exists on the server only in a handoff, for the minutes
between the provider's answer and the device redeeming it, and so may be in a database backup taken
in that window.

## Checks

`crates/server/tests/sign_in.rs`: the authorize request asks for `select_account` and `openid email`;
the email reaches the device from the ID token, or from userinfo with one call; a refresh never
repeats it; userinfo for another subject, no email, and ones that cannot be shown sign the person in
without one. `tests/records.rs` keeps it with the handoff on both stores. `tests/computer.rs`: a
computer's sign-in through the real server keeps the provider's email in `remote_computer`. The
TypeScript client's fixture pins `email` on the wire and drops a bad one. The desktop's and the
phone's screens show it, and say when it is not known.

Not yet run against ZITADEL itself: staging's sign-in, after this server is deployed there, is the
check that ZITADEL answers userinfo with the email and shows its account picker.
