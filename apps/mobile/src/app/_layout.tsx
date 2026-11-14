// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { DarkTheme, DefaultTheme, router, Stack, ThemeProvider } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { useEffect, useRef, useState } from "react";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { Accounts, messageOf } from "../lib/accounts";
import { appearance } from "../lib/appearance";
import { AppProvider } from "../lib/context";
import { diagnostics } from "../lib/diagnostics";
import {
  appearanceFile,
  appVersion,
  nativeEnv,
  pickImage,
  secureVault,
  suggestedDeviceName,
  systemBrowser,
  systemVersion,
  viewsFile,
} from "../lib/native";
import { incoming } from "../lib/pairing";
import { viewChoices } from "../lib/views";
import { installPushHandlers, registerGrantedPush } from "../lib/push-token";
import { Button, FONTS, Problem, Screen, useColors, useScheme } from "../ui/parts";

appearance.start(appearanceFile);
viewChoices.start(viewsFile);

const app = {
  accounts: new Accounts(nativeEnv, secureVault),
  browser: systemBrowser,
  deviceName: suggestedDeviceName(),
  incoming,
  pickImage,
  diagnostics,
  about: { app: appVersion, system: systemVersion },
};

export default function Layout() {
  const [failed, setFailed] = useState<string | null>(null);
  // A tap must not navigate while the sign-in list failed to load: the stack is not mounted then.
  const blocked = useRef(false);
  useEffect(() => {
    blocked.current = failed !== null;
  }, [failed]);
  const colors = useColors();
  // Android's header is drawn from this. Without it, a dark phone gets a white header, with the
  // status bar's white icons lost against it. iOS 26's glass bar and its title stay the system's.
  const scheme = useScheme();
  const base = scheme === "dark" ? DarkTheme : DefaultTheme;
  const theme = {
    ...base,
    colors: {
      ...base.colors,
      background: colors.background,
      card: colors.background,
      text: colors.ink,
      border: colors.line,
    },
  };

  const load = () =>
    app.accounts
      .load()
      .then(() => {
        setFailed(null);
        // Permission was asked in Settings. When it is already granted, every signed-in account
        // gets the token. This does not ask again.
        void registerGrantedPush((read) => app.accounts.registerForNotifications(read));
      })
      .catch((error: unknown) => setFailed(messageOf(error)));
  useEffect(() => {
    void load();
  }, []);
  // Taps, including one that launched the app, and a token the system rotates. Permission is
  // asked from Settings → Notifications, not here. A tap that arrives while the saved sign-ins
  // failed to load is dropped: the stack is not mounted then.
  useEffect(
    () =>
      installPushHandlers(
        (route) => {
          if (blocked.current) return;
          router.push(route);
        },
        (token) => {
          void app.accounts.registerForNotifications(async () => token).catch(() => {});
        },
      ),
    [],
  );

  return (
    <SafeAreaProvider>
      <AppProvider value={app}>
        <StatusBar style={scheme === "dark" ? "light" : "dark"} />
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
            <Stack
              screenOptions={{
                headerBackTitle: "Back",
                headerBackButtonDisplayMode: "minimal",
                headerTitleStyle: { fontFamily: FONTS.semibold },
              }}
            >
              <Stack.Screen name="index" options={{ headerShown: false }} />
              <Stack.Screen name="connect" options={{ title: "Connect" }} />
              <Stack.Screen name="account" options={{ title: "Account" }} />
              <Stack.Screen name="servers" options={{ title: "Servers" }} />
              <Stack.Screen name="settings" options={{ title: "Settings" }} />
              <Stack.Screen name="diagnostics" options={{ title: "Diagnostics" }} />
              <Stack.Screen name="computer" options={{ title: "Computer" }} />
              <Stack.Screen name="workflows" options={{ title: "Workflows" }} />
              <Stack.Screen name="workflow" options={{ title: "Workflow" }} />
              <Stack.Screen name="decision" options={{ title: "Decision" }} />
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
