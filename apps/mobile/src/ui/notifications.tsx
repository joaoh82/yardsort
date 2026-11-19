// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The Notifications list as screens show it (49): the bell in a header's corner with how many
 * things need their person, a row of the list, and the hooks that keep every computer's list
 * read while a screen is in front.
 *
 * A colour is never the only thing that says something: a row's part has a heading, its mark
 * is filled or hollow, and its words say what happened.
 */

import type { Channel, HostId } from "@yardsort/remote-client";
import { Stack, useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { type Computer, messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import {
  type Held,
  leadName,
  lineOf,
  type Listed,
  type Lists,
  listsOf,
  type Looked,
  needing,
  opens,
  spoken,
  toneOf,
  what,
  whenWords,
  whereWords,
} from "../lib/notifications";
import { Presence, type Reach } from "../lib/presence";
import { useLive, useNetworkChanges } from "./live";
import { FONTS, Icon, useColors } from "./parts";
import { SettingsButton } from "./settings-corner";

/** The lists of the profile in use, and what is held of them now. */
export function useLists(): { lists: Lists; held: Held } {
  const profile = useSnapshot().active?.id ?? "";
  const lists = useMemo(() => listsOf(profile), [profile]);
  const held = useSyncExternalStore(lists.subscribe, lists.getSnapshot);
  return { lists, held };
}

/** Whether nothing of a computer is shown: it is offline, refuses this phone, or is gone. */
export function isAway(reach: Reach | undefined): boolean {
  return reach?.kind === "offline" || reach?.kind === "unreachable" || reach?.kind === "gone";
}

/** Keep the list of the one computer a screen holds a channel to. */
export function useListOf(
  host: HostId,
  name: string,
  channel: Channel | null,
  reach: Reach | undefined,
): void {
  const { lists } = useLists();
  const away = isAway(reach);
  useEffect(() => {
    lists.sync([{ host, name, channel, away }]);
  }, [lists, host, name, channel, away]);
}

/** Keep the lists of the computers a screen holds channels to, through a presence of its own. */
export function useListsOf(
  computers: readonly Computer[] | null,
  reach: ReadonlyMap<HostId, Reach>,
  presence: Presence,
): void {
  const { lists } = useLists();
  useEffect(() => {
    if (!computers) return;
    lists.keepOnly(computers.map((c) => c.host));
    lists.sync(
      computers.map((c): Looked => ({
        host: c.host,
        name: c.name,
        channel: reach.get(c.host)?.kind === "online" ? presence.channelOf(c.host) : null,
        away: isAway(reach.get(c.host)),
      })),
    );
  }, [lists, computers, reach, presence]);
}

/**
 * A channel to every paired computer but `except`, held while this screen is in front, with
 * each one's list kept. For the list itself, and for a screen that holds a channel to one
 * computer and shows the bell, whose count is of them all.
 */
export function useEveryList(
  except: HostId | null,
  asked: number,
): { computers: Computer[] | null; reach: ReadonlyMap<HostId, Reach>; problem: string | null } {
  const { accounts, diagnostics } = useApp();
  const presence = useMemo(
    () =>
      new Presence({
        open: (host) => accounts.openChannel(host),
        explain: (host) => accounts.explain(host),
        diagnostics,
      }),
    [accounts, diagnostics],
  );
  const reach = useSyncExternalStore(presence.subscribe, presence.getSnapshot);
  useNetworkChanges(presence);
  const live = useLive();
  const [computers, setComputers] = useState<Computer[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    if (!live) return;
    let stale = false;
    accounts.computers().then(
      (answer) => {
        if (stale) return;
        setComputers(answer.computers);
        setProblem(null);
      },
      (error: unknown) => {
        if (!stale) setProblem(messageOf(error));
      },
    );
    return () => {
      stale = true;
    };
  }, [accounts, live, asked]);

  const others = useMemo(
    () => computers?.filter((c) => c.host !== except) ?? null,
    [computers, except],
  );
  useEffect(() => {
    if (live) presence.watch(others?.map((c) => c.host) ?? []);
    else presence.stop();
  }, [live, others, presence]);
  useEffect(() => () => presence.stop(), [presence]);

  const { lists } = useLists();
  useEffect(() => {
    // The screen's own computer is kept by its own channel: only the others are synced here.
    if (!computers || !others) return;
    lists.keepOnly(computers.map((c) => c.host));
    lists.sync(
      others.map((c): Looked => ({
        host: c.host,
        name: c.name,
        channel: reach.get(c.host)?.kind === "online" ? presence.channelOf(c.host) : null,
        away: isAway(reach.get(c.host)),
      })),
    );
  }, [lists, computers, others, reach, presence]);

  return { computers, reach, problem };
}

/**
 * Mark seen what finished in what this screen shows: its conversation, its terminal or its run.
 * Looking at a finished agent here is looking at it, as at the computer. `channel` is held only
 * while the screen is in front, so a row is never marked from behind another.
 */
export function useLooking(
  host: HostId,
  channel: Channel | null,
  reach: Reach | undefined,
  shows: (row: Listed) => boolean,
): void {
  const { lists, held } = useLists();
  useListOf(host, channel?.host.name ?? held.get(host)?.name ?? "", channel, reach);
  const page = held.get(host)?.page;
  useEffect(() => {
    if (!channel || !page) return;
    const ids = page.notifications
      .filter((row) => row.part === "finished" && shows({ ...row, host, hostName: "" }))
      .map((row) => row.id);
    if (ids.length > 0) void lists.seen(host, ids);
  }, [lists, host, channel, page, shows]);
}

/**
 * The header's corner: the bell with how many things need you, over every computer read, and
 * Settings on the screens it is reached from.
 */
export function Corner({ settings = false }: { settings?: boolean }) {
  return (
    <Stack.Screen
      options={{
        headerRight: () => (
          <View style={styles.corner}>
            <Bell />
            {settings && <SettingsButton />}
          </View>
        ),
      }}
    />
  );
}

function Bell() {
  const router = useRouter();
  const colors = useColors();
  const { held } = useLists();
  const needs = needing(held);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={
        needs > 0
          ? `Notifications, ${needs} ${needs === 1 ? "needs" : "need"} you`
          : "Notifications"
      }
      hitSlop={10}
      onPress={() => router.push("/notification-list")}
      style={({ pressed }) => [styles.bell, { opacity: pressed ? 0.5 : 1 }]}
    >
      <Icon name="bell" color={colors.ink} />
      {needs > 0 && (
        <View style={[styles.count, { backgroundColor: colors.accent }]}>
          <Text style={[styles.countText, { color: colors.onAccent }]}>
            {needs > 99 ? "99+" : needs}
          </Text>
        </View>
      )}
    </Pressable>
  );
}

