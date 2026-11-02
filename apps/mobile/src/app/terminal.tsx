// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Watching one terminal, at the computer's size, scrolling sideways when that is wider than the
 * phone. Nothing here reaches the program: taking control is another step, and another slice.
 *
 * Coming back to this screen, or the app to the foreground, resumes it. So does a dropped channel,
 * and in none of those is anything sent again.
 */

import type { Controller, HostId, TerminalId } from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams } from "expo-router";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { type Sight, PageFeed, TerminalWatch } from "../lib/watch";
import { describeReach, NoLongerPaired, useChannelTo } from "../ui/live";
import { Body, Problem, Screen, useColors } from "../ui/parts";
import { TerminalView } from "../ui/terminal-view";

export default function TerminalScreen() {
  const params = useLocalSearchParams<{ host?: string; terminal?: string; label?: string }>();
  if (!params.host || !params.terminal) return <Redirect href="/account" />;
  return (
    <Watching
      host={params.host as HostId}
      terminal={params.terminal as TerminalId}
      label={params.label || "Terminal"}
    />
  );
}

function Watching({
  host,
  terminal,
  label,
}: {
  host: HostId;
  terminal: TerminalId;
  label: string;
}) {
  const colors = useColors();
  const { reach, channel } = useChannelTo(host);
  const [pageStarts, setPageStarts] = useState(0);
  const [trouble, setTrouble] = useState(false);

  const { feed, watch } = useMemo(() => {
    const feed = new PageFeed(
      () => watch.startOver(),
      () => watch.pageCaughtUp(),
    );
    const watch = new TerminalWatch(terminal, feed);
    return { feed, watch };
  }, [terminal]);
  const sight = useSyncExternalStore(watch.subscribe, watch.getSnapshot);

  useEffect(() => {
    if (!channel || pageStarts === 0) return;
    void watch.follow(channel);
    return () => watch.stop();
  }, [channel, pageStarts, watch]);

  const title = <Stack.Screen options={{ title: label }} />;

  if (reach?.kind === "gone") {
    return (
      <Screen>
        {title}
        <NoLongerPaired />
      </Screen>
    );
  }

  return (
    <SafeAreaView
      edges={["bottom", "left", "right"]}
      style={[styles.fill, { backgroundColor: colors.background }]}
    >
      {title}
      <TerminalView
        feed={feed}
        started={() => {
          watch.pageStarted();
          setPageStarts((n) => n + 1);
        }}
        trouble={() => setTrouble(true)}
      />
      <View style={styles.status}>
        {sight.problem && <Problem>{sight.problem}</Problem>}
        {trouble && <Problem>Something went wrong drawing this terminal.</Problem>}
        <Body muted>{statusLine(sight, describeReach(reach, null))}</Body>
      </View>
    </SafeAreaView>
  );
}

function statusLine(sight: Sight, reach: string): string {
  if (sight.exit) {
    return sight.exit.success
      ? "The program ended."
      : `The program ended with exit code ${sight.exit.code}.`;
  }
  if (sight.gone) return "This terminal is no longer on the computer.";
  if (reach !== "Online") return `${reach}. What is on the computer keeps running.`;
  if (!sight.following) return "Connecting to the terminal.";
  return `Watching. ${controlWords(sight.controller)}`;
}

function controlWords(controller: Controller | null): string {
  switch (controller?.kind) {
    case "device":
      return `${controller.name} has control.`;
    case "local":
      return "The computer has control.";
    default:
      return "Nothing here reaches the program.";
  }
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  status: { paddingHorizontal: 16, paddingVertical: 8, gap: 4 },
});
