// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Workflow runs on one computer. The list is what that computer says now.
 * While it is offline, nothing new is asked and a decision cannot be sent.
 */

import type { Channel, HostId, WorkflowRunPage } from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useState } from "react";

import { messageOf } from "../lib/accounts";
import { OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, Button, Group, Problem, Row, Screen, Section, Waiting } from "../ui/parts";

export default function WorkflowsScreen() {
  const params = useLocalSearchParams<{ host?: string }>();
  if (!params.host) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Runs host={params.host as HostId} />
    </OfProfile>
  );
}

function Runs({ host }: { host: HostId }) {
  const router = useRouter();
  const { reach, channel } = useChannelTo(host);
  const [asked, setAsked] = useState(0);
  const [more, setMore] = useState<WorkflowRunPage | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const read = useCallback(async (through: Channel) => {
    setMore(null);
    return through.workflowRuns();
  }, []);
  const first = useRead(channel, read, asked);
  const page = more ?? first.answer;

  const showMore = async () => {
    if (!channel || !page?.next) return;
    try {
      const next = await channel.workflowRuns({ cursor: page.next, limit: null });
      setMore({
        runs: [
          ...page.runs,
          ...next.runs.filter((run) => !page.runs.some((held) => held.runId === run.runId)),
        ],
        next: next.next,
      });
    } catch (error) {
      setProblem(messageOf(error));
    }
  };

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      <Stack.Screen options={{ title: "Workflows" }} />
      <ReachLine reach={reach} lastSeenAt={null} />
      {first.problem ? <Problem>{first.problem}</Problem> : null}
      {problem ? <Problem>{problem}</Problem> : null}
      {!page && !first.problem ? (
        <Waiting label={channel ? "Asking the computer" : "Waiting for the computer"} />
      ) : null}
      {page ? (
        <Section title="Runs" note={page.runs.length}>
          {page.runs.length === 0 ? <Body muted>No workflow runs this phone may see.</Body> : null}
          <Group>
            {page.runs.map((run) => (
              <Row
                key={run.runId}
                title={run.name}
                detail={[run.status, run.blocking].filter(Boolean).join(" · ")}
                label={[run.name, run.status, run.blocking].filter(Boolean).join(", ")}
                onPress={() =>
                  router.push({ pathname: "/workflow", params: { host, run: run.runId } })
                }
              />
            ))}
          </Group>
          {page.next ? <Button title="Show more" onPress={() => void showMore()} /> : null}
        </Section>
      ) : null}
    </Screen>
  );
}
