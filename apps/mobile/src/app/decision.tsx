// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One decision. The one-time code is fetched when a button is pressed and held only
 * long enough to send it. It is not drawn. While the computer is offline, the action
 * is not sent and will not be sent later.
 */

import type { Channel, HostId, RunId, WorkflowAction } from "@yardsort/remote-client";
import * as Crypto from "expo-crypto";
import { Redirect, Stack, useLocalSearchParams } from "expo-router";
import { useCallback, useRef, useState } from "react";

import { messageOf } from "../lib/accounts";
import { DecisionIds, conflictSummary, staleChallenge } from "../lib/workflows";
import { DecisionForm } from "../ui/DecisionForm";
import { OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, Problem, Screen, Waiting } from "../ui/parts";

export default function DecisionScreen() {
  const params = useLocalSearchParams<{ host?: string; run?: string; step?: string }>();
  if (!params.host || !params.run || !params.step) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Decision
        host={params.host as HostId}
        run={params.run as RunId}
        step={params.step as RunId}
      />
    </OfProfile>
  );
}

function Decision({ host, run, step }: { host: HostId; run: RunId; step: RunId }) {
  const { reach, channel, lastSeenAt } = useChannelTo(host);
  const [asked, setAsked] = useState(0);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const ids = useRef(new DecisionIds());
  const read = useCallback(async (through: Channel) => through.workflowRun(run), [run]);
  const first = useRead(channel, read, asked);
  const detail = first.answer;
  const kind =
    detail?.waitingKind === "question" || detail?.waitingKind === "approval"
      ? detail.waitingKind
      : null;

  const onDecide = async (action: WorkflowAction) => {
    if (!channel || !detail) return;
    const attempt = detail.attempt;
    const round = detail.round;
    const sentNote = note.trim() === "" ? "" : note;
    const id = ids.current.next(action, sentNote, () => Crypto.randomUUID()) as RunId;
    setBusy(true);
    setProblem(null);
    try {
      let presented: string | null = null;
      if (action !== "resume") {
        const challenged = await channel.workflowChallenge(run, step, attempt, round);
        if (challenged.attempt !== attempt || challenged.round !== round) {
          setProblem("This changed. Refresh and read it again.");
          ids.current.settled();
          setAsked((n) => n + 1);
          return;
        }
        presented = challenged.code;
      }
      const decision = await channel.workflowDecide({
        runId: run,
        stepId: action === "resume" ? null : step,
        action,
        note: sentNote === "" ? null : sentNote,
        code: action === "resume" ? null : presented,
        decisionId: id,
        attempt,
        round,
      });
      ids.current.settled();
      if (decision.outcome === "stale") {
        setProblem("This changed. Refresh and read it again.");
        setAsked((n) => n + 1);
        return;
      }
      if (decision.outcome === "conflict") {
        const again = await channel.workflowRun(run);
        setProblem(conflictSummary(again));
        setAsked((n) => n + 1);
        return;
      }
      setMessage(decision.message);
      setAsked((n) => n + 1);
    } catch (error) {
      if (staleChallenge(error)) {
        ids.current.settled();
        setProblem("This changed. Refresh and read it again.");
        setAsked((n) => n + 1);
        return;
      }
      setProblem(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      <Stack.Screen options={{ title: "Decision" }} />
      <ReachLine reach={reach} lastSeenAt={lastSeenAt} />
      {first.problem ? <Problem>{first.problem}</Problem> : null}
      {problem ? <Problem>{problem}</Problem> : null}
      {message ? <Body>{message}</Body> : null}
      {!detail && !first.problem ? (
        <Waiting label={channel ? "Asking the computer" : "Waiting for the computer"} />
      ) : null}
      {detail?.blocking ? <Body>{detail.blocking}</Body> : null}
      {detail ? (
        <DecisionForm
          online={channel !== null}
          kind={kind}
          locked={detail.locked}
          resumeEligible={detail.resumeEligible}
          busy={busy}
          note={note}
          onNote={setNote}
          onDecide={(action) => void onDecide(action)}
        />
      ) : null}
    </Screen>
  );
}
