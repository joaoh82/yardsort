// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Typing into a terminal, natively (31 §5): a field for what the system keyboard, dictation or a
 * paste makes, and a row of the keys a phone lacks. Return in the field sends it with Enter, and a
 * paste keeps its lines, which is why the field is a multiline one that returns nothing itself.
 */

import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";

import type { TerminalControl } from "../lib/control";
import { addedCharacter, bytesOf, controlByte, type Key, KEY_NAMES } from "../lib/keys";
import { Button, useColors } from "./parts";

const ROW: { key: Key | "ctrl" | "type"; label: string }[] = [
  { key: "type", label: "Type" },
  { key: "escape", label: "Esc" },
  { key: "tab", label: "Tab" },
  { key: "ctrl", label: "Ctrl" },
  { key: "left", label: "←" },
  { key: "up", label: "↑" },
  { key: "down", label: "↓" },
  { key: "right", label: "→" },
  { key: "backspace", label: "⌫" },
  { key: "enter", label: "Enter" },
];

export function Composer({
  control,
  holding,
  draft,
  setDraft,
  applicationCursor,
}: {
  control: TerminalControl;
  holding: boolean;
  /** Kept by the screen, since it outlives holding control. */
  draft: string;
  setDraft: (text: string) => void;
  applicationCursor: boolean;
}) {
  const colors = useColors();
  const [ctrl, setCtrl] = useState(false);

  const send = () => {
    if (control.send(draft)) setDraft("");
  };

  const typeOnly = () => {
    if (control.type(draft)) setDraft("");
  };

  const press = (key: Key) => {
    setCtrl(false);
    control.press(KEY_NAMES[key], bytesOf(key, { ctrl, applicationCursor }));
  };

  const changed = (text: string) => {
    if (ctrl) {
      setCtrl(false);
      const added = addedCharacter(draft, text);
      const byte = added === null ? null : controlByte(added);
      if (added !== null && byte !== null) {
        // It was a key, and the field stays as it was.
        control.press(`Ctrl-${added.toUpperCase()}`, byte);
        return;
      }
    }
    setDraft(text);
  };

  return (
    <View style={styles.composer}>
      <View style={styles.line}>
        <TextInput
          accessibilityLabel="What to type"
          accessibilityHint="Return sends it, followed by Enter"
          testID="composer"
          editable={holding}
          multiline
          submitBehavior="submit"
          returnKeyType="send"
          autoCapitalize="none"
          autoCorrect={false}
          spellCheck={false}
          autoComplete="off"
          textContentType="none"
          placeholder="Type here"
          placeholderTextColor={colors.muted}
          value={draft}
          onChangeText={changed}
          onSubmitEditing={send}
          style={[
            styles.field,
            { color: colors.ink, borderColor: colors.line, backgroundColor: colors.surface },
          ]}
        />
        <Button title="Send" kind="primary" disabled={!holding} onPress={send} />
      </View>
      {holding && (
        <ScrollView
          horizontal
          keyboardShouldPersistTaps="always"
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={styles.keys}
        >
          {ROW.map(({ key, label }) => {
            const armed = key === "ctrl" && ctrl;
            return (
              <Pressable
                key={key}
                accessibilityRole={key === "ctrl" ? "togglebutton" : "button"}
                accessibilityLabel={
                  key === "ctrl"
                    ? "Control"
                    : key === "type"
                      ? "Type without Enter"
                      : KEY_NAMES[key]
                }
                {...(key === "ctrl" ? { accessibilityState: { checked: ctrl } } : {})}
                onPress={() => {
                  if (key === "ctrl") setCtrl(!ctrl);
                  else if (key === "type") typeOnly();
                  else press(key);
                }}
                style={({ pressed }) => [
                  styles.key,
                  {
                    borderColor: colors.line,
                    backgroundColor: armed ? colors.accent : colors.surface,
                    opacity: pressed ? 0.7 : 1,
                  },
                ]}
              >
                <Text style={[styles.keyText, { color: armed ? colors.onAccent : colors.ink }]}>
                  {label}
                </Text>
              </Pressable>
            );
          })}
        </ScrollView>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  composer: { gap: 8 },
  line: { flexDirection: "row", gap: 8, alignItems: "flex-end" },
  field: {
    flex: 1,
    minHeight: 44,
    maxHeight: 110,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 16,
  },
  keys: { gap: 6 },
  key: {
    minHeight: 44,
    minWidth: 44,
    paddingHorizontal: 10,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    alignItems: "center",
    justifyContent: "center",
  },
  keyText: { fontSize: 15, fontWeight: "500" },
});
