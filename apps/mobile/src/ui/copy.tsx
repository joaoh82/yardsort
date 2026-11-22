// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A small Copy button, for what a person would rather paste somewhere than select: a whole reply,
 * a message, a block of code. It copies all of it, the folded part too, as it was written.
 */

import { setStringAsync } from "expo-clipboard";
import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text } from "react-native";

import { FONTS, useColors } from "./parts";

/** How long "Copied" shows before the button says Copy again. */
const COPIED_MS = 1_500;

export function CopyButton({ text, name }: { text: string; name: string }) {
  const colors = useColors();
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!copied && !failed) return;
    const back = setTimeout(() => {
      setCopied(false);
      setFailed(false);
    }, COPIED_MS);
    return () => clearTimeout(back);
  }, [copied, failed]);
  const copy = async () => {
    try {
      await setStringAsync(text);
      setFailed(false);
      setCopied(true);
    } catch {
      setCopied(false);
      setFailed(true);
    }
  };
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={name}
      hitSlop={8}
      onPress={() => void copy()}
      style={({ pressed }) => [styles.copy, { opacity: pressed ? 0.6 : 1 }]}
    >
      <Text style={[styles.text, { color: failed ? colors.danger : colors.faint }]}>
        {copied ? "Copied" : failed ? "Could not copy" : "Copy"}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  copy: { alignSelf: "flex-start", paddingVertical: 2 },
  text: { fontFamily: FONTS.mono, fontSize: 11, lineHeight: 14, letterSpacing: 0.4 },
});
