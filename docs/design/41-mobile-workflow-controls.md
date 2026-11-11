# 41 · Mobile workflow controls

Built for YARD-8. The desktop owns the workflow engine. A signed-in phone of the same account
can see a run and decide it, through the relay, while that computer is online.

## Who may decide

A decision is refused before any row is written unless every one of these is true:

- The channel was opened by a phone. A host token, a computer, and any other kind are refused.
  A relay that does not say the kind is treated as other, so it cannot decide.
- The grant includes `workflow:decide`. `workflow:read` is enough to list and to open a run, and
  watching includes it. Terminal control does not include either. `workflow:decide` is granted
  only when the person at the computer checks **and approve or answer workflows** (or answers
  yes to that question on `yardsort-host pair`). A phone that does not ask does not receive it.
- The run is inside the grant. A run with no project is visible only to a grant of every
  project. Anything outside the grant is `not_found`, and nothing is written.

The desktop process's own environment is not the caller. A session Yardsort started is still
refused by `ys` on that session. It is not inferred from the process that is answering the phone.

### What the host trusts about the kind

The kind on the relay ticket is what the server says. The host records the kind the server
reported when the person approved the pairing. A later channel whose kind differs from that
record cannot decide. A pairing from before this column has no recorded kind, and the host
trusts the relay's kind. The host's own device id is never treated as a phone. A ticket that
omits the kind is other, and cannot decide. A server that lies both at approval and on the
ticket is not distinguished from an honest one.

A host with no screen does not drive workflows. It does not advertise these operations, and it
answers them as unsupported.

An existing pairing is not given the new scopes. Unpair the phone and pair it again.

## The one-time code

The phone does not use the desktop window path. It uses the same code-checked update as a
terminal: the row changes only while it is still undecided, the attempt count and the code hash
are still the ones just checked, the run is running, and the step is waiting. The desktop window
and the phone cannot both apply.

The desktop driver keeps the plaintext code in memory and shows it on the computer. The phone
receives it only as the reply to `workflow.challenge`, on the authenticated relay, and only when
the checks above pass. The code is not in a run, an artifact, a notification, or a push payload.
The phone holds it until it sends the decision. It does not draw it.

If the code in memory still matches the stored hash, that is the one returned. The check does
not add an attempt. If it does not match, the memory is dropped and a new code is issued without
resetting the attempt count. After five wrong codes the challenge has no code. Show a new code
on the computer.

## What crosses the relay

Protocol minor 7 adds four operations.

- `workflow.run.list` and `workflow.run.get` need `workflow:read`. A run lists its steps, status,
  what it is waiting on, the blocking sentence, the wrong-code count, whether that step is
  locked, the waiting step's attempt, the open clarification round (0 when it is not asking),
  and whether resume is eligible. A list's waiting line uses the same "still needs a person"
  check as the detail, so a recorded decision is not listed as waiting. It does not list a
  code, a hash, or a salt. A step note that contains a path or a URL is replaced with
  "This note was left out."
- `workflow.challenge` and `workflow.decide` need `workflow:decide`. Both carry the attempt and
  the round the phone was showing. Decide is approve, reject, request changes (with a note),
  answer a question, or resume. Resume sends no code. A challenge or a decision whose attempt
  or round is no longer the open one is refused with `stale` ("This changed. Refresh and read
  it again.") and does not count as a wrong code. The challenge does not return the new code.
- An artifact body is included only for a document the waiting step named, or one stored as the
  decision or the verdict. It is cut at 2,000 characters. A line that looks like a secret is
  replaced. A file whose name is a secret is left out. A run error that contains a path or a URL
  is replaced with "The run ended with an error."

## One decision, once

Every decision carries a `decisionId`. The outcome is stored in `workflow_remote_actions`, keyed
by the phone and that id, in the same transaction as the guarded update. A retry of that pair,
with the same run, device, action, step, and note, returns the stored outcome and does not apply
again. The fingerprint is a hash of those five. It does not include the code, so a retry can send
the code again without changing the record. The same id with a different body is `conflict`.
A different id after a decision is `already_decided`. Rows older than seven days are removed
when a new one is stored. The phone starts a new id when the action or the note changes. On
`conflict` it reads the run again and shows what it is doing now.

A refusal is a response. The next tap uses a new id. A wrong code and the stored outcome commit
together, so a retry of that id does not burn another attempt.

A decision id from before a rewind still returns the outcome it stored. It does not decide the
new round. A cancelled or halted run, or a code that has been replaced, is refused.

Resume is eligible when the run is queued, or when it is running, the human decision or answer
is recorded, `applied_at` is still empty, and the step is still waiting. It wakes the driver.
It does not skip a gate. The same id returns the stored outcome after the run has moved on.

The live approval or answer row records `device_id`. The actor is `phone`. A rewind copies
`device_id` into the history row.

## Offline

A decision is not queued. When the computer is not connected, the relay answers `host_offline`
and the phone shows that the action was not sent and will not be sent later. The buttons stay
off while there is no channel. A lost reply, while the computer was online, is covered by the
decision id.

## The notice

The deep link stays `yardsort://notice/{category}/{id}`. `GET` of that notice may include
`runId`, `stepId`, and `hostId`. `hostId` is the server's host row for the computer that
submitted it. The client cannot set it. A subject that is not an id, or that looks like an
eight-character code, is dropped and the notice is still accepted. The push payload is unchanged.
A computer posting to an older server, which refuses the new fields, sends the notice once more
without them.

The phone opens a workflow notice on the decision screen. When the notice belongs to another
account this phone is signed in to, that account is made the one on screen first.

A phone asks for `workflow:read` and `workflow:decide` only when the server it is pairing with
advertises protocol minor 7 or newer. An older server never sees those names. A scope name this
build does not know is accepted and then dropped, on a pairing request and on a grant, so a
newer peer does not make pairing fail. The unknown name is not stored and not granted.
