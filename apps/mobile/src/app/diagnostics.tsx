// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What happened to this phone's connections since the app started, shown in full: what is shared
 * is the text on the screen, and nothing else (35 §19). See `lib/diagnostics.ts` for what it
 * holds and what it never does.
 */

import { CURRENT } from "@yardsort/remote-client";
import { useState, useSyncExternalStore } from "react";
import { Share, StyleSheet, Text } from "react-native";

import { useApp, useSnapshot } from "../lib/context";
import { report } from "../lib/diagnostics";
import { Body, Button, FONTS, Screen, useColors } from "../ui/parts";

export default function DiagnosticsScreen() {
  const { diagnostics, about } = useApp();
  const { active } = useSnapshot();
  const colors = useColors();
  const entries = useSyncExternalStore(diagnostics.subscribe, diagnostics.getSnapshot);
  const [opened] = useState(Date.now);
  const text = report(
    {
      app: about.app,
      system: about.system,
      protocol: `${CURRENT.major}.${CURRENT.minor}`,
      server: active?.server ?? null,
      device: active?.device ?? null,
    },
    entries,
    Math.max(opened, entries.at(-1)?.at ?? 0),
  );
  return (
    <Screen>
      <Body muted>
        Versions, ids, states and error codes, with their times, since the app started. No names,
        titles or paths, and nothing from a terminal. This is all that is shared.
      </Body>
      <Text selectable style={[styles.report, { color: colors.ink }]}>
        {text}
      </Text>
      <Button title="Share" kind="primary" onPress={() => void Share.share({ message: text })} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  report: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 16 },
});
