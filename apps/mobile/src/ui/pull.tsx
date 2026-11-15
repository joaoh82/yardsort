// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A pull request as the phone shows it (45): the badge on a workspace's row, the card with its
 * checks and reviews, and a row of the list of them all. Read-only: nothing here changes a pull
 * request, and the one thing to press that leaves the app opens its page on the forge.
 *
 * A colour is never the only thing that says something: a badge's words, a check's word and a
 * mark's shape say it too.
 */

import {
  type Channel,
  ChannelClosed,
  ClientError,
  type PullDetail,
  type PullRef,
  type PullsOf,
  type PullSummary,
} from "@yardsort/remote-client";
import { useEffect, useState } from "react";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";

import { messageOf } from "../lib/accounts";
import {
  ago,
  badgeWords,
  changedWords,
  checksShort,
  checksVerdict,
  checksWords,
  keyOf,
  openWords,
  type PullsRead,
  type PullTone,
  readPulls,
  readsPulls,
  refOf,
  reviewLine,
  stateWords,
  toneOf,
  toneWords,
} from "../lib/pulls";
import { useLive } from "./live";
import { Button, type Colors, FONTS, Icon, useColors } from "./parts";

/** How often a screen showing pull requests asks for them again. */
export const PULLS_EVERY_MS = 60_000;
/** How soon it asks again when the computer said it was reading them, and how many times. */
export const PULLS_SOON_MS = 4_000;
const SOON_TIMES = 3;

/** The time now, half a minute at a time: for _updated 5 min ago_. */
export function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const every = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(every);
  }, []);
  return now;
}

/**
 * A computer's pull requests of one kind, while this screen is in front: asked for at once, again
 * a few seconds later when the computer was still reading them, and then every minute. `null`
 * until the computer has answered, and for one that tells of none.
 */
