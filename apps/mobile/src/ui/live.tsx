// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What screens share about being live: whether anyone can see this screen, a channel to a computer
 * held only while they can (31 §5), how far it can be reached, and what was last seen meanwhile.
 */

import { ChannelClosed, type Channel, type HostId } from "@yardsort/remote-client";
import { useFocusEffect, useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { AppState } from "react-native";

import { messageOf } from "../lib/accounts";
import { useApp } from "../lib/context";
import { moment } from "../lib/format";
import type { ObservedComputer } from "../lib/observed";
import { Presence, type Reach } from "../lib/presence";
import { Body, Button, Problem, Row } from "./parts";

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
    const watcher = AppState.addEventListener("change", (state) =>
      setForeground(state !== "background"),
    );
    return () => watcher.remove();
  }, []);

  return focused && foreground;
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
    () => new Presence({ open: (to) => accounts.openChannel(to) }),
    [accounts],
  );
  const reaches = useSyncExternalStore(presence.subscribe, presence.getSnapshot);
  const live = useLive();

  useEffect(() => {
    if (live) presence.watch([host]);
    else presence.stop();
  }, [live, host, presence]);
  useEffect(() => () => presence.stop(), [presence]);

  // A computer that stopped letting this phone in: asking the server settles what became of it,
  // and takes what was kept of it with it.
  const reach = reaches.get(host);
  const gone = reach?.kind === "gone";
  useEffect(() => {
    if (gone) void accounts.computers().catch(() => {});
  }, [gone, accounts]);

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
    case "gone":
      return "No longer lets this phone in";
    default:
      return "Connecting";
  }
}

/** Where a computer stands, at the top of what is read from it. */
export function ReachLine({
  reach,
  lastSeenAt,
}: {
  reach: Reach | undefined;
  lastSeenAt: number | null;
}) {
  return (
    <>
      <Body muted>{describeReach(reach, lastSeenAt)}</Body>
      {reach?.kind === "offline" && <Body muted>Offline does not mean its agents stopped.</Body>}
    </>
  );
}

/** A computer that took back this phone's way in: nothing of it is shown any more. */
export function NoLongerPaired() {
  const router = useRouter();
  return (
    <>
      <Problem>This computer no longer lets this phone in.</Problem>
      <Body muted>To reach it again, pair again and say yes at that computer.</Body>
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
    <>
      {seen.map((computer) => (
        <Row
          key={computer.host}
          title={computer.name}
          detail={`As of ${moment(computer.observedAt)}`}
          onPress={() => router.push({ pathname: "/computer", params: { host: computer.host } })}
        />
      ))}
    </>
  );
}
