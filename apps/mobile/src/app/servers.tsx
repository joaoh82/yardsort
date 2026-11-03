// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The servers this phone is signed in to, one in use at a time, and removing one (31 §5). */

import { useRouter } from "expo-router";
import { useState } from "react";
import { Alert, Pressable, StyleSheet, Text, View } from "react-native";

import { useApp, useSnapshot } from "../lib/context";
import type { Profile } from "../lib/profiles";
import { stillSignedIn } from "../ui/leaving";
import { Body, Button, Screen, useColors } from "../ui/parts";

export default function Servers() {
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const router = useRouter();
  const colors = useColors();
  const [switching, setSwitching] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);

  const use = async (id: string) => {
    setSwitching(id);
    try {
      await accounts.activate(id);
    } finally {
      setSwitching(null);
    }
    router.dismissTo("/account");
  };

  const remove = async (profile: Profile) => {
    setRemoving(profile.id);
    try {
      await accounts.signOut(profile.id);
    } catch (error) {
      stillSignedIn(profile.serverName, error, () => void accounts.forgetHere(profile.id));
    } finally {
      setRemoving(null);
    }
  };

  return (
    <Screen>
      <Body muted>
        Each server’s sign-in, computers and devices are its own. Only the one in use is shown, and
        nothing of one is sent to another.
      </Body>
      {!snapshot.active && snapshot.profiles.length > 0 && <Body>Choose one to use.</Body>}
      {snapshot.profiles.map((profile) => {
        const inUse = profile.id === snapshot.active?.id;
        return (
          <View key={profile.id} style={styles.entry}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${profile.serverName}${inUse ? ", in use" : ""}`}
              accessibilityState={{ selected: inUse, busy: switching === profile.id }}
              disabled={switching !== null || removing !== null}
              onPress={() => void use(profile.id)}
              style={[styles.row, { borderColor: colors.line, backgroundColor: colors.surface }]}
            >
              <View style={styles.text}>
                <Text style={[styles.name, { color: colors.ink }]}>{profile.serverName}</Text>
                <Text style={{ color: colors.muted }}>{profile.address}</Text>
              </View>
              {inUse && <Text style={{ color: colors.accent }}>In use</Text>}
            </Pressable>
            <Button
              title={`Remove ${profile.serverName}`}
              kind="danger"
              busy={removing === profile.id}
              disabled={switching !== null || removing !== null}
              onPress={() => confirmRemove(profile, () => void remove(profile))}
            />
          </View>
        );
      })}
      <Button title="Add a server" onPress={() => router.push("/connect")} />
    </Screen>
  );
}

function confirmRemove(profile: Profile, remove: () => void): void {
  Alert.alert(
    `Remove ${profile.serverName}?`,
    "This phone is signed out of it, and every computer there stops letting it in. Its sign-in and what was last seen are deleted from this phone. Other servers are left as they are.",
    [
      { text: "Cancel", style: "cancel" },
      { text: "Remove", style: "destructive", onPress: remove },
    ],
  );
}

const styles = StyleSheet.create({
  entry: { gap: 8 },
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
