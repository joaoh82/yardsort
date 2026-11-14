// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A conversation as a phone reads it (43): one column at the phone's width, scrolled up and down
 * only. What the person typed, the agent's steps folded to a line, and what it wrote back, read on
 * the computer from the agent's own record. Questions the agent asks for a permission are answered
 * here. A message is typed into the conversation's terminal, as at the computer.
 */

import {
  type Ask,
  type Channel,
  ClientError,
  type ConversationTurn,
  type SessionRecordId,
  type TerminalId,
  type WorkspaceId,
} from "@yardsort/remote-client";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Pressable,
  ScrollView,
  type ScrollViewInstance,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";

import { messageOf } from "../lib/accounts";
import { ActivityFeed } from "../lib/activity";
import { pastedPath } from "../lib/attach";
import {
  type ChatState,
  ConversationFeed,
  type Seen,
  settled,
  type Step,
  span,
  stateOf,
  stepsOf,
  stepSummary,
} from "../lib/chat";
import { useApp } from "../lib/context";
import { stamp } from "../lib/format";
import { lineCount } from "../lib/markdown";
import { type Said, say } from "../lib/say";
import { inFront } from "./live";
import { Markdown } from "./markdown";
import { Button, FONTS, Problem, useColors, Waiting } from "./parts";

/** Past this many lines a reply or a message is folded behind "Show all". */
const FOLD_LINES = 24;
/** How often to read again while the agent works: its replies are written as it goes. */
const WHILE_WORKING_MS = 4_000;
/** Read again this long after the agent's hooks said something, and once more after that: an
 * agent writes its reply just after the hook that says its turn ended. */
const AFTER_EVENT_MS = [400, 2_500];
/** How often to ask whether the conversation's terminal ended or went quiet. */
const SEEN_EVERY_MS = 5_000;
/** Near enough the bottom to stay there as more comes. */
const AT_BOTTOM_PX = 48;

export interface ChatTarget {
  workspace: WorkspaceId;
  session: SessionRecordId;
  /** Its terminal while it has one: what a message is typed into. */
  terminal: TerminalId | null;
  /** Who the agent is, as the computer names it: `claude`, `codex`, … */
  harness: string;
}

