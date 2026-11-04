// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The servers this phone is signed in to, one in use at a time, and removing one (31 §5). */

import { useRouter } from "expo-router";
import { useState } from "react";
import { Alert, StyleSheet, View } from "react-native";

import { useApp, useSnapshot } from "../lib/context";
import type { Profile } from "../lib/profiles";
import { stillSignedIn } from "../ui/leaving";
import { Body, Button, Group, Icon, Row, Screen } from "../ui/parts";

export default function Servers() {
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const router = useRouter();
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
            <Group>
              <Row
                title={profile.serverName}
                detail={profile.address}
                monoDetail
                leading={<Icon name="server" />}
                label={`${profile.serverName}${inUse ? ", in use" : ""}`}
                selected={inUse}
                busy={switching === profile.id}
                disabled={switching !== null || removing !== null}
                {...(inUse ? { tag: "In use" } : {})}
                onPress={() => void use(profile.id)}
              />
            </Group>
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
});
