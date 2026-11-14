// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One conversation, as Chat or as its terminal (43). Chat is a column at the phone's width, read
 * from the agent's own record on the computer; it is offered when the computer reads that record
 * and the agent is one whose record it can read, and is then the default. Which of the two a
 * conversation was last seen in is remembered on this phone. A terminal that is no conversation's
 * — a shell, a program — is the terminal alone, as is a conversation on a computer from before
 * Chat.
 *
 * The terminal is watched at the computer's size, zoomed out to fit the phone's width and pinched
 * to read, and typed into once the person takes control. Then it is drawn to fit the phone, until
 * they hand it back or the person at the computer types. While it has control, the phone can give
 * the agent a picture: it goes to the computer, and its path there is pasted in, as a dropped
 * file's is.
 *
 * Coming back to this screen, or the app to the foreground, resumes watching. So does a dropped
 * channel. In none of those is anything sent again, and control is not taken back: leaving lets go of
 * it, and only pressing the button takes it.
 */

import {
  ChannelClosed,
  type Controller,
  type HostId,
  type SessionPage,
  type SessionRecordId,
  type SessionSummary,
  type TerminalId,
  type WorkspaceId,
} from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams } from "expo-router";
import { useHeaderHeight } from "expo-router/react-navigation";
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { AppState, KeyboardAvoidingView, Pressable, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { messageOf } from "../lib/accounts";
import { pastedPath } from "../lib/attach";
import { type ChatState, CONVERSATION_READ, READABLE, span } from "../lib/chat";
import { useApp } from "../lib/context";
import { deviceName, fitted, TerminalControl } from "../lib/control";
import type { Reach } from "../lib/presence";
import { Timings, timingWords } from "../lib/timing";
import { type View as Shown, viewChoices } from "../lib/views";
import { type Sight, PageFeed, TerminalWatch } from "../lib/watch";
import { ChatView, useClock } from "../ui/chat";
import { Composer } from "../ui/composer";
import {
  describeReach,
  inFront,
  type Link,
  NoLongerPaired,
  OfProfile,
  reachDot,
  useChannelTo,
  useLive,
} from "../ui/live";
import { Button, Dot, FONTS, Problem, Screen, useColors, Waiting } from "../ui/parts";
import { type Room, TerminalView } from "../ui/terminal-view";

export default function TerminalScreen() {
  const params = useLocalSearchParams<{
    host?: string;
    terminal?: string;
    label?: string;
    workspace?: string;
    session?: string;
  }>();
  if (!params.host || (!params.terminal && !(params.session && params.workspace))) {
    return <Redirect href="/account" />;
  }
  return (
    <OfProfile>
      <Conversation
        host={params.host as HostId}
        terminal={(params.terminal as TerminalId | undefined) ?? null}
        session={(params.session as SessionRecordId | undefined) ?? null}
        label={params.label || "Terminal"}
        workspace={(params.workspace as WorkspaceId | undefined) ?? null}
      />
    </OfProfile>
  );
}

/** The conversation behind a terminal, or a conversation by its record: `null` for none. */
function useConversation(
  link: Link,
  workspace: WorkspaceId | null,
  terminal: TerminalId | null,
  session: SessionRecordId | null,
): SessionSummary | null | undefined {
  const { channel } = link;
  const [found, setFound] = useState<SessionSummary | null | undefined>(undefined);
  const capable = !!channel?.host.capabilities.includes(CONVERSATION_READ);
  useEffect(() => {
    if (!channel || !capable || !workspace) return;
    let stale = false;
    const look = async () => {
      for (const includeEnded of [false, true]) {
        let cursor: SessionPage["next"] = null;
        // Every page: an old conversation can be far down a long list of ended ones.
        for (let pages = 0; pages < 50; pages++) {
          const page: SessionPage = await channel.sessions({
            workspace,
            includeEnded,
            ...(cursor ? { cursor } : {}),
          });
          const match = page.sessions.find((s) =>
            session ? s.id === session : s.terminal?.id === terminal,
          );
          if (match) return match;
          cursor = page.next;
          if (!cursor) break;
        }
      }
      return null;
    };
    look().then(
      (match) => {
        if (!stale) setFound(match);
      },
      () => {
        if (!stale) setFound((held) => held ?? null);
      },
    );
    return () => {
      stale = true;
    };
  }, [channel, capable, workspace, terminal, session]);
  // A computer from before Chat, or a terminal outside any workspace, is the terminal alone.
  const answer = channel && (!capable || !workspace) ? null : found;
  // Once known, it stays known while the channel is away: a dropped channel does not make the
  // screen ask again, and what it shows keeps going.
  const [known, setKnown] = useState<SessionSummary | null | undefined>(undefined);
  if (answer !== undefined && answer !== known) setKnown(answer);
  return answer ?? known;
}

function Conversation({
  host,
  terminal,
  session,
  label,
  workspace,
}: {
  host: HostId;
  terminal: TerminalId | null;
  session: SessionRecordId | null;
  label: string;
  workspace: WorkspaceId | null;
}) {
  const link = useChannelTo(host);
  const found = useConversation(link, workspace, terminal, session);
  const chatOffered = !!found && READABLE.includes(found.harness);
  const [picked, setView] = useState<Shown | null>(null);
  const view: Shown = picked ?? (found ? viewChoices.get(found.id) : "chat");
  const [state, setState] = useState<ChatState>({ kind: "unknown" });
  const choose = useCallback(
    (next: Shown) => {
      if (found) viewChoices.choose(found.id, next);
      setView(next);
    },
    [found],
  );
  const showTerminal = useCallback(() => choose("terminal"), [choose]);
  const termId = found?.terminal?.id ?? terminal;

  if (link.reach?.kind === "gone") {
    return (
      <Screen>
        <Stack.Screen options={{ title: label }} />
        <NoLongerPaired why={link.reach.why} />
      </Screen>
    );
  }
  // Looking up which conversation this is, so as not to show the terminal and then switch: once
  // the computer answers, or says it cannot be reached, the terminal says what it can.
  const asking =
    found === undefined &&
    (link.reach === undefined ||
      link.reach.kind === "connecting" ||
      !!link.channel?.host.capabilities.includes(CONVERSATION_READ));
  if (asking) {
    return (
      <Screen>
        <Stack.Screen options={{ title: label }} />
        <Waiting label="Asking the computer" />
      </Screen>
    );
  }
  if (!chatOffered || !found) {
    // Opened by its conversation, with the computer out of reach: not known yet, not gone.
    if (!termId && found === undefined) {
      return (
        <Screen>
          <Stack.Screen options={{ title: label }} />
          <Problem attention>
            {describeReach(link.reach, link.lastSeenAt)}. The conversation shows once the computer
            can be reached.
          </Problem>
        </Screen>
      );
    }
    if (!termId) {
      return (
        <Screen>
          <Stack.Screen options={{ title: label }} />
          <Problem>This conversation has no terminal on the computer any more.</Problem>
        </Screen>
      );
    }
    return (
      <Watching link={link} host={host} terminal={termId} label={label} workspace={workspace} />
    );
  }
  const bar = (
    <SessionBar
      summary={found}
      state={state}
      view={view}
      choose={choose}
      terminal={termId !== null}
    />
  );
  if (view === "terminal" && termId) {
    return (
      <Watching
        link={link}
        host={host}
        terminal={termId}
        label={found.title || label}
        workspace={workspace}
        bar={bar}
      />
    );
  }
  return (
    <ChatScreen bar={bar} title={found.title || label}>
      <ChatView
        channel={link.channel}
        target={{
          workspace: found.workspace,
          session: found.id,
          terminal: found.terminal?.id ?? null,
          harness: found.harness,
        }}
        showTerminal={showTerminal}
        state={setState}
      />
    </ChatScreen>
  );
}

function ChatScreen({
  bar,
  title,
  children,
}: {
  bar: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  const colors = useColors();
  const headerHeight = useHeaderHeight();
  return (
    <SafeAreaView
      edges={["bottom", "left", "right"]}
      style={[styles.fill, { backgroundColor: colors.background }]}
    >
      <Stack.Screen options={{ title }} />
      <KeyboardAvoidingView
        behavior="padding"
        keyboardVerticalOffset={headerHeight}
        style={styles.fill}
      >
        {bar}
        {children}
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

/** Who the agent is, what it is doing, and Chat or Terminal. */
function SessionBar({
  summary,
  state,
  view,
  choose,
  terminal,
}: {
  summary: SessionSummary;
  state: ChatState;
  view: Shown;
  choose: (view: Shown) => void;
  terminal: boolean;
}) {
  const colors = useColors();
  const now = useClock();
  const words = stateWords(state, summary, now);
  const tone =
    state.kind === "working" || state.kind === "needsYou"
      ? colors.accent
      : state.kind === "failed"
        ? colors.danger
        : summary.process === "running"
          ? colors.online
          : colors.faint;
  return (
    <View style={[styles.bar, { backgroundColor: colors.background, borderColor: colors.line }]}>
      <View style={styles.who}>
        <Text numberOfLines={1} style={[styles.agent, { color: colors.muted }]}>
          {[summary.harness, summary.model].filter(Boolean).join(" · ")}
        </Text>
        <View style={styles.stateRow}>
          <View style={[styles.stateDot, { backgroundColor: tone }]} />
          <Text numberOfLines={1} style={[styles.stateText, { color: colors.faint }]}>
            {words}
          </Text>
        </View>
      </View>
      <View style={[styles.switch, { backgroundColor: colors.raised }]} accessibilityRole="tablist">
        {(["chat", "terminal"] as const).map((option) => {
          const on = view === option;
          const disabled = option === "terminal" && !terminal;
          return (
            <Pressable
              key={option}
              accessibilityRole="tab"
              accessibilityLabel={option === "chat" ? "Chat" : "Terminal"}
              accessibilityState={{ selected: on, disabled }}
              disabled={disabled}
              onPress={() => choose(option)}
              style={[
                styles.option,
                on && {
                  backgroundColor: colors.surface,
                  borderColor: colors.lineStrong,
                  borderWidth: 1,
                },
                disabled && { opacity: 0.4 },
              ]}
            >
              <Text
                style={[
                  styles.optionText,
                  {
                    color: on ? colors.ink : colors.muted,
                    fontFamily: on ? FONTS.semibold : FONTS.medium,
                  },
                ]}
              >
                {option === "chat" ? "Chat" : "Terminal"}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

function stateWords(state: ChatState, summary: SessionSummary, now: number): string {
  switch (state.kind) {
    case "working":
      return `Working · ${span(now - state.since)}`;
    case "needsYou":
      return "Needs you";
    case "done":
      return "Done";
    case "failed":
      return "The turn failed";
    case "ended":
      return "Ended";
    case "idle":
      return "Idle";
    case "unknown":
      return summary.process === "running" ? "Running" : "Ended";
  }
}

function Watching({
  link,
  terminal,
  label,
  workspace,
  bar,
}: {
  link: Link;
  host: HostId;
  terminal: TerminalId;
  label: string;
  /** Its workspace, where a picture for it goes. Without one, none can be given. */
  workspace: WorkspaceId | null;
  /** Above the terminal: the conversation's bar, with the way back to Chat. */
  bar?: React.ReactNode;
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

  const { reach, channel, lastSeenAt } = link;
  const [pageStarts, setPageStarts] = useState(0);
  const [trouble, setTrouble] = useState(false);
  const [draft, setDraft] = useState("");
  const [applicationCursor, setApplicationCursor] = useState(false);
  const [cell, setCell] = useState<{ width: number; height: number } | null>(null);
  const [room, setRoom] = useState<Room | null>(null);
  const [sending, setSending] = useState(false);
  const [sendProblem, setSendProblem] = useState<string | null>(null);
  const headerHeight = useHeaderHeight();

  const timings = useMemo(() => new Timings(), []);
  const timed = useSyncExternalStore(timings.subscribe, timings.getSnapshot);
  const { feed, watch } = useMemo(() => {
    const feed = new PageFeed(
      () => watch.startOver(),
      () => {
        watch.pageCaughtUp();
        // A live screen again only once a new attachment's screen is drawn.
        if (watch.getSnapshot().following) timings.drawn();
      },
      () => timings.heard(),
    );
    const watch = new TerminalWatch(terminal, feed);
    return { feed, watch };
  }, [terminal, timings]);
  const sight = useSyncExternalStore(watch.subscribe, watch.getSnapshot);
  useEffect(() => {
    const stopHearing = control.whenSent(() => timings.typed());
    const coming = AppState.addEventListener("change", (state) => {
      if (state === "active") timings.back();
    });
    return () => {
      stopHearing();
      coming.remove();
    };
  }, [control, timings]);

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
        {bar}
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
              {statusLine(sight, reach, lastSeenAt, hold.holding)}
            </Text>
          </View>
          {__DEV__ && (
            <Text testID="timings" style={[styles.timings, { color: colors.faint }]}>
              {timingWords(timed)}
            </Text>
          )}
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

function statusLine(
  sight: Sight,
  reach: Reach | undefined,
  lastSeenAt: number | null,
  holding: boolean,
): string {
  if (sight.exit) {
    return sight.exit.success
      ? "The program ended."
      : `The program ended with exit code ${sight.exit.code}.`;
  }
  if (sight.gone) return "This terminal is no longer on the computer.";
  // A computer the relay says is offline may be asleep or off: what ran there may have stopped.
  // Any other way of not reaching it is this phone's connection, which stops nothing there.
  if (reach?.kind === "offline") {
    return `${describeReach(reach, lastSeenAt)}. That does not mean the program stopped.`;
  }
  if (reach?.kind !== "online") {
    return `${describeReach(reach, lastSeenAt)}. What is on the computer keeps running.`;
  }
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
  bar: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 16,
    paddingTop: 6,
    paddingBottom: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  who: { flex: 1, gap: 2 },
  agent: { fontFamily: FONTS.mono, fontSize: 12 },
  stateRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  stateDot: { width: 7, height: 7, borderRadius: 4 },
  stateText: { flex: 1, fontFamily: FONTS.mono, fontSize: 12 },
  switch: { flexDirection: "row", borderRadius: 9, padding: 3, gap: 2 },
  option: { borderRadius: 7, paddingHorizontal: 12, paddingVertical: 6 },
  optionText: { fontSize: 13 },
  below: {
    paddingHorizontal: 16,
    paddingVertical: 12,
    gap: 8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  status: { flexDirection: "row", alignItems: "center", gap: 8 },
  statusWords: { flex: 1, fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  timings: { fontFamily: FONTS.mono, fontSize: 11, lineHeight: 16 },
  uncertain: { gap: 8 },
  choices: { flexDirection: "row", gap: 8 },
});
