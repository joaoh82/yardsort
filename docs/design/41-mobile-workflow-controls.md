# 41 · Mobile workflow controls

Built for YARD-8. The desktop owns the workflow engine. A signed-in phone of the same account
can see a run and decide it, through the relay, while that computer is online.

## Who may decide

A decision is refused before any row is written unless every one of these is true:

- The channel was opened by a phone. A host token, a computer, and any other kind are refused.
  A relay that does not say the kind is treated as other, so it cannot decide.
- The process answering the channel was not started by Yardsort. The markers are
  `YARDSORT_COORDINATOR`, `YARDSORT_RUN_ID`, `YARDSORT_WORKSPACE_ID`, and
  `YARDSORT_SESSION_RECORD_ID`.
- The grant includes `workflow:decide`. `workflow:read` is enough to list and to open a run.
  Terminal control does not include either. A view-only pairing still offers `workflow:decide`
  when the phone asked for it. A phone that does not ask does not receive it.
- The run is inside the grant. A run with no project is visible only to a grant of every
  project. Anything outside the grant is `not_found`, and nothing is written.

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
  locked, and whether resume is eligible. It does not list a code, a hash, or a salt.
- `workflow.challenge` and `workflow.decide` need `workflow:decide`. Decide is approve, reject,
  request changes (with a note), answer a question, or resume.
- An artifact body is included only for a document the waiting step named, or one stored as the
  decision or the verdict. It is cut at 2,000 characters. A line that looks like a secret is
  replaced. A file whose name is a secret is left out. A run error that contains a path or a URL
  is replaced with "The run ended with an error."

## One decision, once

Every decision carries a `decisionId`. The outcome is stored in `workflow_remote_actions` in the
same transaction as the guarded update. A retry of that id, with the same action, step, and note,
returns the stored outcome and does not apply again. The fingerprint is a hash of those three.
It does not include the code, so a retry can send the code again without changing the record.
The same id with a different body is `conflict`. A different id after a decision is
`already_decided`.

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

The phone opens a workflow notice on the decision screen.
