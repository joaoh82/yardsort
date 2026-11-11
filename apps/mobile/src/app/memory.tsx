// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A project's approved memory, to read. Reviewing and writing it happen at the computer. Nothing
 * here is kept: it shows only when the computer can be reached, and goes with the screen.
 */

import type { Channel, HostId, MemoryPage, ProjectId } from "@yardsort/remote-client";
import { Redirect, useLocalSearchParams } from "expo-router";
import { useCallback, useState } from "react";

import { messageOf } from "../lib/accounts";
import { moment } from "../lib/format";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, Button, Card, Mono, Problem, Screen, Waiting } from "../ui/parts";

export default function MemoryScreen() {
  const params = useLocalSearchParams<{ host?: string; project?: string }>();
  if (!params.host || !params.project) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Memory host={params.host as HostId} project={params.project as ProjectId} />
    </OfProfile>
  );
}

function Memory({ host, project }: { host: HostId; project: ProjectId }) {
  const { reach, channel, lastSeenAt } = useChannelTo(host);
  const [asked, setAsked] = useState(0);
  const [more, setMore] = useState<MemoryPage | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const read = useCallback(
    async (through: Channel) => {
      const page = await through.memory({ project });
      setMore(null);
      return page;
    },
    [project],
  );
  const first = useRead(channel, read, asked);

  const showMore = async (cursor: NonNullable<MemoryPage["next"]>) => {
    if (!channel) return;
    try {
      const page = await channel.memory({ project, cursor });
      const held = more ?? first.answer;
      if (held) setMore({ notes: [...held.notes, ...page.notes], next: page.next });
    } catch (error) {
      setProblem(messageOf(error));
    }
  };

  if (reach?.kind === "gone") {
    return (
      <Screen>
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  const page = more ?? first.answer;

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      <ReachLine reach={reach} lastSeenAt={lastSeenAt} />
      <Body muted>Approved notes only. Proposals are reviewed at the computer.</Body>
      {first.problem && <Problem>{first.problem}</Problem>}
      {problem && <Problem>{problem}</Problem>}
      {channel && !page && !first.problem && <Waiting label="Asking the computer" />}
      {!channel && !page && <Body muted>Memory shows when the computer can be reached.</Body>}
      {page && !channel && first.at !== null && <Body muted>As of {moment(first.at)}.</Body>}
      {page?.notes.length === 0 && <Body muted>No approved notes in this project.</Body>}
      {page?.notes.map((note) => (
        <Card key={note.id} label="A note">
          <Body>{note.text}</Body>
          <Mono>
            {note.author}, {moment(note.updatedAt)}
          </Mono>
        </Card>
      ))}
      {page?.next && (
        <Button
          title="Show more"
          onPress={() => {
            if (page.next) void showMore(page.next);
          }}
        />
      )}
    </Screen>
  );
}
