// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { Redirect } from "expo-router";

import { useSnapshot } from "../lib/context";
import { Screen, Waiting } from "../ui/parts";

/** The active profile's account, or Connect when there is none. */
export default function Start() {
  const snapshot = useSnapshot();
  if (!snapshot.loaded) {
    return (
      <Screen>
        <Waiting label="Opening" />
      </Screen>
    );
  }
  return <Redirect href={snapshot.active ? "/account" : "/connect"} />;
}
