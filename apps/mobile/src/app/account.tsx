// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The active profile: where it stands, its computers, the account's devices, and signing out. */

import type { DevicePage, DeviceSummary } from "@yardsort/remote-client";
import { Redirect, Stack, useRouter } from "expo-router";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { Alert, StyleSheet, Text, View } from "react-native";

import { messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { moment } from "../lib/format";
import type { Profile } from "../lib/profiles";
import { Computers } from "../ui/computers";
import { Body, Button, Card, Problem, Screen, Title, useColors, Waiting } from "../ui/parts";

export default function Account() {
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const router = useRouter();
  const profile = snapshot.active;
  const standing = snapshot.standing;

  if (!snapshot.loaded) return <Screen>{null}</Screen>;
  if (!profile) return <Redirect href="/connect" />;

  const servers = <Button title="Servers on this phone" onPress={() => router.push("/servers")} />;
  const title = <Stack.Screen options={{ title: profile.serverName }} />;

  if (!standing || standing.kind === "opening") {
    return (
      <Screen>
        {title}
        <Waiting label={`Signing in to ${profile.serverName}`} />
      </Screen>
    );
  }

  if (standing.kind === "unreachable") {
    return (
      <Screen>
        {title}
        <Problem>
          {profile.serverName} could not be reached. {standing.message}
        </Problem>
        <Body muted>Nothing was changed on this phone. Your computers keep working.</Body>
        <Button title="Try again" kind="primary" onPress={() => void accounts.open()} />
        {servers}
      </Screen>
    );
  }

  if (standing.kind === "otherServer") {
    return (
      <Screen>
        {title}
        <Problem>
          {profile.address} now answers as another server, “{standing.name}”. This phone sent it
          nothing, and will not.
        </Problem>
        <Body muted>
          If the server was moved or set up again, remove it from this phone and sign in again.
        </Body>
        <Button
          title="Remove from this phone"
          kind="danger"
          onPress={() => confirmForget(profile, () => void accounts.forgetHere(profile.id))}
        />
        {servers}
      </Screen>
    );
  }

  // Keyed, so one server's devices never show under another's name, and a late answer lands nowhere.
  return <SignedIn key={profile.id} profile={profile} title={title} servers={servers} />;
}

function SignedIn({
  profile,
  title,
  servers,
}: {
  profile: Profile;
  title: ReactNode;
  servers: ReactNode;
}) {
  const { accounts } = useApp();
  const [page, setPage] = useState<DevicePage | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [signingOut, setSigningOut] = useState(false);
  /** Goes up with each pull to refresh, for the computers to ask again too. */
  const [asked, setAsked] = useState(0);

  const fetchDevices = useCallback(
    () =>
      accounts.devices().then(
        (devices) => ({ devices, problem: null }),
        (error: unknown) => ({ devices: null, problem: messageOf(error) }),
      ),
    [accounts],
  );
  const show = (answer: { devices: DevicePage | null; problem: string | null }) => {
    if (answer.devices) setPage(answer.devices);
    setProblem(answer.problem);
  };

  useEffect(() => {
    let live = true;
    void fetchDevices().then((answer) => live && show(answer));
    return () => {
      live = false;
    };
  }, [fetchDevices, profile.id]);

  const refresh = async () => {
    setRefreshing(true);
    setAsked((n) => n + 1);
    show(await fetchDevices());
    setRefreshing(false);
  };

  const signOut = async () => {
    setSigningOut(true);
    try {
      await accounts.signOut();
    } catch (error) {
      setSigningOut(false);
      Alert.alert(
        "Still signed in",
        `${messageOf(error)} ${profile.serverName} still counts this phone as signed in. You can remove it from this phone only, then revoke it from another of your devices.`,
        [
          { text: "Cancel", style: "cancel" },
          {
            text: "Remove from this phone",
            style: "destructive",
            onPress: () => void accounts.forgetHere(profile.id),
          },
        ],
      );
    }
  };

  return (
    <Screen refreshing={refreshing} onRefresh={() => void refresh()}>
      {title}
      <Body>
        Signed in to {profile.serverName} as {profile.deviceName}.
      </Body>
      <Body muted>{profile.address}</Body>

      <Computers asked={asked} />

      <Title>This account’s devices</Title>
      {problem && <Problem>{problem}</Problem>}
      {!page && !problem && <Waiting label="Asking the server" />}
      {page?.devices.map((device) => (
        <DeviceRow key={device.id} device={device} />
      ))}

      <Button
        title="Sign out"
        kind="danger"
        busy={signingOut}
        onPress={() =>
          Alert.alert(
            `Sign out of ${profile.serverName}?`,
            "Every computer this phone is paired with stops letting it in. To use them again, sign in and pair again.",
            [
              { text: "Cancel", style: "cancel" },
              { text: "Sign out", style: "destructive", onPress: () => void signOut() },
            ],
          )
        }
      />
      {servers}
    </Screen>
  );
}

function DeviceRow({ device }: { device: DeviceSummary }) {
  const colors = useColors();
  const when = device.revokedAt
    ? `Signed out ${moment(device.revokedAt)}`
    : device.lastSeenAt
      ? `Last seen ${moment(device.lastSeenAt)}`
      : "Not seen yet";
  return (
    <Card label={device.current ? `${device.name}, this phone` : device.name}>
      <View style={styles.row}>
        <Text style={[styles.name, { color: colors.ink }]}>{device.name}</Text>
        {device.current && <Text style={{ color: colors.accent }}>This phone</Text>}
      </View>
      <Body muted>
        {KINDS[device.kind]} · {PLATFORMS[device.platform]} · {when}
      </Body>
    </Card>
  );
}

function confirmForget(profile: Profile, forget: () => void): void {
  Alert.alert(
    `Remove ${profile.serverName}?`,
    "Its sign-in is deleted from this phone. Nothing is sent to the server.",
    [
      { text: "Cancel", style: "cancel" },
      { text: "Remove", style: "destructive", onPress: forget },
    ],
  );
}

const KINDS: Record<DeviceSummary["kind"], string> = {
  phone: "Phone",
  computer: "Computer",
  host: "Computer",
  other: "Device",
};

const PLATFORMS: Record<DeviceSummary["platform"], string> = {
  linux: "Linux",
  macos: "macOS",
  windows: "Windows",
  ios: "iOS",
  android: "Android",
  other: "Other",
};

const styles = StyleSheet.create({
  row: { flexDirection: "row", justifyContent: "space-between", gap: 8, flexWrap: "wrap" },
  name: { fontSize: 16, fontWeight: "600", flexShrink: 1 },
});