/** One row of the list. Pressing it opens what the phone has for it, when it has anything. */
export function NotificationRow({ row, now }: { row: Listed; now: number }) {
  const router = useRouter();
  const colors = useColors();
  const { lists } = useLists();
  const tone = toneOf(row);
  const line = lineOf(row);
  const to = opens(row);
  const left = tone === "earlier";
  const mark =
    tone === "bad" ? colors.danger : tone === "earlier" ? colors.faint : colors.accentText;
  const open = useCallback(() => {
    if (!to) return;
    // Opening a finished one is looking at it: seen on the computer too.
    if (row.part === "finished") void lists.seen(row.host, [row.id]);
    router.push({ pathname: to.pathname, params: to.params });
  }, [lists, router, row.host, row.id, row.part, to]);
  const inside = (
    <>
      <View
        style={[
          styles.mark,
          tone === "needs" || tone === "bad"
            ? { backgroundColor: mark }
            : { borderColor: mark, borderWidth: 2 },
        ]}
      />
      <View style={styles.text}>
        <Text style={[styles.lead, { color: left ? colors.muted : colors.ink }]}>
          {leadName(row)}
        </Text>
        <Text style={[styles.where, { color: colors.muted }]}>{whereWords(row)}</Text>
        <Text style={[styles.what, { color: left ? colors.muted : colors.ink }]}>{what(row)}</Text>
        {line ? (
          <Text
            numberOfLines={3}
            style={[line.as === "code" ? styles.where : styles.said, { color: colors.muted }]}
          >
            {line.text}
          </Text>
        ) : null}
        {!to ? (
          <Text style={[styles.said, { color: colors.faint }]}>Changed at the computer.</Text>
        ) : null}
      </View>
      <Text style={[styles.when, { color: colors.faint }]}>{whenWords(row, now)}</Text>
      {to ? <Icon name="chevron" color={colors.faint} /> : null}
    </>
  );
  const says = spoken(row, now);
  if (!to) {
    return (
      <View accessible accessibilityLabel={says} style={styles.row}>
        {inside}
      </View>
    );
  }
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={says}
      onPress={open}
      style={({ pressed }) => [styles.row, { opacity: pressed ? 0.7 : 1 }]}
    >
      {inside}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  corner: { flexDirection: "row", alignItems: "center", gap: 18 },
  // Room for the count inside its own bounds: Android's header clips what is drawn outside them.
  bell: { paddingTop: 6, paddingRight: 8, paddingLeft: 2, paddingBottom: 2 },
  count: {
    position: "absolute",
    top: 0,
    right: 0,
    minWidth: 17,
    height: 17,
    borderRadius: 9,
    paddingHorizontal: 4,
    alignItems: "center",
    justifyContent: "center",
  },
  countText: { fontFamily: FONTS.monoMedium, fontSize: 10, lineHeight: 13 },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    paddingVertical: 12,
    paddingLeft: 16,
    paddingRight: 12,
  },
  mark: { width: 10, height: 10, borderRadius: 5, marginTop: 6, marginHorizontal: 1 },
  text: { flex: 1, gap: 3 },
  lead: { fontFamily: FONTS.semibold, fontSize: 16, lineHeight: 21 },
  where: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 16 },
  what: { fontFamily: FONTS.regular, fontSize: 14, lineHeight: 19 },
  said: { fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  when: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 21 },
});