export function ChatView({
  channel,
  target,
  showTerminal,
  state,
}: {
  channel: Channel | null;
  target: ChatTarget;
  /** Go to the terminal: for its keys, or what Chat cannot show. */
  showTerminal: () => void;
  /** Hear the conversation's state, for the bar above. */
  state: (state: ChatState) => void;
}) {
  const colors = useColors();
  const { pickImage } = useApp();
  const feed = useMemo(
    () => new ConversationFeed(target.workspace, target.session),
    [target.workspace, target.session],
  );
  const chat = useSyncExternalStore(feed.subscribe, feed.getSnapshot);
  const activityFeed = useMemo(() => new ActivityFeed(target.workspace), [target.workspace]);
  const activity = useSyncExternalStore(activityFeed.subscribe, activityFeed.getSnapshot);
  const events = useMemo(
    () => activity.events.filter((event) => event.session === target.session),
    [activity.events, target.session],
  );
  const now = useClock();
  // What the conversation's terminal shows: whether it ended, and how long it has been quiet.
  const [seen, setSeen] = useState<Seen | null>(null);
  useEffect(() => {
    if (!channel) return;
    let stale = false;
    const look = async () => {
      try {
        const page = await channel.sessions({ workspace: target.workspace });
        const running = page.sessions.find((s) => s.id === target.session);
        const terminal = running?.terminal ?? null;
        if (!stale) {
          setSeen({
            ended: !running || !terminal || terminal.activity === "exited",
            quietMs: terminal?.activity === "quiet" ? terminal.idleMs : 0,
          });
        }
      } catch {
        // Said by the reach, and asked again in a moment.
      }
    };
    void look();
    const every = setInterval(() => void look(), SEEN_EVERY_MS);
    return () => {
      stale = true;
      clearInterval(every);
    };
  }, [channel, target.workspace, target.session]);
  const current = useMemo(
    () => settled(stateOf(events, chat.asks, chat.answersAsks), seen, now),
    [events, chat.asks, chat.answersAsks, seen, now],
  );
  const steps = useMemo(() => stepsOf(events, chat.turns), [events, chat.turns]);

  useEffect(() => state(current), [current, state]);

  // Read, and follow what the agent's hooks record, while the computer can be reached.
  useEffect(() => {
    if (!channel) return;
    void feed.read(channel);
    void activityFeed.follow(channel);
    return () => activityFeed.stop();
  }, [channel, feed, activityFeed]);

  const newest = events.at(-1)?.id;
  useEffect(() => {
    if (!channel || newest === undefined) return;
    const timers = AFTER_EVENT_MS.map((ms) => setTimeout(() => void feed.read(channel), ms));
    return () => timers.forEach(clearTimeout);
  }, [channel, feed, newest]);

  const working = current.kind === "working" || current.kind === "needsYou";
  useEffect(() => {
    if (!channel || !working) return;
    const every = setInterval(() => void feed.read(channel), WHILE_WORKING_MS);
    return () => clearInterval(every);
  }, [channel, feed, working]);

  // Down at the newest unless the person scrolled up; then a button says there is more.
  const scroller = useRef<ScrollViewInstance>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [unseen, setUnseen] = useState(false);
  const lastHeight = useRef(0);
  const onScroll = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    const bottom = contentSize.height - layoutMeasurement.height - contentOffset.y < AT_BOTTOM_PX;
    setAtBottom(bottom);
    if (bottom) setUnseen(false);
  };
  const onContentSize = (_width: number, height: number) => {
    const grew = height > lastHeight.current;
    lastHeight.current = height;
    if (!grew) return;
    if (atBottom) scroller.current?.scrollToEnd({ animated: false });
    else setUnseen(true);
  };

  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [said, setSaid] = useState<Said | null>(null);
  const ended = target.terminal === null || current.kind === "ended";

  const type = async (
    text: string,
    enter: boolean,
    options: { takeOver?: boolean; paste?: boolean } = {},
  ) => {
    if (!channel || !target.terminal) return false;
    setSending(true);
    setSaid(null);
    const result = await say(
      channel,
      target.terminal,
      { text, enter, ...(options.paste ? { paste: true } : {}) },
      { takeOver: options.takeOver ?? false },
    );
    setSending(false);
    if (result.kind !== "sent") setSaid(result);
    return result.kind === "sent";
  };

  const send = async (takeOver = false) => {
    const text = draft;
    if (text.trim() === "") return;
    if (await type(text, true, { takeOver })) {
      setDraft("");
      setAtBottom(true);
      scroller.current?.scrollToEnd({ animated: true });
      void feed.read(channel as Channel);
    }
  };

  const stop = () => void type("\u001b", false);

  const canAttach =
    !ended && !!channel?.host.capabilities.includes("upload.begin") && channel !== null;
  const attach = async () => {
    if (!channel || !target.terminal) return;
    try {
      const picked = await inFront(pickImage);
      if (!picked) return;
      setSending(true);
      const path = await channel.upload(target.workspace, picked.name, picked.bytes);
      setSending(false);
      await type(pastedPath(path, channel.host.platform), false, { paste: true });
    } catch (error) {
      setSending(false);
      setSaid({ kind: "refused", why: messageOf(error) });
    }
  };

  const lastTurn = chat.turns.at(-1)?.n;

  return (
    <View style={styles.fill}>
      <ScrollView
        ref={scroller}
        testID="chat"
        style={styles.fill}
        contentContainerStyle={styles.column}
        onScroll={onScroll}
        scrollEventThrottle={64}
        onContentSizeChange={onContentSize}
        keyboardShouldPersistTaps="handled"
      >
        {chat.hasEarlier && channel && (
          <Button
            title="Show earlier turns"
            kind="quiet"
            onPress={() => void feed.earlier(channel)}
          />
        )}
        {!chat.loaded && !chat.problem && <Waiting label="Reading the conversation" />}
        {chat.unreadable && (
          <View style={styles.notice}>
            <Problem attention>
              The computer could not read this conversation: {chat.unreadable}. Its terminal shows
              everything.
            </Problem>
            <Button title="Open the terminal" onPress={showTerminal} />
          </View>
        )}
        {chat.loaded && chat.turns.length === 0 && !chat.unreadable && (
          <Text style={[styles.quiet, { color: colors.muted }]}>
            Nothing has been said in this conversation yet.
          </Text>
        )}
        {chat.turns.map((turn) => (
          <TurnView
            key={turn.n}
            turn={turn}
            harness={target.harness}
            steps={steps.get(turn.n) ?? []}
            live={turn.n === lastTurn && current.kind === "working"}
            now={now}
            showTerminal={showTerminal}
          />
        ))}
        {current.kind === "working" && (
          <WorkingLine
            since={current.since}
            now={now}
            steps={lastTurn === undefined ? 0 : (steps.get(lastTurn)?.length ?? 0)}
            stop={!ended && channel ? stop : undefined}
          />
        )}
        {chat.asks.map((ask) => (
          <AskCard
            key={ask.id}
            ask={ask}
            harness={target.harness}
            answer={
              chat.answersAsks && channel
                ? async (decision) => {
                    try {
                      await channel.answerAsk({
                        workspace: target.workspace,
                        session: target.session,
                        ask: ask.id,
                        decision,
                      });
                      feed.answered(ask.id);
                    } catch (error) {
                      // Answered already, or the agent moved on: it goes. Anything else may not
                      // have reached the computer, so the question stays to be answered.
                      if (error instanceof ClientError && error.code === "not_found") {
                        feed.answered(ask.id);
                      } else {
                        setSaid({ kind: "refused", why: messageOf(error) });
                      }
                    }
                    void feed.read(channel);
                  }
                : undefined
            }
            showTerminal={showTerminal}
          />
        ))}
        {current.kind === "needsYou" && chat.asks.length === 0 && (
          <View
            style={[styles.ask, { backgroundColor: colors.accentTint, borderColor: colors.accent }]}
          >
            <Text style={[styles.askHead, { color: colors.accentText }]}>NEEDS YOU</Text>
            <Text style={[styles.askTitle, { color: colors.ink }]}>
              {target.harness} is waiting for an answer in its terminal.
            </Text>
            <Button title="Open the terminal" onPress={showTerminal} />
          </View>
        )}
        {chat.problem && <Problem>{chat.problem}</Problem>}
      </ScrollView>

      {unseen && !atBottom && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Latest"
          onPress={() => {
            scroller.current?.scrollToEnd({ animated: true });
            setUnseen(false);
          }}
          style={[
            styles.latest,
            { backgroundColor: colors.raised, borderColor: colors.lineStrong },
          ]}
        >
          <Text style={[styles.latestText, { color: colors.ink }]}>↓ Latest</Text>
        </Pressable>
      )}

      <View
        style={[styles.composer, { borderColor: colors.line, backgroundColor: colors.background }]}
      >
        {said && said.kind !== "sent" && (
          <View style={styles.notice}>
            <Problem attention={said.kind === "uncertain"}>{said.why}</Problem>
            {said.kind === "held" && /device/i.test(said.why) && (
              <Button title="Take over and send" onPress={() => void send(true)} />
            )}
          </View>
        )}
        {ended ? (
          <Text style={[styles.quiet, { color: colors.muted }]}>
            This conversation has ended. Its record stays on the computer.
          </Text>
        ) : (
          <>
            <View style={styles.row}>
              {canAttach && (
                <RoundButton
                  label="+"
                  name="Give a picture"
                  busy={sending}
                  onPress={() => void attach()}
                />
              )}
              <TextInput
                accessibilityLabel={`Message ${target.harness}`}
                placeholder={`Message ${target.harness}…`}
                placeholderTextColor={colors.faint}
                value={draft}
                onChangeText={setDraft}
                multiline
                style={[
                  styles.field,
                  {
                    color: colors.ink,
                    backgroundColor: colors.surface,
                    borderColor: colors.lineStrong,
                  },
                ]}
              />
              <RoundButton
                label="↑"
                name="Send"
                primary
                busy={sending}
                disabled={draft.trim() === "" || !channel}
                onPress={() => void send()}
              />
            </View>
            <View style={styles.under}>
              <Text style={[styles.underText, { color: colors.faint }]}>
                {current.kind === "working"
                  ? "It is working: a message waits for its turn."
                  : "Typed into its terminal on the computer."}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Keys"
                onPress={showTerminal}
                style={[styles.keys, { borderColor: colors.lineStrong }]}
              >
                <Text style={[styles.keysText, { color: colors.muted }]}>Keys</Text>
              </Pressable>
            </View>
          </>
        )}
      </View>
    </View>
  );
}

