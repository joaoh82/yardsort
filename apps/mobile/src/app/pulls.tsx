// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A computer's pull requests, over every project this phone may see (45): the open ones, and
 * beside them the merged and the closed among the newest the computer read. As old as the
 * computer's last reading, which it does again while this is shown. Read-only.
 */

import type { Channel, HostId, PullsOf } from "@yardsort/remote-client";
import { Redirect, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { keyOf, readsPulls, routeOf } from "../lib/pulls";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, FONTS, Group, Problem, Screen, useColors, Waiting } from "../ui/parts";
import { PullRow, useNow, usePulls } from "../ui/pull";

const TABS: { of: PullsOf; words: string; none: string; foot: string }[] = [
  {
    of: "open",
    words: "Open",
    none: "No open pull requests.",
    foot: "Every open pull request of the projects this phone may see, as the computer last read them from gh.",
  },
  {
    of: "merged",
    words: "Merged",
    none: "No merged pull requests among the newest the computer read.",
    foot: "The merged ones among the newest fifty of each project, as the computer last read them from gh.",
  },
  {
    of: "closed",
    words: "Closed",
    none: "No closed pull requests among the newest the computer read.",
    foot: "The ones closed without merging among the newest fifty of each project, as the computer last read them from gh.",
  },
];

export default function PullsScreen() {
  const params = useLocalSearchParams<{ host?: string }>();
  if (!params.host) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Pulls host={params.host as HostId} />
    </OfProfile>
  );
}

function Pulls({ host }: { host: HostId }) {
  const colors = useColors();
  const router = useRouter();
  const { reach, channel, lastSeenAt } = useChannelTo(host);
  const [asked, setAsked] = useState(0);
  const now = useNow();
  const [of, setOf] = useState<PullsOf>("open");
  const tab = TABS.find((t) => t.of === of) ?? TABS[0];
  const { read: shown, problem } = usePulls(channel, of, asked);

  // What the projects and workspaces are called. Not a reason to hold the list back.
  const readNames = useCallback((through: Channel) => through.workspaces(), []);
  const named = useRead(channel, readNames, asked).answer;
  const names = useMemo(() => {
    const found = new Map<string, string>();
    for (const project of named?.projects ?? []) found.set(project.id, project.name);
    for (const workspace of named?.workspaces ?? []) found.set(workspace.id, workspace.name);
    return found;
  }, [named]);
  const projects = useMemo(
    () => new Set((shown?.pulls ?? []).map((pull) => pull.project)).size,
    [shown],
  );

  if (reach?.kind === "gone") {
    return (
      <Screen>
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      <ReachLine reach={reach} lastSeenAt={lastSeenAt} />
      <View style={[styles.tabs, { backgroundColor: colors.raised }]} accessibilityRole="tablist">
        {TABS.map((option) => {
          const on = option.of === of;
          const words =
            option.of === "open" && shown ? `${option.words} · ${shown.open}` : option.words;
          return (
            <Pressable
              key={option.of}
              accessibilityRole="tab"
              accessibilityLabel={words}
              accessibilityState={{ selected: on }}
              onPress={() => setOf(option.of)}
              style={[
                styles.tab,
                on && {
                  backgroundColor: colors.surface,
                  borderColor: colors.lineStrong,
                  borderWidth: 1,
                },
              ]}
            >
              <Text
                style={[
                  styles.tabText,
                  {
                    color: on ? colors.ink : colors.muted,
                    fontFamily: on ? FONTS.semibold : FONTS.medium,
                  },
                ]}
              >
                {words}
              </Text>
            </Pressable>
          );
        })}
      </View>
      {problem && <Problem>{problem}</Problem>}
      {!channel && !shown && (
        <Body muted>Pull requests show when the computer can be reached.</Body>
      )}
      {channel && !readsPulls(channel) && (
        <Body muted>This computer does not tell of pull requests.</Body>
      )}
      {readsPulls(channel) && !shown && !problem && <Waiting label="Asking the computer" />}
      {shown && shown.pulls.length === 0 && (
        <Body muted>
          {shown.reading ? "The computer is reading them from gh." : (tab?.none ?? "")}
        </Body>
      )}
      <Group>
        {shown?.pulls.map((pull) => (
          <PullRow
            key={keyOf(pull)}
            pull={pull}
            project={projects > 1 || pull.repo ? names.get(pull.project) : undefined}
            workspaces={pull.workspaces
              .map((workspace) => names.get(workspace))
              .filter((name): name is string => name !== undefined)}
            now={now}
            onPress={() => router.push({ pathname: "/pull", params: { host, ...routeOf(pull) } })}
          />
        ))}
      </Group>
      {shown && <Body muted>{tab?.foot ?? ""}</Body>}
    </Screen>
  );
}

const styles = StyleSheet.create({
  tabs: { flexDirection: "row", borderRadius: 9, padding: 3, gap: 2 },
  tab: { flex: 1, borderRadius: 7, paddingVertical: 8, alignItems: "center" },
  tabText: { fontSize: 14 },
});
