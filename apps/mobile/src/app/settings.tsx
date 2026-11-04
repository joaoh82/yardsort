// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** What is this phone's own to choose, kept on the phone and never sent to a server. */

import { useSyncExternalStore } from "react";

import { appearance, type Choice, CHOICES } from "../lib/appearance";
import { Body, Group, Row, Screen, Section } from "../ui/parts";

const NAMES: Record<Choice, string> = {
  automatic: "Automatic",
  light: "Light",
  dark: "Dark",
};

export default function Settings() {
  const chosen = useSyncExternalStore(appearance.subscribe, appearance.get);
  return (
    <Screen>
      <Section title="Appearance">
        <Group>
          {CHOICES.map((choice) => (
            <Row
              key={choice}
              title={NAMES[choice]}
              plainTitle
              {...(choice === "automatic" ? { detail: "As this phone is set" } : {})}
              checked={choice === chosen}
              onPress={() => appearance.choose(choice)}
            />
          ))}
        </Group>
        <Body muted>A terminal stays dark in all of them.</Body>
      </Section>
    </Screen>
  );
}
