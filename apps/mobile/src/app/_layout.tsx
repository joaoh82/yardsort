// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { DarkTheme, DefaultTheme, Stack, ThemeProvider } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { useEffect, useState } from "react";
import { useColorScheme } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { Accounts, messageOf } from "../lib/accounts";
import { AppProvider } from "../lib/context";
import { nativeEnv, secureVault, suggestedDeviceName, systemBrowser } from "../lib/native";
import { incoming } from "../lib/pairing";
import { Button, Problem, Screen, useColors } from "../ui/parts";

const app = {
  accounts: new Accounts(nativeEnv, secureVault),
  browser: systemBrowser,
  deviceName: suggestedDeviceName(),
  incoming,
};

export default function Layout() {
  const [failed, setFailed] = useState<string | null>(null);
  const colors = useColors();
  // Android's header is drawn from this, and iOS's title. Without it, a dark phone gets a white
  // header, with the status bar's white icons lost against it.
  const base = useColorScheme() === "dark" ? DarkTheme : DefaultTheme;
  const theme = {
    ...base,
    colors: {
      ...base.colors,
      primary: colors.accent,
      background: colors.background,
      card: colors.surface,
      text: colors.ink,
      border: colors.line,
    },
  };

  const load = () => app.accounts.load().catch((error: unknown) => setFailed(messageOf(error)));
  useEffect(() => {
    void load();
  }, []);

  return (
    <SafeAreaProvider>
      <AppProvider value={app}>
        <StatusBar style="auto" />
        {failed ? (
          // Nothing is shown, and nothing written, until what is kept can be read: writing over a
          // list that did not read would lose every profile in it.
          <Screen>
            <Problem>This phone’s saved sign-ins could not be read. {failed}</Problem>
            <Button
              title="Try again"
              kind="primary"
              onPress={() => {
                setFailed(null);
                void load();
              }}
            />
          </Screen>
        ) : (
          <ThemeProvider value={theme}>
            <Stack screenOptions={{ headerBackTitle: "Back" }}>
              <Stack.Screen name="index" options={{ headerShown: false }} />
              <Stack.Screen name="connect" options={{ title: "Connect" }} />
              <Stack.Screen name="account" options={{ title: "Account" }} />
              <Stack.Screen name="servers" options={{ title: "Servers" }} />
              <Stack.Screen name="computer" options={{ title: "Computer" }} />
              <Stack.Screen name="workspace" options={{ title: "Workspace" }} />
              <Stack.Screen name="memory" options={{ title: "Approved memory" }} />
              <Stack.Screen name="terminal" options={{ title: "Terminal" }} />
              {/* One at a time: a link arriving while it is open comes to this one. */}
              <Stack.Screen
                name="pair"
                options={{ title: "Pair a computer" }}
                dangerouslySingular
              />
            </Stack>
          </ThemeProvider>
        )}
      </AppProvider>
    </SafeAreaProvider>
  );
}
