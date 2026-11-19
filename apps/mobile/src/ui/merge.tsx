// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Merge… on a pull request's card, and the sheet that asks first (48, Figma M1 to M5). The sheet
 * names the commit the computer will merge at, the one shown now; the computer refuses any other.
 */

import type { Channel, MergeMethod, PublishState, PullSummary } from "@yardsort/remote-client";
import { MERGE_METHODS } from "@yardsort/remote-client";
import { useEffect, useRef, useState } from "react";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import {
  checksWarning,
  METHOD_NAMES,
  mergeButtonWords,
  mergedWords,
  mergeFailure,
  type MergeOffer,
  mergeOffer,
  mergesPulls,
  mergeWorkspace,
  methodWords,
  shortOid,
} from "../lib/merge";
import { Button, FONTS, useColors } from "./parts";

/**
 * Merging a pull request from its card: asks the computer where the workspace that opened it
 * stands, which names the commit, and offers Merge… when it can be. `again` reads it again with
 * the card; `onDone` is called once a press was answered, either way, for the card to be read.
 */
export function PullMerge({
  channel,
  pull,
  again,
  onDone,
}: {
  channel: Channel | null;
  pull: PullSummary;
  again: unknown;
  onDone: () => void;
}) {
  const through = mergeWorkspace(pull);
  const can = mergesPulls(channel);
  // What the computer last said, and of which workspace: one said of another is not this one's.
  const [held, setHeld] = useState<{ of: string; state: PublishState | null } | null>(null);
  const publish = can && through !== null && held?.of === through ? held.state : null;
  // Asked of the forge again with each reading of the card, so a push since shows before a press.
  useEffect(() => {
    if (!can || !channel || !through) return;
    let stale = false;
    void channel.publishState({ workspace: through, forge: true }).then(
      (state) => {
        if (!stale) setHeld({ of: through, state });
      },
      () => {
        if (!stale) setHeld({ of: through, state: null });
      },
    );
    return () => {
      stale = true;
    };
  }, [can, channel, through, again]);
  const offer: MergeOffer = can ? mergeOffer(pull, publish) : { kind: "none" };
  // What the sheet shows, fixed when Merge… is pressed: a read that lands while it is open does
  // not change the commit the person is reading. The computer refuses it if the head moved.
  const [asking, setAsking] = useState<Extract<MergeOffer, { kind: "ready" }> | null>(null);
  const [merging, setMerging] = useState(false);
  const sent = useRef(false);
  const [said, setSaid] = useState<{ words: string; bad: boolean } | null>(null);

  const merge = async (method: MergeMethod) => {
    const shown = asking;
    setAsking(null);
    // One press, one merge: a second tap before the first is answered sends nothing.
    if (!channel || !shown || sent.current) return;
    sent.current = true;
    setMerging(true);
    setSaid(null);
    try {
      await channel.mergePullRequest({
        workspace: shown.workspace,
        number: pull.number,
        headOid: shown.head,
        method,
      });
      setSaid({ words: mergedWords(method, shown.base, shown.head, pull.branch), bad: false });
    } catch (error) {
      const failure = mergeFailure(error);
      setSaid({ words: failure.words, bad: failure.kind !== "lost" });
    } finally {
      sent.current = false;
      setMerging(false);
      // Read again either way. The merge itself is never sent again by itself.
      onDone();
    }
  };

  if (offer.kind === "none" && said === null && !asking) return null;
  return (
    <>
      <MergeArea
        offer={offer}
        said={said?.words ?? null}
        saidBad={said?.bad ?? false}
        busy={merging}
        onMerge={() => {
          if (offer.kind === "ready" && !merging) setAsking(offer);
        }}
      />
      {asking && channel && (
        <MergeSheet
          visible
          pull={pull}
          head={asking.head}
          base={asking.base}
          computer={channel.host.name}
          onCancel={() => setAsking(null)}
          onConfirm={(method) => void merge(method)}
        />
      )}
    </>
  );
}

/** What the card shows of merging: the button, a reason, a result, or nothing. */
export function MergeArea({
  offer,
  said,
  saidBad,
  onMerge,
  busy,
}: {
  offer: MergeOffer;
  /** What came of the last press: merged, refused, or lost. */
  said: string | null;
  saidBad: boolean;
  onMerge: () => void;
  busy: boolean;
}) {
  const colors = useColors();
  const line = (words: string, color: string) => (
    <Text style={[styles.line, { color }]}>{words}</Text>
  );
  return (
    <View style={styles.area}>
      {offer.kind === "ready" && (
        <Button title="Merge…" kind="primary" busy={busy} onPress={onMerge} />
      )}
      {offer.kind === "blocked" && (
        <>
          <Button title="Merge…" disabled onPress={() => {}} />
          {line(offer.why, colors.muted)}
        </>
      )}
      {offer.kind === "note" && line(offer.why, colors.muted)}
      {said !== null && line(said, saidBad ? colors.danger : colors.muted)}
    </View>
  );
}

