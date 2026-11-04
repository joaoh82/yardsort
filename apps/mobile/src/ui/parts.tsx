// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The few pieces every screen is built from, as the design has them (docs/design/33). Native
 * components only: text scales with the system's size, and every control says what it is to a
 * screen reader.
 */

import { Children, type ReactNode } from "react";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  type TextInputProps,
  useColorScheme,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import Svg, { Path } from "react-native-svg";

// The desktop app's own, with faint text and amber words made stronger for a phone's small text.
const LIGHT = {
  background: "#f4f5f7",
  surface: "#ffffff",
  raised: "#eceef1",
  line: "#d9dde3",
  lineStrong: "#b9c0ca",
  ink: "#14171c",
  muted: "#4b5563",
  faint: "#636c7a",
  accent: "#f5b83d",
  onAccent: "#14171c",
  accentText: "#8a5c00",
  accentTint: "#fbf0d6",
  online: "#1f7a45",
  danger: "#b3261e",
  dangerTint: "#fbe9e7",
  claude: "#c2603f",
  codex: "#2f74c0",
};

const DARK: typeof LIGHT = {
  background: "#0f1115",
  surface: "#14171c",
  raised: "#1b1f26",
  line: "#262b34",
  lineStrong: "#3a414d",
  ink: "#e8eaed",
  muted: "#9aa3af",
  faint: "#858d9a",
  accent: "#f5b83d",
  onAccent: "#14171c",
  accentText: "#f5b83d",
  accentTint: "#2a2413",
  online: "#4fbf7a",
  danger: "#f28b82",
  dangerTint: "#2b1716",
  claude: "#cf6a48",
  codex: "#4a8ad0",
};

export type Colors = typeof LIGHT;

export function useColors(): Colors {
  return useColorScheme() === "dark" ? DARK : LIGHT;
}

/** Embedded by the build (app.config.ts). A weight is its own family, so nothing sets `fontWeight`. */
export const FONTS = {
  regular: "Inter-Regular",
  medium: "Inter-Medium",
  semibold: "Inter-SemiBold",
  mono: "JetBrainsMono-Regular",
  monoMedium: "JetBrainsMono-Medium",
};

const ICONS = {
  chevron: "M9 6l6 6-6 6",
  computer: "M4 5h16v11H4zM9 20h6M12 16v4",
  phone: "M8 3h8a1 1 0 011 1v16a1 1 0 01-1 1H8a1 1 0 01-1-1V4a1 1 0 011-1zM11 18h2",
  note: "M7 4h7l4 4v12H7zM14 4v4h4M10 13h5M10 16h5",
  server: "M5 5h14v6H5zM5 13h14v6H5zM8 8h.01M8 16h.01",
  terminal: "M5 7l5 5-5 5M12 18h7",
};

export function Icon({ name, color }: { name: keyof typeof ICONS; color?: string }) {
  const colors = useColors();
  return (
    <Svg width={24} height={24} viewBox="0 0 24 24" fill="none">
      <Path
        d={ICONS[name]}
        stroke={color ?? colors.muted}
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </Svg>
  );
}

export function Screen({
  children,
  refreshing,
  onRefresh,
}: {
  children: ReactNode;
  /** Pull to refresh, when given. */
  refreshing?: boolean;
  onRefresh?: () => void;
}) {
  const colors = useColors();
  return (
    <SafeAreaView
      edges={["bottom", "left", "right"]}
      style={[styles.fill, { backgroundColor: colors.background }]}
    >
      {/* From iOS 26 the header is glass over the screen, so the content has to start below it. */}
      <ScrollView
        testID="screen"
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={styles.screen}
        keyboardShouldPersistTaps="handled"
        refreshControl={
          onRefresh ? (
            <RefreshControl refreshing={refreshing ?? false} onRefresh={onRefresh} />
          ) : undefined
        }
      >
        {children}
      </ScrollView>
    </SafeAreaView>
  );
}

export function Title({ children }: { children: ReactNode }) {
  const colors = useColors();
  return (
    <Text accessibilityRole="header" style={[styles.title, { color: colors.ink }]}>
      {children}
    </Text>
  );
}

export function Body({ children, muted }: { children: ReactNode; muted?: boolean }) {
  const colors = useColors();
  return (
    <Text style={[styles.body, { color: muted ? colors.muted : colors.ink }]}>{children}</Text>
  );
}

/** What a machine says rather than a person: an address, a branch. */
export function Mono({ children }: { children: ReactNode }) {
  const colors = useColors();
  return <Text style={[styles.mono, { color: colors.muted }]}>{children}</Text>;
}

