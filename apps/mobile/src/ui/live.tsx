// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What screens share about being live: whether anyone can see this screen, a channel to a computer
 * held only while they can (31 §5), how far it can be reached, and what was last seen meanwhile.
 */

import { ChannelClosed, type Channel, type HostId } from "@yardsort/remote-client";
import { useFocusEffect, useRouter } from "expo-router";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";
import { addNetworkStateListener } from "expo-network";
import { AppState, StyleSheet, View } from "react-native";

import { messageOf, type Parted } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { moment } from "../lib/format";
import type { ObservedComputer } from "../lib/observed";
import { NetworkChanges } from "../lib/network";
import { Presence, type Reach } from "../lib/presence";
import { Body, Button, Dot, Group, Mono, Problem, Row, Screen } from "./parts";

/**
 * What one profile reaches, shown only while that profile is the one in use and its address still
 * answers as its server. Otherwise nothing of it stays on screen, or waits behind another, and the
 * account says what happened.
 */
export function OfProfile({ children }: { children: ReactNode }) {
  const snapshot = useSnapshot();
  const router = useRouter();
  const active = snapshot.active?.id ?? null;
  const [opened] = useState(active);
  const stays = active === opened && snapshot.standing?.kind !== "otherServer";
  useFocusEffect(
    useCallback(() => {
      if (!stays) router.dismissTo("/account");
    }, [stays, router]),
  );
  return stays ? children : <Screen>{null}</Screen>;
}

/** How many things the app opened itself are in front of it now, such as the system's photo picker. */
let opened = 0;

/**
 * Do `work`, which shows something of the system's own in front of the app. On Android that is
 * another activity, and the app is told it went to the background; it has not been left, so
 * screens stay live, and what the person held, such as control of a terminal, is still theirs.
 */
export async function inFront<T>(work: () => Promise<T>): Promise<T> {
  opened += 1;
  try {
    return await work();
  } finally {
    opened -= 1;
  }
}

/** Whether this screen is the one shown, with the app in the foreground. */
export function useLive(): boolean {
  const [focused, setFocused] = useState(false);
  const [foreground, setForeground] = useState(AppState.currentState !== "background");

  useFocusEffect(
    useCallback(() => {
      setFocused(true);
      return () => setFocused(false);
    }, []),
  );

  useEffect(() => {
    const watcher = AppState.addEventListener("change", (state) => {
      // The app comes back to the foreground after what it opened, so that ends it.
      if (state === "background" && opened > 0) return;
      setForeground(state !== "background");
    });
    return () => watcher.remove();
  }, []);

  return focused && foreground;
}

/** The phone's network, for every screen holding channels. */
const network = new NetworkChanges((heard) => {
  const subscription = addNetworkStateListener(heard);
  return () => subscription.remove();
});

/** Tell `presence` each time the phone's network changes, while this screen is there. */
export function useNetworkChanges(presence: Presence): void {
  useEffect(() => network.listen(() => presence.networkChanged()), [presence]);
}

export interface Link {
  reach: Reach | undefined;
  /** Open, or `null`: a new one each time it is reached again, to read everything through anew. */
  channel: Channel | null;
}

/** A channel to `host`, held while this screen is live. */
export function useChannelTo(host: HostId): Link {
  const { accounts } = useApp();
  const presence = useMemo(
    () =>
      new Presence({
        open: (to) => accounts.openChannel(to),
        explain: (to) => accounts.explain(to),
      }),
    [accounts],
  );
  const reaches = useSyncExternalStore(presence.subscribe, presence.getSnapshot);
  const live = useLive();
  useNetworkChanges(presence);

  useEffect(() => {
    if (live) presence.watch([host]);
    else presence.stop();
  }, [live, host, presence]);
  useEffect(() => () => presence.stop(), [presence]);

  const reach = reaches.get(host);
  return { reach, channel: reach?.kind === "online" ? presence.channelOf(host) : null };
}