function TurnView({
  turn,
  harness,
  steps,
  live,
  now,
  showTerminal,
}: {
  turn: ConversationTurn;
  harness: string;
  steps: Step[];
  live: boolean;
  now: number;
  showTerminal: () => void;
}) {
  const colors = useColors();
  const reply = turn.said.at(-1);
  const commentary = turn.said.slice(0, -1);
  return (
    <View style={styles.turn} accessibilityLabel={`Turn ${turn.n}`}>
      <View style={styles.you}>
        <Text style={[styles.meta, { color: colors.faint }]}>YOU · {stamp(turn.at, now)}</Text>
        <View style={[styles.bubble, { backgroundColor: colors.raised, borderColor: colors.line }]}>
          <Folded text={turn.user} plain />
        </View>
      </View>
      {steps.length > 0 && <Steps steps={steps} open={live} />}
      {(commentary.length > 0 || reply !== undefined) && (
        <View style={styles.agent}>
          <Text style={[styles.meta, { color: colors.faint }]}>{harness.toUpperCase()}</Text>
          {commentary.map((text, i) => (
            <Markdown key={i} text={text} muted />
          ))}
          {reply !== undefined && !live && <Folded text={reply} />}
          {reply !== undefined && live && <Markdown text={reply} muted />}
        </View>
      )}
      {turn.cut && (
        <Pressable accessibilityRole="button" onPress={showTerminal}>
          <Text style={[styles.cut, { color: colors.accentText }]}>
            Part of this turn is too long to send to a phone. The terminal shows it.
          </Text>
        </Pressable>
      )}
    </View>
  );
}