export function Card({
  children,
  label,
  onPress,
}: {
  children: ReactNode;
  label?: string;
  /** What pressing it does, when it does anything. */
  onPress?: () => void;
}) {
  const colors = useColors();
  if (onPress) {
    return (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={label}
        onPress={onPress}
        style={({ pressed }) => [
          styles.card,
          {
            backgroundColor: colors.surface,
            borderColor: colors.line,
            opacity: pressed ? 0.7 : 1,
          },
        ]}
      >
        {children}
      </Pressable>
    );
  }
  return (
    <View
      accessibilityLabel={label}
      style={[styles.card, { backgroundColor: colors.surface, borderColor: colors.line }]}
    >
      {children}
    </View>
  );
}

export function Field({
  label,
  hint,
  ...input
}: { label: string; hint?: string } & TextInputProps) {
  const colors = useColors();
  return (
    <View style={styles.field}>
      <Text style={[styles.caption, { color: colors.muted }]}>{label}</Text>
      <TextInput
        accessibilityLabel={label}
        {...(hint ? { accessibilityHint: hint } : {})}
        placeholderTextColor={colors.faint}
        style={[
          styles.input,
          { color: colors.ink, borderColor: colors.lineStrong, backgroundColor: colors.surface },
        ]}
        {...input}
      />
      {hint && <Text style={[styles.caption, { color: colors.muted }]}>{hint}</Text>}
    </View>
  );
}

export function Button({
  title,
  onPress,
  kind = "plain",
  busy = false,
  disabled = false,
  compact = false,
}: {
  title: string;
  onPress: () => void;
  kind?: "primary" | "plain" | "quiet" | "danger";
  busy?: boolean;
  disabled?: boolean;
  /** As tall as a key, to sit beside a field. */
  compact?: boolean;
}) {
  const colors = useColors();
  const primary = kind === "primary";
  const off = disabled || busy;
  const words =
    kind === "primary"
      ? colors.onAccent
      : kind === "danger"
        ? colors.danger
        : kind === "quiet"
          ? colors.muted
          : colors.ink;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={title}
      accessibilityState={{ disabled: off, busy }}
      disabled={off}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        compact && styles.compact,
        {
          backgroundColor: primary ? colors.accent : kind === "plain" ? colors.raised : undefined,
          borderColor: kind === "danger" ? colors.danger : colors.line,
          borderWidth:
            primary || kind === "quiet" ? 0 : kind === "danger" ? 1 : StyleSheet.hairlineWidth,
          opacity: off ? 0.5 : pressed ? 0.7 : 1,
        },
      ]}
    >
      {busy ? (
        <ActivityIndicator color={words} />
      ) : (
        <Text style={[styles.buttonText, { color: words }]}>{title}</Text>
      )}
    </Pressable>
  );
}

/** A small heading inside a section. */
export function Label({ children, note }: { children: ReactNode; note?: ReactNode }) {
  const colors = useColors();
  return (
    <View style={styles.labelLine}>
      <Text accessibilityRole="header" style={[styles.label, { color: colors.faint }]}>
        {children}
      </Text>
      {note !== undefined && <Text style={[styles.label, { color: colors.faint }]}>{note}</Text>}
    </View>
  );
}

/** A part of a screen, under its heading. */
export function Section({
  title,
  note,
  children,
}: {
  title?: ReactNode;
  /** Beside the heading: how many, or what the section is doing. */
  note?: ReactNode;
  children: ReactNode;
}) {
  return (
    <View style={styles.section}>
      {title !== undefined && <Label note={note}>{title}</Label>}
      {children}
    </View>
  );
}

/** Rows that belong together, with a line between each. */
export function Group({ children }: { children: ReactNode }) {
  const colors = useColors();
  const rows = Children.toArray(children);
  if (rows.length === 0) return null;
  return (
    <View style={[styles.group, { backgroundColor: colors.surface, borderColor: colors.line }]}>
      {rows.map((row, at) => (
        <View key={at}>
          {at > 0 && <View style={[styles.divider, { backgroundColor: colors.line }]} />}
          {row}
        </View>
      ))}
    </View>
  );
}

