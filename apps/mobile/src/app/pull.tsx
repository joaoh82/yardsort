// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One pull request, as the computer last read it (45): its card, and the workspaces it came out
 * of. Reached from a workspace row's badge, and from the list of them all. A workspace's own can
 * be merged from here (48): the computer merges it, at the commit the sheet showed.
 */

import type { Channel, HostId, ProjectId, PullDetail, WorkspaceId } from "@yardsort/remote-client";
import { ClientError } from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useEffect, useState } from "react";

import { readsPulls, refOf } from "../lib/pulls";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useLive, useRead } from "../ui/live";
import { Body, Group, Problem, Row, Screen, Section, Waiting } from "../ui/parts";
import { PullMerge } from "../ui/merge";
import { PullCard, PULLS_EVERY_MS, useNow } from "../ui/pull";

export default function PullScreen() {
  const params = useLocalSearchParams<{
    host?: string;
    project?: string;
    number?: string;
    followed?: string;
  }>();
  const number = Number(params.number);
  if (!params.host || !params.project || !Number.isInteger(number) || number < 1) {
    return <Redirect href="/account" />;
  }
  return (
    <OfProfile>
      <Pull
        host={params.host as HostId}
        project={params.project as ProjectId}
        number={number}
        followed={params.followed}
      />
    </OfProfile>
  );
}

function Pull({
  host,
  project,
  number,
  followed,
}: {
  host: HostId;
  project: ProjectId;
  number: number;
  followed: string | undefined;
}) {
  const router = useRouter();
  const { reach, channel, lastSeenAt } = useChannelTo(host);
  const live = useLive();
  const [asked, setAsked] = useState(0);
  const [gone, setGone] = useState(false);
  const now = useNow();

  // Again each minute while this is in front: a check finishes without anyone pressing anything.
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(() => setAsked((n) => n + 1), PULLS_EVERY_MS);
    return () => clearInterval(timer);
  }, [live]);

  const read = useCallback(
    async (through: Channel): Promise<PullDetail | null> => {
      try {
        const detail = await through.pull({ project, number, ...(followed ? { followed } : {}) });
        setGone(false);
        return detail;
      } catch (error) {
        if (error instanceof ClientError && error.code === "not_found") {
          setGone(true);
          return null;
        }
        throw error;
      }
    },
    [project, number, followed],
  );
  const detail = useRead(readsPulls(channel) ? channel : null, read, asked);

  // The names of the workspaces it came out of. Not a reason to hold the card back.
  const from = detail.answer?.pull.workspaces;
  const [names, setNames] = useState<Record<string, string>>({});
  useEffect(() => {
    if (!channel || !from) return;
    let stale = false;
    for (const workspace of from) {
      void channel.workspace(workspace).then(
        (found) => {
          if (!stale) setNames((held) => ({ ...held, [workspace]: found.workspace.name }));
        },
        () => {},
      );
    }
    return () => {
      stale = true;
    };
  }, [channel, from]);

  const pull = detail.answer?.pull;

  const title = <Stack.Screen options={{ title: pull ? refOf(pull) : "Pull request" }} />;

  if (reach?.kind === "gone") {
    return (
      <Screen>
        {title}
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      {title}
      <ReachLine reach={reach} lastSeenAt={lastSeenAt} />
      {detail.problem && <Problem>{detail.problem}</Problem>}
      {!channel && !detail.answer && (
        <Body muted>A pull request shows when the computer can be reached.</Body>
      )}
      {channel && !readsPulls(channel) && (
        <Body muted>This computer does not tell of pull requests.</Body>
      )}
      {readsPulls(channel) && !detail.answer && !detail.problem && !gone && (
        <Waiting label="Asking the computer" />
      )}
      {gone && !detail.answer && (
        <Body muted>
          The computer no longer has this pull request among those it read. It may be an older one
          than it reads now.
        </Body>
      )}
      {detail.answer && pull && (
        <PullCard pull={pull} detail={detail.answer} now={now}>
          <PullMerge
            key={`${pull.project}#${pull.number}`}
            channel={channel}
            pull={pull}
            again={asked}
            onDone={() => setAsked((n) => n + 1)}
          />
        </PullCard>
      )}
      {pull && pull.workspaces.length > 0 && (
        <Section title={pull.followed ? "Followed from" : "Opened from"}>
          <Group>
            {pull.workspaces.map((workspace: WorkspaceId) => (
              <Row
                key={workspace}
                title={names[workspace] ?? "Workspace"}
                onPress={() => router.push({ pathname: "/workspace", params: { host, workspace } })}
              />
            ))}
          </Group>
        </Section>
      )}
    </Screen>
  );
}
