// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { Alert } from "react-native";

import { messageOf } from "../lib/accounts";

/** Signing out did not reach the server: nothing is deleted unless the person says so. */
export function stillSignedIn(serverName: string, error: unknown, forget: () => void): void {
  Alert.alert(
    "Still signed in",
    `${messageOf(error)} ${serverName} still counts this phone as signed in. You can remove it from this phone only, then revoke it from another of your devices.`,
    [
      { text: "Cancel", style: "cancel" },
      { text: "Remove from this phone", style: "destructive", onPress: forget },
    ],
  );
}
