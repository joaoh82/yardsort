// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** Settings, as a gear in the header's corner of the screens it is reached from. */

import { Stack, useRouter } from "expo-router";
import { Pressable } from "react-native";

import { Icon, useColors } from "./parts";

export function SettingsCorner() {
  return <Stack.Screen options={{ headerRight: () => <SettingsButton /> }} />;
}

function SettingsButton() {
  const router = useRouter();
  const colors = useColors();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Settings"
      hitSlop={10}
      onPress={() => router.push("/settings")}
      style={({ pressed }) => ({ opacity: pressed ? 0.5 : 1 })}
    >
      <Icon name="settings" color={colors.ink} />
    </Pressable>
  );
}
