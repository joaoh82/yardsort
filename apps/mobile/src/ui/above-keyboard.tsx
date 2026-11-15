// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A screen with a field at its foot, which a keyboard lifts, and maybe a bar at its head.
 *
 * Where a screen starts is the system's to say. On Android, and on iOS before 26, it starts below
 * the header. From iOS 26 the header is glass laid over the screen, which starts at the top of the
 * window. A keyboard's place is given on the window, so lifting the field by the right amount
 * takes knowing where this starts, and so does keeping a bar clear of the header. Taking both from
 * the header's height alone left the field a header's height above the keyboard on iOS 26, and the
 * bar under the status bar. So the system is asked.
 */

import { useHeaderHeight } from "expo-router/react-navigation";
import { type ReactNode, useCallback, useLayoutEffect, useRef, useState } from "react";
import { type HostInstance, KeyboardAvoidingView, StyleSheet, View } from "react-native";

export function AboveKeyboard({
  clear,
  children,
}: {
  /**
   * Start below a header laid over the screen. For a screen whose first thing is not a scroll
   * view, which makes that room by itself.
   */
  clear: boolean;
  children: ReactNode;
}) {
  const headerHeight = useHeaderHeight();
  const holder = useRef<HostInstance>(null);
  // How far down the window this starts. Until the system has said: below the header.
  const [top, setTop] = useState<number | null>(null);
  const measure = useCallback(() => {
    holder.current?.measureInWindow((_x, y) => {
      if (Number.isFinite(y)) setTop(y);
    });
  }, []);
  // Before the first frame is shown, so a bar under the header is not seen to move.
  useLayoutEffect(measure, [measure]);

  const covered = top === null ? 0 : Math.max(0, headerHeight - top);
  return (
    <View
      ref={holder}
      testID="above-keyboard"
      onLayout={measure}
      style={[styles.fill, clear && { paddingTop: covered }]}
    >
      <KeyboardAvoidingView
        testID="lifted"
        behavior="padding"
        // What takes the keyboard's place on the window to its place in here.
        keyboardVerticalOffset={top ?? headerHeight}
        style={styles.fill}
      >
        {children}
      </KeyboardAvoidingView>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
});