export function usePulls(
  channel: Channel | null,
  of: PullsOf,
  asked: number,
): { read: PullsRead | null; problem: string | null } {
  const live = useLive();
  // Kept with the kind it was read for: a list that changes kind never shows the last kind's.
  const [held, setHeld] = useState<{ of: PullsOf; read: PullsRead } | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const reads = readsPulls(channel);

  useEffect(() => {
    if (!channel || !reads || !live) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let soon = 0;
    const ask = async () => {
      let again = PULLS_EVERY_MS;
      try {
        const got = await readPulls(channel, of);
        if (stopped) return;
        setHeld({ of, read: got });
        setProblem(null);
        if (got.reading && soon < SOON_TIMES) {
          soon += 1;
          again = PULLS_SOON_MS;
        } else {
          soon = 0;
        }
      } catch (error) {
        if (stopped) return;
        // The reach says what became of a channel; what was read stays.
        if (!(error instanceof ChannelClosed)) setProblem(messageOf(error));
      }
      timer = setTimeout(() => void ask(), again);
    };
    void ask();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [channel, reads, of, asked, live]);

  return reads
    ? { read: held !== null && held.of === of ? held.read : null, problem }
    : { read: null, problem: null };
}

/**
 * One pull request with each check and reviewer by name, read when `which` names another and
 * whenever `again` changes. `null` until the computer has answered, and once it knows of none.
 */
export function usePullDetail(
  channel: Channel | null,
  which: PullRef | null,
  again: unknown,
): PullDetail | null {
  const [held, setHeld] = useState<{ key: string; detail: PullDetail } | null>(null);
  const project = which?.project;
  const number = which?.number;
  const followed = which?.followed;
  const key =
    project === undefined || number === undefined ? null : keyOf({ project, number, followed });
  useEffect(() => {
    if (!channel || project === undefined || number === undefined || key === null) return;
    let stale = false;
    channel.pull({ project, number, ...(followed ? { followed } : {}) }).then(
      (detail) => {
        if (!stale) setHeld({ key, detail });
      },
      (error: unknown) => {
        // Gone from what the computer read: the card says what the list said.
        if (!stale && error instanceof ClientError && error.code === "not_found") setHeld(null);
      },
    );
    return () => {
      stale = true;
    };
  }, [channel, project, number, followed, key, again]);
  return held !== null && held.key === key ? held.detail : null;
}

function toneColors(colors: Colors, tone: PullTone): { ink: string; tint: string } {
  switch (tone) {
    case "open":
      return { ink: colors.online, tint: colors.onlineTint };
    case "waiting":
      return { ink: colors.accentText, tint: colors.accentTint };
    case "merged":
      return { ink: colors.merged, tint: colors.mergedTint };
    case "failing":
    case "closed":
      return { ink: colors.danger, tint: colors.dangerTint };
  }
}

/**
 * A pull request at the end of a row, coloured as on the desktop. With `more`, the workspace has
 * that many others. Pressing it opens the pull request's card.
 */
export function PullBadge({
  pull,
  more = 0,
  words,
  onPress,
}: {
  pull: PullSummary;
  more?: number;
  /** What it says, when not the row's own words: a list names every one by its number. */
  words?: string;
  onPress?: (() => void) | undefined;
}) {
  const colors = useColors();
  const tone = toneOf(pull);
  const { ink, tint } = toneColors(colors, tone);
  const says = more > 0 ? `${toneWords(pull)}, and ${more} more` : toneWords(pull);
  const inside = (
    <View style={[styles.badge, { backgroundColor: tint }]}>
      <Text
        style={[styles.badgeText, { color: ink }, tone === "closed" && !words && styles.struck]}
      >
        {words ?? badgeWords(pull, more)}
      </Text>
    </View>
  );
  if (!onPress) return <View accessibilityLabel={says}>{inside}</View>;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Show ${says}`}
      hitSlop={BADGE_SLOP}
      onPress={onPress}
      style={({ pressed }) => ({ opacity: pressed ? 0.6 : 1 })}
    >
      {inside}
    </Pressable>
  );
}

/** A badge is small: this makes it as easy to press as a button. */
const BADGE_SLOP = { top: 10, bottom: 10, left: 12, right: 8 };

/**
 * A mark before a line. For checks, a tick or a cross once they have finished; otherwise, and for
 * a line about reviews, a dot in the line's colour.
 */
function Mark({ kind, dot }: { kind: "good" | "bad" | "waiting" | "none"; dot?: boolean }) {
  const colors = useColors();
  if (!dot && kind === "good") return <Icon name="check" size={16} color={colors.online} />;
  if (!dot && kind === "bad") return <Icon name="cross" size={16} color={colors.danger} />;
  const color =
    kind === "good"
      ? colors.online
      : kind === "bad"
        ? colors.danger
        : kind === "waiting"
          ? colors.accent
          : colors.faint;
  return (
    <View style={styles.markRoom}>
      <View style={[styles.markDot, { backgroundColor: color }]} />
    </View>
  );
}

const CHECK_WORDS = { passed: "passed", failed: "failed", running: "running", other: "" } as const;

/**
 * One pull request: its number and state, title, branches, lines changed, checks, reviews and
 * whether it merges, and the way to its page. `detail` adds each check and reviewer by name; a
 * card without it says the same in counts.
 */
export function PullCard({
  pull,
  detail,
  now,
}: {
  pull: PullSummary;
  detail: PullDetail | null;
  now: number;
}) {
  const colors = useColors();
  const verdict = checksVerdict(pull.checks);
  const checks = detail?.checks ?? [];
  // Open when something failed: that is what the card is opened for.
  const [shown, setShown] = useState<boolean | null>(null);
  const open = shown ?? verdict === "failed";
  // The pill says the state in words, in the colour the badge has: an open one that failed is red.
  const pill = toneColors(colors, toneOf(pull));
  const changed = changedWords(pull, now);
  const review = reviewLine(pull, detail);
  const url = pull.url;

  return (
    <View
      accessibilityLabel={`Pull request ${refOf(pull)}`}
      style={[styles.card, { backgroundColor: colors.surface, borderColor: colors.line }]}
    >
      <View style={styles.head}>
        <Text style={[styles.number, { color: colors.ink }]}>{refOf(pull)}</Text>
        <View style={[styles.badge, { backgroundColor: pill.tint }]}>
          <Text style={[styles.badgeText, { color: pill.ink }]}>{stateWords(pull)}</Text>
        </View>
        <View style={styles.fill} />
        {changed && <Text style={[styles.small, { color: colors.faint }]}>{changed}</Text>}
      </View>
      <Text style={[styles.title, { color: colors.ink }]}>{pull.title}</Text>
      <View style={styles.facts}>
        {pull.repo && <Text style={[styles.small, { color: colors.muted }]}>{pull.repo}</Text>}
        <Text style={[styles.small, { color: colors.muted }]}>
          {pull.base ? `${pull.branch} → ${pull.base}` : pull.branch}
        </Text>
        {pull.additions !== undefined && pull.deletions !== undefined && (
          <Text
            accessibilityLabel={`${pull.additions} lines added, ${pull.deletions} removed`}
            style={styles.small}
          >
            <Text style={{ color: colors.online }}>+{pull.additions}</Text>
            {"  "}
            <Text style={{ color: colors.danger }}>−{pull.deletions}</Text>
          </Text>
        )}
      </View>

      <View style={styles.line}>
        <Mark
          kind={
            verdict === "passed"
              ? "good"
              : verdict === "failed"
                ? "bad"
                : verdict === "running"
                  ? "waiting"
                  : "none"
          }
        />
        <Text style={[styles.lineText, { color: verdict === "none" ? colors.muted : colors.ink }]}>
          {checksWords(pull.checks)}
        </Text>
        {checks.length > 0 && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={open ? "Hide the checks" : "Show the checks"}
            accessibilityState={{ expanded: open }}
            hitSlop={BADGE_SLOP}
            onPress={() => setShown(!open)}
            style={styles.toggle}
          >
            <Text style={[styles.lineText, styles.toggleText, { color: colors.accentText }]}>
              {open ? "Hide" : "Show"}
            </Text>
            <Text style={[styles.small, { color: colors.accentText }]}>{open ? "▾" : "▸"}</Text>
          </Pressable>
        )}
      </View>
      {open && checks.length > 0 && (
        <View
          accessibilityLabel="Checks"
          style={[styles.checks, { backgroundColor: colors.raised }]}
        >
          {checks.map((check, at) => (
            <View key={at} style={styles.check}>
              <View
                style={[
                  styles.markDot,
                  {
                    backgroundColor:
                      check.state === "failed"
                        ? colors.danger
                        : check.state === "running"
                          ? colors.accent
                          : check.state === "passed"
                            ? colors.online
                            : colors.faint,
                  },
                ]}
              />
              <Text numberOfLines={2} style={[styles.checkName, { color: colors.ink }]}>
                {check.name}
              </Text>
              <Text
                style={[
                  styles.small,
                  { color: check.state === "failed" ? colors.danger : colors.faint },
                ]}
              >
                {CHECK_WORDS[check.state]}
              </Text>
            </View>
          ))}
          {detail !== null && detail.moreChecks > 0 && (
            <Text style={[styles.small, { color: colors.faint }]}>
              and {detail.moreChecks} more
            </Text>
          )}
        </View>
      )}

      {review && (
        <View style={styles.line}>
          <Mark kind={review.tone} dot />
          <Text
            style={[styles.lineText, { color: review.tone === "none" ? colors.muted : colors.ink }]}
          >
            {review.words}
          </Text>
        </View>
      )}
      {pull.conflicts && (
        <View style={styles.line}>
          <Icon name="warning" size={16} color={colors.danger} />
          <Text style={[styles.lineText, { color: colors.ink }]}>
            Conflicts with {pull.base ?? "its base branch"}
          </Text>
        </View>
      )}
      {url !== undefined && (
        <Button title={openWords(url)} onPress={() => void Linking.openURL(url).catch(() => {})} />
      )}
    </View>
  );
}

/**
 * A pull request as a line of a list: where it is, its number, how long since it changed, its
 * title, who opened it, its checks and lines changed, and the workspaces it came out of.
 */
export function PullRow({
  pull,
  project,
  workspaces,
  now,
  onPress,
}: {
  pull: PullSummary;
  /** Its project's name, when the list is of more than one. */
  project?: string | undefined;
  /** The names of the workspaces it came out of. */
  workspaces: string[];
  now: number;
  onPress: () => void;
}) {
  const colors = useColors();
  const verdict = checksVerdict(pull.checks);
  const short = checksShort(pull.checks);
  const where = [project, pull.repo].filter(Boolean).join(" · ");
  const when = pull.updatedAt === undefined ? null : ago(pull.updatedAt, now);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={[
        pull.title,
        toneWords(pull),
        where,
        checksWords(pull.checks),
        pull.conflicts ? "conflicts" : "",
        workspaces.length > 0 ? `in ${workspaces.join(", ")}` : "",
      ]
        .filter(Boolean)
        .join(", ")}
      onPress={onPress}
      style={({ pressed }) => [styles.row, { opacity: pressed ? 0.7 : 1 }]}
    >
      <View style={styles.head}>
        {where !== "" && (
          <Text numberOfLines={1} style={[styles.small, styles.shrink, { color: colors.muted }]}>
            {where}
          </Text>
        )}
        <PullBadge pull={pull} words={`#${pull.number}`} />
        <View style={styles.fill} />
        {when && <Text style={[styles.small, { color: colors.faint }]}>{when}</Text>}
      </View>
      <Text style={[styles.rowTitle, { color: colors.ink }]}>{pull.title}</Text>
      <View style={styles.rowFacts}>
        {pull.author && <Text style={[styles.small, { color: colors.muted }]}>{pull.author}</Text>}
        {short === null ? (
          <Text style={[styles.small, { color: colors.faint }]}>no checks</Text>
        ) : (
          <View style={styles.count}>
            <Mark kind={verdict === "passed" ? "good" : verdict === "failed" ? "bad" : "waiting"} />
            <Text
              style={[
                styles.small,
                {
                  color:
                    verdict === "failed"
                      ? colors.danger
                      : verdict === "running"
                        ? colors.accentText
                        : colors.online,
                },
              ]}
            >
              {short}
            </Text>
          </View>
        )}
        {pull.additions !== undefined && pull.deletions !== undefined && (
          <Text style={styles.small}>
            <Text style={{ color: colors.online }}>+{pull.additions}</Text>{" "}
            <Text style={{ color: colors.danger }}>−{pull.deletions}</Text>
          </Text>
        )}
        {pull.conflicts && (
          <View style={styles.count}>
            <Icon name="warning" size={14} color={colors.danger} />
            <Text style={[styles.small, { color: colors.danger }]}>conflicts</Text>
          </View>
        )}
      </View>
      {workspaces.length > 0 && (
        <Text style={[styles.rowIn, { color: colors.accentText }]}>In {workspaces.join(", ")}</Text>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  shrink: { flexShrink: 1 },
  badge: { borderRadius: 6, paddingHorizontal: 8, paddingVertical: 3 },
  badgeText: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 16 },
  struck: { textDecorationLine: "line-through" },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 14, padding: 14, gap: 12 },
  head: { flexDirection: "row", alignItems: "center", gap: 8 },
  number: { fontFamily: FONTS.monoMedium, fontSize: 14, lineHeight: 18 },
  small: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 16 },
  title: { fontFamily: FONTS.semibold, fontSize: 16, lineHeight: 21 },
  facts: { gap: 4 },
  line: { flexDirection: "row", alignItems: "center", gap: 8, minHeight: 20 },
  lineText: { flex: 1, fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  toggle: { flexDirection: "row", alignItems: "center", gap: 4 },
  toggleText: { flex: 0, fontFamily: FONTS.medium },
  markRoom: { width: 16, height: 16, alignItems: "center", justifyContent: "center" },
  markDot: { width: 8, height: 8, borderRadius: 4 },
  checks: { borderRadius: 10, paddingHorizontal: 12, paddingVertical: 10, gap: 10 },
  check: { flexDirection: "row", alignItems: "center", gap: 10 },
  checkName: { flex: 1, fontFamily: FONTS.mono, fontSize: 13, lineHeight: 18 },
  row: { paddingHorizontal: 16, paddingVertical: 14, gap: 6 },
  rowTitle: { fontFamily: FONTS.medium, fontSize: 15, lineHeight: 21 },
  rowFacts: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    columnGap: 12,
    rowGap: 4,
  },
  count: { flexDirection: "row", alignItems: "center", gap: 4 },
  rowIn: { fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
});
