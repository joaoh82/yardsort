// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One workflow run: its steps, why it is waiting, and the documents a phone may see.
 * A secret file is left out by the computer. A one-time code is not in this view.
 */

import type { Channel, HostId, RunId, WorkflowRunDetail } from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useState } from "react";

import { OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, Button, Group, Problem, Row, Screen, Section, Waiting } from "../ui/parts";

export default function WorkflowScreen() {
  const params = useLocalSearchParams<{ host?: string; run?: string }>();
  if (!params.host || !params.run) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Run host={params.host as HostId} run={params.run as RunId} />
    </OfProfile>
  );
}

function Run({ host, run }: { host: HostId; run: RunId }) {
  const router = useRouter();
  const { reach, channel } = useChannelTo(host);
  const [asked, setAsked] = useState(0);
  const read = useCallback(async (through: Channel) => through.workflowRun(run), [run]);
  const first = useRead(channel, read, asked);
  const detail = first.answer;
  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      <Stack.Screen options={{ title: detail?.name ?? "Workflow" }} />
      <ReachLine reach={reach} lastSeenAt={null} />
      {first.problem ? <Problem>{first.problem}</Problem> : null}
      {!detail && !first.problem ? (
        <Waiting label={channel ? "Asking the computer" : "Waiting for the computer"} />
      ) : null}
      {detail ? (
        <Detail
          detail={detail}
          onDecide={(step) => router.push({ pathname: "/decision", params: { host, run, step } })}
        />
      ) : null}
    </Screen>
  );
}

function Detail({
  detail,
  onDecide,
}: {
  detail: WorkflowRunDetail;
  onDecide: (step: string) => void;
}) {
  return (
    <>
      <Body muted>{[detail.workspace, detail.status].filter(Boolean).join(" · ")}</Body>
      {detail.error ? <Problem>{detail.error}</Problem> : null}
      {detail.blocking ? <Body>{detail.blocking}</Body> : null}
      {detail.locked ? (
        <Body muted>Too many wrong codes. Show a new code on the computer.</Body>
      ) : null}
      {(detail.waitingOn || detail.resumeEligible) && (
        <Button
          title={
            detail.waitingKind === "question"
              ? "Answer"
              : detail.resumeEligible && !detail.waitingOn
                ? "Resume"
                : "Decide"
          }
          kind="primary"
          onPress={() => onDecide(detail.waitingOn ?? detail.steps[0]?.stepId ?? detail.runId)}
        />
      )}
      <Section title="Steps" note={detail.steps.length}>
        <Group>
          {detail.steps.map((step) => (
            <Row
              key={step.stepId}
              title={step.stepId}
              detail={[step.action, step.status, step.note].filter(Boolean).join(" · ")}
              label={[step.stepId, step.status].join(", ")}
            />
          ))}
        </Group>
      </Section>
      {detail.artifacts.length > 0 ? (
        <Section title="Documents" note={detail.artifacts.length}>
          {detail.artifacts.map((artifact) => (
            <Body key={artifact.name}>
              {artifact.name}
              {artifact.body ? `\n${artifact.body}` : ""}
              {artifact.truncated ? "\nCut short." : ""}
            </Body>
          ))}
        </Section>
      ) : null}
    </>
  );
}