/** Text that folds past [`FOLD_LINES`] lines, with a button for the rest. */
function Folded({ text, plain }: { text: string; plain?: boolean }) {
  const colors = useColors();
  const lines = lineCount(text);
  const [open, setOpen] = useState(lines <= FOLD_LINES);
  const shown = open ? text : text.split("\n").slice(0, FOLD_LINES).join("\n");
  return (
    <View style={styles.folded}>
      {plain ? (
        <Text selectable style={[styles.body, { color: colors.ink }]}>
          {shown}
        </Text>
      ) : (
        <Markdown text={shown} />
      )}
      {!open && (
        <Pressable accessibilityRole="button" onPress={() => setOpen(true)}>
          <Text style={[styles.more, { color: colors.accentText }]}>
            Show all · {lines - FOLD_LINES} more lines
          </Text>
        </Pressable>
      )}
    </View>
  );
}

function Steps({ steps, open: startOpen }: { steps: Step[]; open: boolean }) {
  const colors = useColors();
  // Open while the agent works, then as the person leaves it.
  const [chosen, setOpen] = useState<boolean | null>(null);
  const open = chosen ?? startOpen;
  const first = steps[0] as Step;
  const last = steps.at(-1) as Step;
  const took = last.at + (last.durationMs ?? 0) - first.at;
  const count = `${steps.length} ${steps.length === 1 ? "step" : "steps"}`;
  return (
    <View style={[styles.steps, { borderColor: colors.line }]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${count}, ${stepSummary(steps)}`}
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen(!open)}
        style={styles.stepsHead}
      >
        <Text style={[styles.chevron, { color: colors.faint }]}>{open ? "▾" : "▸"}</Text>
        <Text style={[styles.stepsCount, { color: colors.muted }]}>{count}</Text>
        <Text numberOfLines={1} style={[styles.stepsSummary, { color: colors.faint }]}>
          {stepSummary(steps)}
        </Text>
        <Text style={[styles.stepTime, { color: colors.faint }]}>{span(took)}</Text>
      </Pressable>
      {open &&
        steps.map((step) => (
          <View key={step.id} style={styles.step}>
            <View
              style={[
                styles.stepDot,
                {
                  backgroundColor:
                    step.state === "running"
                      ? colors.accent
                      : step.state === "failed"
                        ? colors.danger
                        : colors.online,
                },
              ]}
            />
            <Text style={[styles.stepTool, { color: colors.ink }]}>{step.tool}</Text>
            <Text numberOfLines={1} style={[styles.stepPath, { color: colors.muted }]}>
              {step.path ?? ""}
            </Text>
            <Text style={[styles.stepTime, { color: colors.faint }]}>
              {step.state === "running"
                ? "now"
                : step.durationMs !== null
                  ? span(step.durationMs)
                  : ""}
            </Text>
          </View>
        ))}
    </View>
  );
}

function WorkingLine({
  since,
  now,
  steps,
  stop,
}: {
  since: number;
  now: number;
  steps: number;
  stop: (() => void) | undefined;
}) {
  const colors = useColors();
  return (
    <View style={styles.working} accessibilityLabel="Working">
      <View style={[styles.stepDot, { backgroundColor: colors.accent }]} />
      <Text style={[styles.workingText, { color: colors.muted }]}>
        Working · {span(now - since)}
        {steps > 0 ? ` · ${steps} ${steps === 1 ? "step" : "steps"}` : ""}
      </Text>
      {stop && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Stop"
          onPress={stop}
          style={[styles.stop, { borderColor: colors.lineStrong }]}
        >
          <View style={[styles.stopMark, { backgroundColor: colors.danger }]} />
          <Text style={[styles.stopText, { color: colors.ink }]}>Stop</Text>
        </Pressable>
      )}
    </View>
  );
}

function AskCard({
  ask,
  harness,
  answer,
  showTerminal,
}: {
  ask: Ask;
  harness: string;
  answer: ((decision: "allow" | "deny") => Promise<void>) | undefined;
  showTerminal: () => void;
}) {
  const colors = useColors();
  const [busy, setBusy] = useState<"allow" | "deny" | null>(null);
  // Returns nothing: a press starts the answer, and the card goes when it has gone.
  const press = (decision: "allow" | "deny") => () => {
    if (!answer) return;
    setBusy(decision);
    void answer(decision).finally(() => setBusy(null));
  };
  return (
    <View
      accessibilityLabel={`${harness} asks to use ${ask.tool}`}
      style={[styles.ask, { backgroundColor: colors.accentTint, borderColor: colors.accent }]}
    >
      <Text style={[styles.askHead, { color: colors.accentText }]}>
        NEEDS YOU · {stamp(ask.askedAt)}
      </Text>
      <Text style={[styles.askTitle, { color: colors.ink }]}>
        {harness} asks to use {ask.tool}
      </Text>
      {(ask.command ?? ask.path) && (
        <View style={[styles.code, { backgroundColor: colors.raised }]}>
          <Text selectable style={[styles.codeText, { color: colors.ink }]}>
            {ask.command ?? ask.path}
          </Text>
        </View>
      )}
      {answer ? (
        <>
          <View style={styles.row}>
            <View style={styles.fill}>
              <Button
                title="Deny"
                busy={busy === "deny"}
                disabled={busy !== null}
                onPress={press("deny")}
              />
            </View>
            <View style={styles.fill}>
              <Button
                title="Allow once"
                kind="primary"
                busy={busy === "allow"}
                disabled={busy !== null}
                onPress={press("allow")}
              />
            </View>
          </View>
          <Pressable accessibilityRole="button" onPress={showTerminal}>
            <Text style={[styles.other, { color: colors.accentText }]}>
              Other answers: open the terminal
            </Text>
          </Pressable>
        </>
      ) : (
        <Button title="Answer in the terminal" onPress={showTerminal} />
      )}
    </View>
  );
}

function RoundButton({
  label,
  name,
  primary,
  busy,
  disabled,
  onPress,
}: {
  label: string;
  name: string;
  primary?: boolean;
  busy?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  const colors = useColors();
  const off = (disabled ?? false) || (busy ?? false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={name}
      accessibilityState={{ disabled: off, busy: busy ?? false }}
      disabled={off}
      onPress={onPress}
      style={({ pressed }) => [
        styles.round,
        {
          backgroundColor: primary ? colors.accent : colors.raised,
          opacity: off ? 0.5 : pressed ? 0.7 : 1,
        },
      ]}
    >
      <Text style={[styles.roundText, { color: primary ? colors.onAccent : colors.muted }]}>
        {label}
      </Text>
    </Pressable>
  );
}

/** The time now, a second at a time, for "Working · 48s". */
export function useClock(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const every = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(every);
  }, []);
  return now;
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  column: { paddingHorizontal: 16, paddingVertical: 16, gap: 18 },
  quiet: { fontFamily: FONTS.regular, fontSize: 14, lineHeight: 20 },
  notice: { gap: 8 },
  turn: { gap: 14 },
  you: { gap: 6 },
  agent: { gap: 10 },
  meta: { fontFamily: FONTS.mono, fontSize: 11, lineHeight: 14, letterSpacing: 0.4 },
  bubble: {
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  body: { fontFamily: FONTS.regular, fontSize: 15, lineHeight: 22 },
  folded: { gap: 8 },
  more: { fontFamily: FONTS.medium, fontSize: 14, lineHeight: 20 },
  cut: { fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  steps: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 10, paddingHorizontal: 12 },
  stepsHead: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: 10 },
  chevron: { fontSize: 13, width: 12 },
  stepsCount: { fontFamily: FONTS.medium, fontSize: 14 },
  stepsSummary: { flex: 1, fontFamily: FONTS.mono, fontSize: 12 },
  step: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: 7 },
  stepDot: { width: 7, height: 7, borderRadius: 4 },
  stepTool: { fontFamily: FONTS.monoMedium, fontSize: 13 },
  stepPath: { flex: 1, fontFamily: FONTS.mono, fontSize: 13 },
  stepTime: { fontFamily: FONTS.mono, fontSize: 11 },
  working: { flexDirection: "row", alignItems: "center", gap: 10 },
  workingText: { flex: 1, fontFamily: FONTS.mono, fontSize: 12 },
  stop: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  stopMark: { width: 9, height: 9, borderRadius: 1 },
  stopText: { fontFamily: FONTS.medium, fontSize: 13 },
  ask: { borderWidth: 1, borderRadius: 12, padding: 14, gap: 12 },
  askHead: { fontFamily: FONTS.mono, fontSize: 11, letterSpacing: 0.4 },
  askTitle: { fontFamily: FONTS.semibold, fontSize: 16, lineHeight: 22 },
  code: { borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10 },
  codeText: { fontFamily: FONTS.mono, fontSize: 13, lineHeight: 20 },
  other: { fontFamily: FONTS.medium, fontSize: 14, textAlign: "center" },
  latest: {
    position: "absolute",
    alignSelf: "center",
    bottom: 120,
    borderWidth: 1,
    borderRadius: 18,
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  latestText: { fontFamily: FONTS.medium, fontSize: 13 },
  composer: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingTop: 10,
    paddingBottom: 10,
    gap: 8,
  },
  row: { flexDirection: "row", alignItems: "flex-end", gap: 8 },
  field: {
    flex: 1,
    minHeight: 40,
    maxHeight: 140,
    borderWidth: 1,
    borderRadius: 20,
    paddingHorizontal: 14,
    paddingTop: 10,
    paddingBottom: 10,
    fontFamily: FONTS.regular,
    fontSize: 15,
  },
  round: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: "center",
    justifyContent: "center",
  },
  roundText: { fontFamily: FONTS.semibold, fontSize: 18 },
  under: { flexDirection: "row", alignItems: "center", gap: 8 },
  underText: { flex: 1, fontFamily: FONTS.regular, fontSize: 12 },
  keys: { borderWidth: 1, borderRadius: 6, paddingHorizontal: 10, paddingVertical: 4 },
  keysText: { fontFamily: FONTS.mono, fontSize: 12 },
});
