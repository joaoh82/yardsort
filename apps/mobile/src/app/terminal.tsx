// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One terminal: watched at the computer's size, scrolling sideways when that is wider than the phone,
 * and typed into once the person takes control. Then it is drawn to fit the phone, until they hand it
 * back or the person at the computer types. While it has control, the phone can give the agent a
 * picture: it goes to the computer, and its path there is pasted in, as a dropped file's is.
 *
 * Coming back to this screen, or the app to the foreground, resumes watching. So does a dropped
 * channel. In none of those is anything sent again, and control is not taken back: leaving lets go of
 * it, and only pressing the button takes it.
 */

import {
  ChannelClosed,
  type Controller,
  type HostId,
  type TerminalId,
  type WorkspaceId,
} from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams } from "expo-router";
import { useHeaderHeight } from "expo-router/react-navigation";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { KeyboardAvoidingView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { messageOf } from "../lib/accounts";
import { pastedPath } from "../lib/attach";
import { useApp } from "../lib/context";
import { deviceName, fitted, TerminalControl } from "../lib/control";
import { type Sight, PageFeed, TerminalWatch } from "../lib/watch";
import { Composer } from "../ui/composer";
import {
  describeReach,
  inFront,
  NoLongerPaired,
  OfProfile,
  reachDot,
  useChannelTo,
  useLive,
} from "../ui/live";
import { Button, Dot, FONTS, Problem, Screen, useColors } from "../ui/parts";
import { type Room, TerminalView } from "../ui/terminal-view";

export default function TerminalScreen() {
  const params = useLocalSearchParams<{
    host?: string;
    terminal?: string;
    label?: string;
    workspace?: string;
  }>();
  if (!params.host || !params.terminal) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Watching
        host={params.host as HostId}
        terminal={params.terminal as TerminalId}
        label={params.label || "Terminal"}
        workspace={(params.workspace as WorkspaceId | undefined) ?? null}
      />
    </OfProfile>
  );
}

function Watching({
  host,
  terminal,
  label,
  workspace,
}: {
  host: HostId;
  terminal: TerminalId;
  label: string;
  /** Its workspace, where a picture for it goes. Without one, none can be given. */
  workspace: WorkspaceId | null;
}) {
  const colors = useColors();
  const { accounts, pickImage } = useApp();
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
  const [sending, setSending] = useState(false);
  const [sendProblem, setSendProblem] = useState<string | null>(null);
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

  const canSend = workspace !== null && !!channel?.host.capabilities.includes("upload.begin");
  const sendPicture = async () => {
    if (!channel || !workspace) return;
    setSendProblem(null);
    try {
      const picked = await inFront(pickImage);
      if (!picked) return;
      setSending(true);
      const path = await channel.upload(workspace, picked.name, picked.bytes);
      if (!control.paste(pastedPath(path, channel.host.platform), picked.name)) {
        setSendProblem(
          "The picture is on the computer, but control went before it could be given.",
        );
      }
    } catch (error) {
      setSendProblem(
        error instanceof ChannelClosed
          ? "The picture did not reach the computer. Try again."
          : messageOf(error),
      );
    } finally {
      setSending(false);
    }
  };

  const ended = sight.exit !== null || sight.gone;
  const canTake = channel !== null && sight.following && !ended && !hold.holding;
  const held = sight.controller?.kind === "device" && sight.controller.device !== me;

  return (
    <SafeAreaView
      edges={["bottom", "left", "right"]}
      style={[styles.fill, { backgroundColor: colors.surface }]}
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
        <View style={[styles.below, { borderColor: colors.line }]}>
          {sight.problem && <Problem>{sight.problem}</Problem>}
          {trouble && <Problem>Something went wrong drawing this terminal.</Problem>}
          {hold.notice && <Problem>{hold.notice}</Problem>}
          {sendProblem && <Problem>{sendProblem}</Problem>}
          {hold.uncertain && (
            <View style={styles.uncertain}>
              <Problem attention>
                This may not have reached the computer, or may be waiting there for Enter. Look at
                the screen before sending it again: {hold.uncertain.said}
              </Problem>
              <View style={styles.choices}>
                {hold.uncertain.text !== null && (
                  <View style={styles.fill}>
                    <Button
                      title="Put it back"
                      onPress={() => {
                        const text = control.settleUncertain();
                        if (text !== null) setDraft(draft ? `${draft}\n${text}` : text);
                      }}
                    />
                  </View>
                )}
                <View style={styles.fill}>
                  <Button
                    title="Dismiss"
                    kind="quiet"
                    onPress={() => void control.settleUncertain()}
                  />
                </View>
              </View>
            </View>
          )}
          <View style={styles.status}>
            <Dot kind={stopped(sight) ? "bad" : reachDot(reach)} />
            <Text style={[styles.statusWords, { color: colors.muted }]}>
              {statusLine(sight, describeReach(reach, null), hold.holding)}
            </Text>
          </View>
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
              {...(canSend ? { attach: { busy: sending, press: () => void sendPicture() } } : {})}
            />
          )}
          {hold.holding && (
            <Button title="Hand back" kind="quiet" onPress={() => control.release()} />
          )}
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

function takeOverTitle(name: string): string {
  return name ? `Take over from ${name}` : "Take over from another device";
}

/** Told why not, and not trying again until the channel is: not on its way, whatever the computer is. */
function stopped(sight: Sight): boolean {
  return sight.problem !== null && !sight.following;
}

function statusLine(sight: Sight, reach: string, holding: boolean): string {
  if (sight.exit) {
    return sight.exit.success
      ? "The program ended."
      : `The program ended with exit code ${sight.exit.code}.`;
  }
  if (sight.gone) return "This terminal is no longer on the computer.";
  if (reach !== "Online") return `${reach}. What is on the computer keeps running.`;
  if (stopped(sight)) return "Not watching this terminal.";
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
  below: {
    paddingHorizontal: 16,
    paddingVertical: 12,
    gap: 8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  status: { flexDirection: "row", alignItems: "center", gap: 8 },
  statusWords: { flex: 1, fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  uncertain: { gap: 8 },
  choices: { flexDirection: "row", gap: 8 },
});
