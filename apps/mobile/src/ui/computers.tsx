// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The computers this phone is paired with, each with whether it can be reached (31 §5), and what
 * is still waiting for an answer at one. Channels are held only while this is on screen and the
 * app is in the foreground.
 */

import type { GrantId } from "@yardsort/remote-client";
import { useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";

import { type Computer, type ComputerList, messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { Presence, type Reach } from "../lib/presence";
import { describeReach, PARTED, reachDot, SeenComputers, UNREACHABLE, useLive } from "./live";
import { Body, Button, Card, Dot, Group, Problem, Row, Section, Waiting } from "./parts";

/** How often the list is asked again while a request waits at a computer (31 §5). */
const WAITING_POLL_MS = 2_000;

export function Computers({ asked }: { asked: number }) {
  const { accounts } = useApp();
  const { lost } = useSnapshot();
  const router = useRouter();
  const presence = useMemo(
    () =>
      new Presence({
        open: (host) => accounts.openChannel(host),
        explain: (host) => accounts.explain(host),
      }),
    [accounts],
  );
  const reach = useSyncExternalStore(presence.subscribe, presence.getSnapshot);
  const [list, setList] = useState<ComputerList | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const live = useLive();

  const load = useCallback(
    () =>
      accounts.computers().then(
        (answer) => {
          setList(answer);
          setProblem(null);
        },
        (error: unknown) => setProblem(messageOf(error)),
      ),
    [accounts],
  );

  // Asked again on coming back, and on each pull to refresh.
  useEffect(() => {
    if (live) void load();
  }, [live, asked, load]);

  // Held only while someone can see the answer.
  const hosts = useMemo(() => list?.computers.map((c) => c.host) ?? [], [list]);
  useEffect(() => {
    if (live) presence.watch(hosts);
    else presence.stop();
  }, [live, hosts, presence]);
  useEffect(() => () => presence.stop(), [presence]);

  // A computer that stopped letting this phone in: the list knows what became of it.
  const gone = [...reach.values()].filter((r) => r.kind === "gone").length;
  useEffect(() => {
    if (gone > 0) void load();
  }, [gone, load]);

  const waiting = list?.waiting.length ?? 0;
  useEffect(() => {
    if (!live || waiting === 0) return;
    const timer = setInterval(() => void load(), WAITING_POLL_MS);
    return () => clearInterval(timer);
  }, [live, waiting, load]);

  const takeBack = async (grant: GrantId) => {
    try {
      await accounts.unpair(grant);
    } catch (error) {
      setProblem(messageOf(error));
    }
    await load();
  };

  const anyIs = (kind: Reach["kind"]) =>
    list?.computers.some((c) => reach.get(c.host)?.kind === kind) ?? false;

  return (
    <Section title="Computers" note={list?.computers.length}>
      {lost.map(({ host, name, why }) => (
        <Card key={host} label={`${name} ${PARTED[why].what}`}>
          <Body>
            {name} {PARTED[why].what} {PARTED[why].next}
          </Body>
          <Button title="OK" onPress={() => accounts.dismissLost(host)} />
        </Card>
      ))}
      {problem && <Problem>{problem}</Problem>}
      {!list && !problem && <Waiting label="Asking the server" />}
      {!list && <SeenComputers />}
      {list?.computers.length === 0 && waiting === 0 && (
        <Body muted>
          No computer lets this phone in yet. Open Yardsort on one, choose Pair a device in
          Settings, then pair here.
        </Body>
      )}
      <Group>
        {list?.computers.map((computer) => (
          <ComputerRow
            key={computer.grant}
            computer={computer}
            reach={reach.get(computer.host)}
            onOpen={() => router.push({ pathname: "/computer", params: { host: computer.host } })}
          />
        ))}
      </Group>
      {anyIs("offline") && <Body muted>Offline does not mean its agents stopped.</Body>}
      {anyIs("unreachable") && <Body muted>{UNREACHABLE}</Body>}
      {list?.waiting.map((grant) => (
        <Card key={grant.id} label={`Waiting for ${grant.hostName}`}>
          <Body>Waiting for an answer at {grant.hostName}.</Body>
          <Button title="Cancel the request" onPress={() => void takeBack(grant.id)} />
        </Card>
      ))}
      <Button title="Pair a computer" kind="primary" onPress={() => router.push("/pair")} />
    </Section>
  );
}

function ComputerRow({
  computer,
  reach,
  onOpen,
}: {
  computer: Computer;
  reach: Reach | undefined;
  onOpen: () => void;
}) {
  return (
    <Row
      title={computer.name}
      detail={describeReach(reach, computer.lastSeenAt)}
      monoDetail
      leading={<Dot kind={reachDot(reach)} />}
      onPress={onOpen}
    />
  );
}
