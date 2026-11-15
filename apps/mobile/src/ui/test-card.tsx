// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A smoke test run waiting for its person, in Chat: the computer's approval card at a phone's
 * width. Every step in words, the names of the test environment it can read, what changed since
 * the version approved before, and **Run** or **Don't run**. Seeing it is the gate a file from a
 * repository goes through, so nothing of it is folded but the unchanged lines of a changed file,
 * and a card the computer could not send whole offers no Run.
 */

import type { WaitingTest } from "@yardsort/remote-client";
import { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { stamp } from "../lib/format";
import { changed, diffLines, PHASE_WORDS, PHASES } from "../lib/tests";
import { Button, FONTS, Problem, useColors } from "./parts";

export function TestCard({
  test,
  behind,
  askedBy,
  problem,
  decide,
}: {
  test: WaitingTest;
  /** How many more runs wait behind this one. */
  behind: number;
  /** Who asked, in words. */
  askedBy: string;
  /** Why the last answer did not go through. */
  problem: string | null;
  /** Answer it. Without it, the computer cannot be reached and the card only shows. */
  decide: ((decision: "run" | "refuse") => Promise<void>) | undefined;
}) {
  const colors = useColors();
  const [busy, setBusy] = useState<"run" | "refuse" | null>(null);
  // Returns nothing: a press starts the answer, and the card goes when it has gone.
  const press = (decision: "run" | "refuse") => () => {
    if (!decide) return;
    setBusy(decision);
    void decide(decision).finally(() => setBusy(null));
  };
  return (
    <View
      accessibilityLabel={`Run ${test.name}?`}
      style={[styles.card, { backgroundColor: colors.accentTint, borderColor: colors.accent }]}
    >
      <Text style={[styles.head, { color: colors.accentText }]}>
        NEEDS YOU · {stamp(test.askedAt)}
      </Text>
      <Text style={[styles.title, { color: colors.ink }]}>Run “{test.name}”?</Text>
      <Text style={[styles.body, { color: colors.muted }]}>
        Asked by {askedBy}
        {test.inputs.length > 0 &&
          `, with ${test.inputs.map((input) => `${input.name} = ${input.value}`).join(", ")}`}
        . A file from the repository runs only once you have seen it; this is what it does.
      </Text>

      {PHASES.map((phase) => {
        const steps = test.steps.filter((step) => step.phase === phase);
        if (steps.length === 0) return null;
        return (
          <View key={phase} style={styles.phase} accessibilityLabel={`${PHASE_WORDS[phase]} steps`}>
            <Text style={[styles.phaseName, { color: colors.faint }]}>{PHASE_WORDS[phase]}</Text>
            {steps.map((step) => (
              <View key={step.id} style={[styles.step, { backgroundColor: colors.raised }]}>
                <Text style={[styles.stepId, { color: colors.faint }]}>{step.id}</Text>
                <Text selectable style={[styles.stepWords, { color: colors.ink }]}>
                  {step.words}
                </Text>
              </View>
            ))}
          </View>
        );
      })}

      <EnvUses test={test} />
      {test.previous !== undefined && test.definition !== undefined && (
        <Changed before={test.previous} after={test.definition} />
      )}
      {test.approvedBefore && (
        <Text style={[styles.body, { color: colors.muted }]}>
          This exact file was approved before; the project asks to see every run.
        </Text>
      )}
      {!test.whole && (
        <Problem attention>
          {test.whyNot ?? "This test cannot be shown whole here. Run it at the computer."}
        </Problem>
      )}
      {problem && <Problem>{problem}</Problem>}

      <View style={styles.buttons}>
        <View style={styles.fill}>
          <Button
            title="Don’t run"
            busy={busy === "refuse"}
            disabled={busy !== null || !decide}
            onPress={press("refuse")}
          />
        </View>
        {test.whole && (
          <View style={styles.fill}>
            <Button
              title="Run"
              kind="primary"
              busy={busy === "run"}
              disabled={busy !== null || !decide}
              onPress={press("run")}
            />
          </View>
        )}
      </View>
      <Text style={[styles.foot, { color: colors.faint }]}>
        {test.whole &&
          "Run approves this version of the file for the project. Don’t run records nothing against it."}
        {test.whole && behind > 0 && " "}
        {behind > 0 && `${behind} more ${behind === 1 ? "run waits" : "runs wait"} behind it.`}
      </Text>
    </View>
  );
}

/**
 * What of the project's test environment the file can read. A step that runs a program is handed
 * all of it, not only the names the file templates, so then the card names all of it.
 */
function EnvUses({ test }: { test: WaitingTest }) {
  const colors = useColors();
  const unset = test.env.filter((use) => !use.set).map((use) => use.name);
  if (test.runsAProgram && test.environment.length > 0) {
    return (
      <Text style={[styles.body, { color: colors.muted }]}>
        A step runs a program, so it can read the project’s whole test environment:{" "}
        <Text style={styles.names}>{test.environment.join(", ")}</Text>
        {unset.length > 0 &&
          `. ${unset.join(", ")} ${unset.length === 1 ? "is named but" : "are named but"} not set`}
        .
      </Text>
    );
  }
  if (test.env.length === 0) return null;
  return (
    <Text style={[styles.body, { color: colors.muted }]}>
      Uses{" "}
      <Text style={styles.names}>
        {test.env.map((use) => (use.set ? use.name : `${use.name} (not set)`)).join(", ")}
      </Text>{" "}
      from the project’s test environment
      {unset.length > 0 &&
        `; ${unset.length === 1 ? "one is" : `${unset.length} are`} not set and would be empty`}
      .
    </Text>
  );
}

/** What changed since the version approved before: the changed lines, and those around them. */
function Changed({ before, after }: { before: string; after: string }) {
  const colors = useColors();
  const [open, setOpen] = useState(true);
  const lines = useMemo(() => changed(diffLines(before, after)), [before, after]);
  return (
    <View style={styles.phase}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="What changed since the version you approved"
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen(!open)}
      >
        <Text style={[styles.body, { color: colors.accentText }]}>
          {open ? "▾" : "▸"} What changed since the version you approved
        </Text>
      </Pressable>
      {open && (
        <View
          accessibilityLabel="What changed"
          style={[styles.diff, { backgroundColor: colors.raised }]}
        >
          {lines.map((line, at) =>
            line.kind === "gap" ? (
              <Text key={at} style={[styles.diffLine, { color: colors.faint }]}>
                {"  "}⋯ {line.hidden} unchanged {line.hidden === 1 ? "line" : "lines"}
              </Text>
            ) : (
              <Text
                key={at}
                selectable
                style={[
                  styles.diffLine,
                  {
                    color:
                      line.kind === "added"
                        ? colors.online
                        : line.kind === "removed"
                          ? colors.danger
                          : colors.faint,
                  },
                  line.kind === "removed" && styles.removed,
                ]}
              >
                {line.kind === "added" ? "+ " : line.kind === "removed" ? "- " : "  "}
                {line.text}
              </Text>
            ),
          )}
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  card: { borderWidth: 1, borderRadius: 12, padding: 14, gap: 12 },
  head: { fontFamily: FONTS.mono, fontSize: 11, letterSpacing: 0.4 },
  title: { fontFamily: FONTS.semibold, fontSize: 16, lineHeight: 22 },
  body: { fontFamily: FONTS.regular, fontSize: 14, lineHeight: 20 },
  names: { fontFamily: FONTS.mono, fontSize: 13 },
  phase: { gap: 6 },
  phaseName: {
    fontFamily: FONTS.monoMedium,
    fontSize: 11,
    lineHeight: 14,
    letterSpacing: 0.7,
    textTransform: "uppercase",
  },
  step: { borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10, gap: 4 },
  stepId: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 16 },
  stepWords: { fontFamily: FONTS.regular, fontSize: 14, lineHeight: 20 },
  diff: { borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10 },
  diffLine: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 18 },
  removed: { textDecorationLine: "line-through" },
  buttons: { flexDirection: "row", alignItems: "flex-end", gap: 8 },
  foot: { fontFamily: FONTS.regular, fontSize: 12, lineHeight: 17 },
});
