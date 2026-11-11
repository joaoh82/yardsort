// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Approve, reject, request changes, answer, or resume. The one-time code is not shown.
 * While the computer is offline the buttons do nothing, and nothing is queued.
 */

import type { WorkflowAction } from "@yardsort/remote-client";

import { decisionAvailability } from "../lib/workflows";
import { Body, Button, Field } from "./parts";

export function DecisionForm({
  online,
  kind,
  locked,
  busy,
  note,
  onNote,
  onDecide,
}: {
  online: boolean;
  kind: "approval" | "question" | null;
  locked: boolean;
  busy: boolean;
  note: string;
  onNote: (note: string) => void;
  onDecide: (action: WorkflowAction) => void;
}) {
  const availability = decisionAvailability(online);
  const off = !availability.enabled || locked || busy;
  const send = (action: WorkflowAction, blocked: boolean) => {
    if (blocked) return;
    onDecide(action);
  };
  return (
    <>
      {availability.reason ? <Body>{availability.reason}</Body> : null}
      {locked ? <Body muted>Too many wrong codes. Show a new code on the computer.</Body> : null}
      {kind === "question" ? (
        <>
          <Field label="Answer" value={note} onChangeText={onNote} />
          <Button
            title="Send answer"
            kind="primary"
            disabled={off || note.trim() === ""}
            onPress={() => send("answer", off || note.trim() === "")}
          />
        </>
      ) : (
        <>
          <Button
            title="Approve"
            kind="primary"
            disabled={off}
            onPress={() => send("approve", off)}
          />
          <Button title="Reject" kind="danger" disabled={off} onPress={() => send("reject", off)} />
          <Field label="What should change" value={note} onChangeText={onNote} />
          <Button
            title="Request changes"
            disabled={off || note.trim() === ""}
            onPress={() => send("changes", off || note.trim() === "")}
          />
        </>
      )}
      <Button
        title="Resume"
        disabled={!availability.enabled || busy}
        onPress={() => send("resume", !availability.enabled || busy)}
      />
    </>
  );
}