/** The sheet: what it merges, into which branch, at which commit, and how. */
export function MergeSheet({
  visible,
  pull,
  head,
  base,
  computer,
  onCancel,
  onConfirm,
}: {
  visible: boolean;
  pull: PullSummary;
  head: string;
  base: string | undefined;
  computer: string;
  onCancel: () => void;
  onConfirm: (method: MergeMethod) => void;
}) {
  const colors = useColors();
  const insets = useSafeAreaInsets();
  // Squash first, as the window does (48: decided 2026-11-18).
  const [method, setMethod] = useState<MergeMethod>("squash");
  const warning = checksWarning(pull.checks);
  const fact = (key: string, value: string) => (
    <View style={styles.fact}>
      <Text style={[styles.factKey, { color: colors.faint }]}>{key}</Text>
      <Text style={[styles.factValue, { color: colors.ink }]} numberOfLines={1}>
        {value}
      </Text>
    </View>
  );
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onCancel}>
      <Pressable accessibilityLabel="Cancel the merge" style={styles.dim} onPress={onCancel} />
      <View
        accessibilityViewIsModal
        style={[
          styles.sheet,
          {
            backgroundColor: colors.raised,
            borderColor: colors.line,
            paddingBottom: Math.max(insets.bottom, 16) + 8,
          },
        ]}
      >
        <View style={[styles.grab, { backgroundColor: colors.faint }]} />
        <Text accessibilityRole="header" style={[styles.title, { color: colors.ink }]}>
          Merge #{pull.number}?
        </Text>
        <Text style={[styles.line, { color: colors.muted }]}>{pull.title}</Text>
        <View style={[styles.facts, { backgroundColor: colors.surface, borderColor: colors.line }]}>
          {fact("into", base ?? "its base branch")}
          {fact("at", `${shortOid(head)} · the commit shown now`)}
          {fact("on", `${computer}, with its gh`)}
        </View>
        <Text style={[styles.label, { color: colors.faint }]}>METHOD</Text>
        <View
          accessibilityRole="radiogroup"
          style={[styles.methods, { backgroundColor: colors.surface, borderColor: colors.line }]}
        >
          {MERGE_METHODS.map((each) => {
            const on = each === method;
            return (
              <Pressable
                key={each}
                accessibilityRole="radio"
                accessibilityLabel={METHOD_NAMES[each]}
                accessibilityState={{ checked: on }}
                onPress={() => setMethod(each)}
                style={[styles.method, on && { backgroundColor: colors.accent }]}
              >
                <Text
                  style={[
                    styles.methodText,
                    { color: on ? colors.onAccent : colors.muted },
                    on && { fontFamily: FONTS.semibold },
                  ]}
                >
                  {METHOD_NAMES[each]}
                </Text>
              </Pressable>
            );
          })}
        </View>
        <Text style={[styles.line, { color: colors.muted }]}>{methodWords(method, base)}</Text>
        {warning !== null && (
          <View style={[styles.warning, { backgroundColor: colors.accentTint }]}>
            <Text style={[styles.line, { color: colors.accentText }]}>{warning}</Text>
          </View>
        )}
        <Button title={mergeButtonWords(method)} kind="primary" onPress={() => onConfirm(method)} />
        <Button title="Cancel" kind="quiet" onPress={onCancel} />
        <Text style={[styles.small, { color: colors.faint }]}>The branch is kept.</Text>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  area: { gap: 8 },
  line: { fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  small: { fontFamily: FONTS.regular, fontSize: 12, lineHeight: 16 },
  dim: { flex: 1, backgroundColor: "rgba(0,0,0,0.55)" },
  sheet: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    borderWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingHorizontal: 20,
    gap: 14,
  },
  grab: { alignSelf: "center", width: 36, height: 5, borderRadius: 3 },
  title: { fontFamily: FONTS.semibold, fontSize: 18, lineHeight: 24 },
  facts: { borderRadius: 12, borderWidth: StyleSheet.hairlineWidth, padding: 12, gap: 6 },
  fact: { flexDirection: "row", gap: 10 },
  factKey: { fontFamily: FONTS.mono, fontSize: 13, width: 40 },
  factValue: { fontFamily: FONTS.mono, fontSize: 13, flex: 1 },
  label: { fontFamily: FONTS.monoMedium, fontSize: 11, letterSpacing: 0.5 },
  methods: {
    flexDirection: "row",
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 4,
    gap: 4,
  },
  method: { flex: 1, alignItems: "center", paddingVertical: 8, borderRadius: 7 },
  methodText: { fontFamily: FONTS.regular, fontSize: 14 },
  warning: { borderRadius: 10, paddingVertical: 10, paddingHorizontal: 12 },
});
