// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The servers this phone is signed in to, one in use at a time (31 §5). */

import { useRouter } from "expo-router";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useApp, useSnapshot } from "../lib/context";
import { Body, Button, Screen, useColors } from "../ui/parts";

export default function Servers() {
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const router = useRouter();
  const colors = useColors();
  const [switching, setSwitching] = useState<string | null>(null);

  const use = async (id: string) => {
    setSwitching(id);
    try {
      await accounts.activate(id);
    } finally {
      setSwitching(null);
    }
    router.replace("/account");
  };

  return (
    <Screen>
      <Body muted>
        Each server’s sign-in, computers and devices are its own. Only the one in use is shown, and
        nothing of one is sent to another.
      </Body>
      {snapshot.profiles.map((profile) => {
        const inUse = profile.id === snapshot.active?.id;
        return (
          <Pressable
            key={profile.id}
            accessibilityRole="button"
            accessibilityLabel={`${profile.serverName}${inUse ? ", in use" : ""}`}
            accessibilityState={{ selected: inUse, busy: switching === profile.id }}
            disabled={switching !== null}
            onPress={() => void use(profile.id)}
            style={[styles.row, { borderColor: colors.line, backgroundColor: colors.surface }]}
          >
            <View style={styles.text}>
              <Text style={[styles.name, { color: colors.ink }]}>{profile.serverName}</Text>
              <Text style={{ color: colors.muted }}>{profile.address}</Text>
            </View>
            {inUse && <Text style={{ color: colors.accent }}>In use</Text>}
          </Pressable>
        );
      })}
      <Button title="Add a server" onPress={() => router.push("/connect")} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  row: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    padding: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  text: { flex: 1, gap: 2 },
  name: { fontSize: 16, fontWeight: "600" },
});