/** One line of a group: pressable when it goes somewhere, and with a chevron to say so. */
export function Row({
  title,
  about,
  detail,
  monoDetail,
  plainTitle,
  leading,
  tag,
  badge,
  label,
  selected,
  busy,
  disabled,
  onPress,
}: {
  title: string;
  /** A line a machine says, under the title. */
  about?: string;
  detail?: string;
  /** The detail is a branch, a state or an address, not a sentence. */
  monoDetail?: boolean;
  /** The title reads as text in a list, rather than as a name. */
  plainTitle?: boolean;
  leading?: ReactNode;
  /** Words at the end, outlined. */
  tag?: string;
  /** Something at the end that is happening now. */
  badge?: string;
  /** What a screen reader says, when not the title and detail. */
  label?: string;
  selected?: boolean;
  busy?: boolean;
  disabled?: boolean;
  onPress?: () => void;
}) {
  const colors = useColors();
  const says = label ?? (detail ? `${title}, ${detail}` : title);
  const inside = (
    <>
      {leading}
      <View style={styles.rowText}>
        <Text style={[plainTitle ? styles.rowPlain : styles.rowTitle, { color: colors.ink }]}>
          {title}
        </Text>
        {about ? <Text style={[styles.monoSmall, { color: colors.muted }]}>{about}</Text> : null}
        {detail ? (
          <Text style={[monoDetail ? styles.monoSmall : styles.caption, { color: colors.muted }]}>
            {detail}
          </Text>
        ) : null}
      </View>
      {badge ? (
        <View style={[styles.badge, { backgroundColor: colors.accentTint }]}>
          <Text style={[styles.monoSmall, { color: colors.accentText }]}>{badge}</Text>
        </View>
      ) : null}
      {tag ? (
        <View style={[styles.tag, { borderColor: colors.lineStrong }]}>
          <Text style={[styles.monoSmall, { color: colors.accentText }]}>{tag}</Text>
        </View>
      ) : null}
      {onPress ? <Icon name="chevron" color={colors.faint} /> : null}
    </>
  );
  if (!onPress) {
    return (
      <View accessibilityLabel={says} style={styles.row}>
        {inside}
      </View>
    );
  }
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={says}
      {...(selected !== undefined || busy !== undefined
        ? { accessibilityState: { selected: selected ?? false, busy: busy ?? false } }
        : {})}
      disabled={disabled ?? false}
      onPress={onPress}
      style={({ pressed }) => [styles.row, { opacity: pressed ? 0.7 : 1 }]}
    >
      {inside}
    </Pressable>
  );
}

/** A computer's standing at a glance. Filled or hollow as well as its color, for those who can't tell. */
export function Dot({ kind }: { kind: "on" | "off" | "trying" | "bad" }) {
  const colors = useColors();
  const color =
    kind === "on"
      ? colors.online
      : kind === "trying"
        ? colors.accentText
        : kind === "bad"
          ? colors.danger
          : colors.faint;
  return (
    <View
      style={[
        styles.dot,
        kind === "on" ? { backgroundColor: color } : { borderColor: color, borderWidth: 2 },
      ]}
    />
  );
}

/** Something that went wrong, announced to a screen reader as it appears. */
export function Problem({
  children,
  attention,
}: {
  children: ReactNode;
  /** Something to look at before going on, rather than something that failed. */
  attention?: boolean;
}) {
  const colors = useColors();
  return (
    <Text
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      style={[
        styles.body,
        styles.problem,
        attention
          ? { color: colors.accentText, backgroundColor: colors.accentTint }
          : { color: colors.danger, backgroundColor: colors.dangerTint },
      ]}
    >
      {children}
    </Text>
  );
}

export function Waiting({ label }: { label: string }) {
  const colors = useColors();
  return (
    <View style={styles.waiting} accessibilityLabel={label}>
      <ActivityIndicator color={colors.muted} />
      <Text style={[styles.body, { color: colors.muted }]}>{label}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  screen: { padding: 16, gap: 16 },
  title: { fontFamily: FONTS.semibold, fontSize: 22, lineHeight: 28 },
  body: { fontFamily: FONTS.regular, fontSize: 15, lineHeight: 21 },
  caption: { fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
  mono: { fontFamily: FONTS.mono, fontSize: 13, lineHeight: 18 },
  monoSmall: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 16 },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 14, padding: 16, gap: 12 },
  field: { gap: 6 },
  label: {
    fontFamily: FONTS.monoMedium,
    fontSize: 11,
    lineHeight: 14,
    letterSpacing: 0.7,
    textTransform: "uppercase",
  },
  labelLine: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 8,
    paddingHorizontal: 4,
  },
  section: { gap: 8, marginTop: 8 },
  group: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 14, overflow: "hidden" },
  divider: { height: StyleSheet.hairlineWidth },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 60,
    paddingVertical: 12,
    paddingLeft: 16,
    paddingRight: 12,
  },
  rowText: { flex: 1, gap: 4 },
  rowTitle: { fontFamily: FONTS.semibold, fontSize: 16, lineHeight: 21 },
  rowPlain: { fontFamily: FONTS.medium, fontSize: 15, lineHeight: 21 },
  badge: { borderRadius: 6, paddingHorizontal: 8, paddingVertical: 4 },
  tag: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 6,
    paddingHorizontal: 8,
    paddingVertical: 4,
  },
  dot: { width: 10, height: 10, borderRadius: 5, margin: 1 },
  input: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    minHeight: 48,
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontFamily: FONTS.mono,
    fontSize: 13,
  },
  button: {
    borderRadius: 10,
    minHeight: 48,
    paddingHorizontal: 16,
    alignItems: "center",
    justifyContent: "center",
  },
  compact: { minHeight: 44 },
  buttonText: { fontFamily: FONTS.medium, fontSize: 15, lineHeight: 21 },
  problem: {
    borderRadius: 10,
    overflow: "hidden",
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  waiting: { flexDirection: "row", gap: 10, alignItems: "center" },
});