/** An answer read through a channel: asked again on a new channel, and whenever `asked` changes. */
export function useRead<T>(
  channel: Channel | null,
  read: (channel: Channel) => Promise<T>,
  asked: number,
): { answer: T | null; at: number | null; problem: string | null } {
  const [state, setState] = useState<{
    answer: T | null;
    at: number | null;
    problem: string | null;
  }>({ answer: null, at: null, problem: null });
  useEffect(() => {
    if (!channel) return;
    let stale = false;
    read(channel).then(
      (answer) => {
        if (!stale) setState({ answer, at: Date.now(), problem: null });
      },
      (error: unknown) => {
        // The reach says what became of a channel.
        if (stale || error instanceof ChannelClosed) return;
        setState((held) => ({ ...held, problem: messageOf(error) }));
      },
    );
    return () => {
      stale = true;
    };
  }, [channel, read, asked]);
  return state;
}

export function describeReach(reach: Reach | undefined, lastSeenAt: number | null): string {
  switch (reach?.kind) {
    case "online":
      return "Online";
    case "offline":
      return lastSeenAt ? `Offline, last seen ${moment(lastSeenAt)}` : "Offline";
    case "reconnecting":
      return "Reconnecting";
    case "unreachable":
      return "Cannot be reached";
    case "gone":
      return reach.why === "removed" ? "No longer on this account" : "No longer lets this phone in";
    default:
      return "Connecting";
  }
}

export function reachDot(reach: Reach | undefined): "on" | "off" | "trying" | "bad" {
  switch (reach?.kind) {
    case "online":
      return "on";
    case "reconnecting":
      return "trying";
    case "unreachable":
    case "gone":
      return "bad";
    default:
      return "off";
  }
}

/** Where a computer stands, at the top of what is read from it. */
export function ReachLine({
  reach,
  lastSeenAt,
  also,
}: {
  reach: Reach | undefined;
  lastSeenAt: number | null;
  /** Where in the computer this is: a branch. */
  also?: string | undefined;
}) {
  return (
    <>
      <View style={styles.reach}>
        <Dot kind={reachDot(reach)} />
        <Mono>{describeReach(reach, lastSeenAt)}</Mono>
        {also ? <Mono>· {also}</Mono> : null}
      </View>
      {reach?.kind === "offline" && <Body muted>Offline does not mean its agents stopped.</Body>}
      {reach?.kind === "unreachable" && <Body muted>{UNREACHABLE}</Body>}
    </>
  );
}

/** Why nothing was deleted, while a computer refuses this phone and the server cannot say why. */
export const UNREACHABLE =
  "It turned this phone away, and the server could not say why. Nothing was changed on this phone, and it keeps trying.";

/** What a person can do about a computer no longer reached. */
export const PARTED: Record<Parted, { what: string; next: string }> = {
  tookBack: {
    what: "no longer lets this phone in.",
    next: "To reach it again, pair again and say yes at that computer.",
  },
  removed: {
    what: "is no longer on this account.",
    next: "If it is set up again, pair with it again.",
  },
};

/** A computer this phone can no longer reach: nothing of it is shown any more. */
export function NoLongerPaired({ why }: { why: Parted }) {
  const router = useRouter();
  return (
    <>
      <Problem>This computer {PARTED[why].what}</Problem>
      <Body muted>{PARTED[why].next}</Body>
      <Button title="Back to the account" onPress={() => router.dismissTo("/account")} />
    </>
  );
}

/** The computers as last seen, for before the server answers, or when it cannot. */
export function SeenComputers() {
  const { accounts } = useApp();
  const router = useRouter();
  const [seen, setSeen] = useState<ObservedComputer[]>([]);

  useEffect(() => {
    let stale = false;
    void accounts
      .observed()
      ?.computers()
      .then((computers) => {
        if (!stale) setSeen(computers);
      });
    return () => {
      stale = true;
    };
  }, [accounts]);

  return (
    <Group>
      {seen.map((computer) => (
        <Row
          key={computer.host}
          title={computer.name}
          detail={`As of ${moment(computer.observedAt)}`}
          monoDetail
          leading={<Dot kind="off" />}
          onPress={() => router.push({ pathname: "/computer", params: { host: computer.host } })}
        />
      ))}
    </Group>
  );
}

const styles = StyleSheet.create({
  reach: { flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: 8 },
});
