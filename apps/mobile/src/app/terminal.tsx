// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One terminal: watched at the computer's size, scrolling sideways when that is wider than the phone,
 * and typed into once the person takes control. Then it is drawn to fit the phone, until they hand it
 * back or the person at the computer types.
 *
 * Coming back to this screen, or the app to the foreground, resumes watching. So does a dropped
 * channel. In none of those is anything sent again, and control is not taken back: leaving lets go of
 * it, and only pressing the button takes it.
 */

import type { Controller, HostId, TerminalId } from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams } from "expo-router";
import { useHeaderHeight } from "expo-router/react-navigation";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { KeyboardAvoidingView, StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { useApp } from "../lib/context";
import { deviceName, fitted, TerminalControl } from "../lib/control";
import { type Sight, PageFeed, TerminalWatch } from "../lib/watch";
import { Composer } from "../ui/composer";
import { describeReach, NoLongerPaired, OfProfile, useChannelTo, useLive } from "../ui/live";
import { Body, Button, Problem, Screen, useColors } from "../ui/parts";
import { type Room, TerminalView } from "../ui/terminal-view";

export default function TerminalScreen() {
  const params = useLocalSearchParams<{ host?: string; terminal?: string; label?: string }>();
  if (!params.host || !params.terminal) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Watching
        host={params.host as HostId}
        terminal={params.terminal as TerminalId}
        label={params.label || "Terminal"}
      />
    </OfProfile>
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
  const { accounts } = useApp();
  const me = accounts.getSnapshot().active?.device ?? null;
  const control = useMemo(() => new TerminalControl(terminal, me), [terminal, me]);
  const hold = useSyncExternalStore(control.subscribe, control.getSnapshot);

  // Before the channel's own hook, whose effect closes the channel: letting go has to go first.
  const live = useLive();
  useEffect(() => {
    if (!live) control.release();
  }, [live, control]);
  useEffect(() => () => control.release(), [control]);

  const { reach, channel } = useChannelTo(host);
  const [pageStarts, setPageStarts] = useState(0);
  const [trouble, setTrouble] = useState(false);
  const [draft, setDraft] = useState("");
  const [applicationCursor, setApplicationCursor] = useState(false);
  const [cell, setCell] = useState<{ width: number; height: number } | null>(null);
  const [room, setRoom] = useState<Room | null>(null);
  const headerHeight = useHeaderHeight();

  const { feed, watch } = useMemo(() => {
    const feed = new PageFeed(
      () => watch.startOver(),
      () => watch.pageCaughtUp(),
    );
    const watch = new TerminalWatch(terminal, feed);
    return { feed, watch };
  }, [terminal]);
  const sight = useSyncExternalStore(watch.subscribe, watch.getSnapshot);

  useEffect(() => control.seen(sight), [control, sight]);

  useEffect(() => {
    if (!channel || pageStarts === 0) return;
    void watch.follow(channel);
    return () => watch.stop();
  }, [channel, pageStarts, watch]);

  useEffect(() => {
    if (!room || !cell) return;
    // The page may start under a see-through header, which covers its first rows.
    const covered = Math.max(0, headerHeight - room.top);
    const size = fitted({ width: room.width, height: room.height - covered }, cell);
    if (size) control.fit(size);
  }, [room, cell, headerHeight, control]);

  const title = <Stack.Screen options={{ title: label }} />;

  if (reach?.kind === "gone") {
    return (
      <Screen>
        {title}
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  const ended = sight.exit !== null || sight.gone;
  const canTake = channel !== null && sight.following && !ended && !hold.holding;
  const held = sight.controller?.kind === "device" && sight.controller.device !== me;

  return (
    <SafeAreaView
      edges={["bottom", "left", "right"]}
      style={[styles.fill, { backgroundColor: colors.background }]}
    >
      {title}
      <KeyboardAvoidingView
        behavior="padding"
        keyboardVerticalOffset={room?.top ?? 0}
        style={styles.fill}
      >
        <TerminalView
          feed={feed}
          started={() => {
            setApplicationCursor(false);
            watch.pageStarted();
            setPageStarts((n) => n + 1);
          }}
          trouble={() => setTrouble(true)}
          room={setRoom}
          cell={setCell}
          cursorKeys={setApplicationCursor}
        />
        <View style={styles.below}>
          {sight.problem && <Problem>{sight.problem}</Problem>}
          {trouble && <Problem>Something went wrong drawing this terminal.</Problem>}
          {hold.notice && <Problem>{hold.notice}</Problem>}
          {hold.uncertain && (
            <View style={styles.uncertain}>
              <Problem>
                This may not have reached the computer, or may be waiting there for Enter. Look at
                the screen before sending it again: {hold.uncertain.said}
              </Problem>
              <View style={styles.choices}>
                {hold.uncertain.text !== null && (
                  <Button
                    title="Put it back"
                    onPress={() => {
                      const text = control.settleUncertain();
                      if (text !== null) setDraft(draft ? `${draft}\n${text}` : text);
                    }}
                  />
                )}
                <Button title="Dismiss" onPress={() => void control.settleUncertain()} />
              </View>
            </View>
          )}
          <Body muted>{statusLine(sight, describeReach(reach, null), hold.holding)}</Body>
          {canTake && (
            <Button
              title={
                held && sight.controller?.kind === "device"
                  ? takeOverTitle(sight.controller.name)
                  : "Take control"
              }
              busy={hold.asking}
              onPress={() => void control.take(channel)}
            />
          )}
          {(hold.holding || draft.length > 0) && !ended && (
            <Composer
              control={control}
              holding={hold.holding}
              draft={draft}
              setDraft={setDraft}
              applicationCursor={applicationCursor}
            />
          )}
          {hold.holding && <Button title="Hand back" onPress={() => control.release()} />}
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

function takeOverTitle(name: string): string {
  return name ? `Take over from ${name}` : "Take over from another device";
}

function statusLine(sight: Sight, reach: string, holding: boolean): string {
  if (sight.exit) {
    return sight.exit.success
      ? "The program ended."
      : `The program ended with exit code ${sight.exit.code}.`;
  }
  if (sight.gone) return "This terminal is no longer on the computer.";
  if (reach !== "Online") return `${reach}. What is on the computer keeps running.`;
  if (!sight.following) return "Connecting to the terminal.";
  if (holding) return "You have control. The person at the computer can take it back by typing.";
  return `Watching. ${controlWords(sight.controller)}`;
}

function controlWords(controller: Controller | null): string {
  switch (controller?.kind) {
    case "device":
      return `${deviceName(controller.name)} has control.`;
    case "local":
      return "The person at the computer has control.";
    default:
      return "Nobody has control.";
  }
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  below: { paddingHorizontal: 16, paddingVertical: 8, gap: 8 },
  uncertain: { gap: 8 },
  choices: { flexDirection: "row", gap: 8 },
});
