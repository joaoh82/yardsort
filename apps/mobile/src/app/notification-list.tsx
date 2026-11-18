// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Notifications (48): one list over every computer this phone can reach. What needs you, what
 * finished and was not seen, and what left, kept a week by its computer. A computer that is
 * offline shows a line and no rows. Which notices are pushed to this phone is chosen elsewhere,
 * under Settings → Notifications, reached from the foot of this list too.
 */

import type { HostId } from "@yardsort/remote-client";
import { Redirect, Stack, useRouter } from "expo-router";
import { useMemo, useState } from "react";

import { useSnapshot } from "../lib/context";
import { cut, needing, sections, unseen } from "../lib/notifications";
import { OfProfile, UNREACHABLE } from "../ui/live";
import { NotificationRow, isAway, useEveryList, useLists } from "../ui/notifications";
import { Body, Button, Group, Problem, Screen, Section, Waiting } from "../ui/parts";
import { useNow } from "../ui/pull";

export default function NotificationListScreen() {
  const snapshot = useSnapshot();
  if (!snapshot.loaded) return <Screen>{null}</Screen>;
  if (snapshot.standing?.kind !== "signedIn") return <Redirect href="/account" />;
  return (
    <OfProfile>
      <List />
    </OfProfile>
  );
}

function List() {
  const router = useRouter();
  const [asked, setAsked] = useState(0);
  const { computers, reach, problem } = useEveryList(null, asked);
  const { lists, held } = useLists();
  const now = useNow();
  const parts = useMemo(() => sections(held), [held]);

  const refresh = () => {
    setAsked((n) => n + 1);
    for (const host of held.keys()) void lists.read(host);
  };

  const kindOf = (host: HostId) => reach.get(host)?.kind;
  const offline = computers?.filter((c) => kindOf(c.host) === "offline") ?? [];
  const refusing = computers?.filter((c) => kindOf(c.host) === "unreachable") ?? [];
  const older = [...held.values()].filter((list) => !list.keeps);
  const unread = [...held.values()].filter((list) => list.problem !== null);
  // Still to hear from: a computer neither away nor answered yet.
  const asking =
    !problem &&
    (computers === null || computers.some((c) => !isAway(reach.get(c.host)) && !held.has(c.host)));
  const answered = [...held.values()].some((list) => list.page !== null);
  const waits = parts.needs.length + parts.finished.length;

  return (
    <Screen refreshing={false} onRefresh={refresh}>
      <Stack.Screen options={{ title: "Notifications" }} />
      {problem && <Problem>{problem}</Problem>}
      {offline.map((computer) => (
        <Problem key={computer.host} attention>
          {computer.name} is offline. What waits there shows when it is back.
        </Problem>
      ))}
      {refusing.map((computer) => (
        <Body key={computer.host} muted>
          {computer.name} cannot be reached. {UNREACHABLE}
        </Body>
      ))}
      {older.map((list) => (
        <Body key={list.host} muted>
          {list.name} runs an earlier Yardsort, which keeps no list. Update it to see what waits
          there.
        </Body>
      ))}
      {unread.map((list) => (
        <Problem key={list.host}>
          {list.name}: {list.problem}
        </Problem>
      ))}
      {asking && <Waiting label="Asking your computers" />}
      {computers?.length === 0 && (
        <Body muted>No computer lets this phone in yet. Pair one from the account screen.</Body>
      )}
      {/* Said only once a computer has answered: with none reached, nothing is known. */}
      {answered && waits === 0 && (
        <>
          <Body>Nothing needs you.</Body>
          <Body muted>
            An agent that asks for a permission or finishes, a workflow that waits for an answer,
            and a test that waits for a yes show up here.
          </Body>
        </>
      )}

      {parts.needs.length > 0 && (
        <Section title="Needs you" note={needing(held)}>
          <Group>
            {parts.needs.map((row) => (
              <NotificationRow key={`${row.host}/${row.id}`} row={row} now={now} />
            ))}
          </Group>
        </Section>
      )}
      {parts.finished.length > 0 && (
        <Section title="Finished, not seen" note={unseen(held)}>
          <Group>
            {parts.finished.map((row) => (
              <NotificationRow key={`${row.host}/${row.id}`} row={row} now={now} />
            ))}
          </Group>
        </Section>
      )}
      {parts.earlier.length > 0 && (
        <Section title="Earlier">
          <Group>
            {parts.earlier.map((row) => (
              <NotificationRow key={`${row.host}/${row.id}`} row={row} now={now} />
            ))}
          </Group>
        </Section>
      )}
      {cut(held) && <Body muted>Only the newest are shown. The rest are at the computer.</Body>}
      {waits > 0 && (
        <Body muted>
          A row leaves Needs you when it is answered, here or at the computer. Opening a finished
          one marks it seen on the computer too.
        </Body>
      )}
      <Button
        title="Choose what reaches this phone"
        kind="quiet"
        onPress={() => router.push("/notifications")}
      />
    </Screen>
  );
}
