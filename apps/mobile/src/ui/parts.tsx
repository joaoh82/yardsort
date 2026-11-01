// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The few pieces every screen is built from. Native components only: text scales with the
 * system's size, and every control says what it is to a screen reader.
 */

import type { ReactNode } from "react";
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

const LIGHT = {
  background: "#f6f6f4",
  surface: "#ffffff",
  ink: "#1c1c1a",
  muted: "#5c5c57",
  line: "#d9d9d4",
  accent: "#2f6f4f",
  onAccent: "#ffffff",
  danger: "#b3261e",
};

const DARK: typeof LIGHT = {
  background: "#151514",
  surface: "#1f1f1d",
  ink: "#ecece8",
  muted: "#a3a39c",
  line: "#3a3a36",
  accent: "#6fbf8f",
  onAccent: "#0d1f15",
  danger: "#f2b8b5",
};

export type Colors = typeof LIGHT;

export function useColors(): Colors {
  return useColorScheme() === "dark" ? DARK : LIGHT;
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

export function Card({ children, label }: { children: ReactNode; label?: string }) {
  const colors = useColors();
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
      <Text style={[styles.label, { color: colors.ink }]}>{label}</Text>
      <TextInput
        accessibilityLabel={label}
        {...(hint ? { accessibilityHint: hint } : {})}
        placeholderTextColor={colors.muted}
        style={[
          styles.input,
          { color: colors.ink, borderColor: colors.line, backgroundColor: colors.surface },
        ]}
        {...input}
      />
      {hint && <Text style={[styles.hint, { color: colors.muted }]}>{hint}</Text>}
    </View>
  );
}

export function Button({
  title,
  onPress,
  kind = "plain",
  busy = false,
  disabled = false,
}: {
  title: string;
  onPress: () => void;
  kind?: "primary" | "plain" | "danger";
  busy?: boolean;
  disabled?: boolean;
}) {
  const colors = useColors();
  const primary = kind === "primary";
  const off = disabled || busy;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={title}
      accessibilityState={{ disabled: off, busy }}
      disabled={off}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        {
          backgroundColor: primary ? colors.accent : "transparent",
          borderColor: kind === "danger" ? colors.danger : primary ? colors.accent : colors.line,
          opacity: off ? 0.5 : pressed ? 0.7 : 1,
        },
      ]}
    >
      {busy ? (
        <ActivityIndicator color={primary ? colors.onAccent : colors.ink} />
      ) : (
        <Text
          style={[
            styles.buttonText,
            {
              color: primary ? colors.onAccent : kind === "danger" ? colors.danger : colors.ink,
            },
          ]}
        >
          {title}
        </Text>
      )}
    </Pressable>
  );
}

/** Something that went wrong, announced to a screen reader as it appears. */
export function Problem({ children }: { children: ReactNode }) {
  const colors = useColors();
  return (
    <Text
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      style={[styles.body, { color: colors.danger }]}
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
  title: { fontSize: 22, fontWeight: "600" },
  body: { fontSize: 16, lineHeight: 22 },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 10, padding: 14, gap: 8 },
  field: { gap: 6 },
  label: { fontSize: 15, fontWeight: "500" },
  input: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 16,
  },
  hint: { fontSize: 13 },
  button: {
    borderWidth: 1,
    borderRadius: 8,
    minHeight: 44,
    paddingHorizontal: 16,
    alignItems: "center",
    justifyContent: "center",
  },
  buttonText: { fontSize: 16, fontWeight: "600" },
  waiting: { flexDirection: "row", gap: 10, alignItems: "center" },
});
