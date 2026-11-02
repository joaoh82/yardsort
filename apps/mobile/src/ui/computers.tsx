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
import { Alert, StyleSheet, Text, View } from "react-native";

import { type Computer, type ComputerList, messageOf } from "../lib/accounts";
import { useApp } from "../lib/context";
import { Presence, type Reach } from "../lib/presence";
import { describeReach, SeenComputers, useLive } from "./live";
import { Body, Button, Card, Problem, Title, useColors, Waiting } from "./parts";

/** How often the list is asked again while a request waits at a computer (31 §5). */
const WAITING_POLL_MS = 2_000;

export function Computers({ asked }: { asked: number }) {
  const { accounts } = useApp();
  const router = useRouter();
  const presence = useMemo(
    () => new Presence({ open: (host) => accounts.openChannel(host) }),
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

  const anyOffline = list?.computers.some((c) => reach.get(c.host)?.kind === "offline") ?? false;

  return (
    <>
      <Title>Computers</Title>
      {problem && <Problem>{problem}</Problem>}
      {!list && !problem && <Waiting label="Asking the server" />}
      {!list && <SeenComputers />}
      {list?.computers.length === 0 && waiting === 0 && (
        <Body muted>
          No computer lets this phone in yet. Open Yardsort on one, choose Pair a device in
          Settings, then pair here.
        </Body>
      )}
      {list?.computers.map((computer) => (
        <ComputerRow
          key={computer.grant}
          computer={computer}
          reach={reach.get(computer.host)}
          onOpen={() => router.push({ pathname: "/computer", params: { host: computer.host } })}
          onUnpair={() => confirmUnpair(computer.name, () => void takeBack(computer.grant))}
        />
      ))}
      {anyOffline && <Body muted>Offline does not mean its agents stopped.</Body>}
      {list?.waiting.map((grant) => (
        <Card key={grant.id} label={`Waiting for ${grant.hostName}`}>
          <Body>Waiting for an answer at {grant.hostName}.</Body>
          <Button title="Cancel the request" onPress={() => void takeBack(grant.id)} />
        </Card>
      ))}
      <Button title="Pair a computer" kind="primary" onPress={() => router.push("/pair")} />
    </>
  );
}

function ComputerRow({
  computer,
  reach,
  onOpen,
  onUnpair,
}: {
  computer: Computer;
  reach: Reach | undefined;
  onOpen: () => void;
  onUnpair: () => void;
}) {
  const colors = useColors();
  const says = describeReach(reach, computer.lastSeenAt);
  return (
    <Card label={`${computer.name}, ${says}`}>
      <View style={styles.row}>
        <Text style={[styles.name, { color: colors.ink }]}>{computer.name}</Text>
        <Text style={{ color: reach?.kind === "online" ? colors.accent : colors.muted }}>
          {says}
        </Text>
      </View>
      <Button title={`Open ${computer.name}`} onPress={onOpen} />
      <Button title={`Unpair ${computer.name}`} kind="danger" onPress={onUnpair} />
    </Card>
  );
}

function confirmUnpair(name: string, unpair: () => void): void {
  Alert.alert(
    `Unpair ${name}?`,
    "This phone stops reaching it, and stays signed in. To reach it again, pair again and say yes at that computer.",
    [
      { text: "Cancel", style: "cancel" },
      { text: "Unpair", style: "destructive", onPress: unpair },
    ],
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", justifyContent: "space-between", gap: 8, flexWrap: "wrap" },
  name: { fontSize: 17, fontWeight: "600" },
});
