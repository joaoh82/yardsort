// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Appearance stays on this phone. Notification choices live on the account, on their own screen.
 * Diagnostics are read here, then shared from their own screen.
 */

import { useRouter } from "expo-router";
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
  const router = useRouter();
  return (
    <Screen>
      <Section title="Notifications">
        <Group>
          <Row title="Notifications" onPress={() => router.push("/notifications")} />
        </Group>
        <Body muted>
          A notification opens the item on this phone. You answer a workflow, approve it, and
          install an update on the computer.
        </Body>
      </Section>
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
      <Section title="Diagnostics">
        <Group>
          <Row title="Diagnostics" onPress={() => router.push("/diagnostics")} />
        </Group>
        <Body muted>
          What happened to this phone’s connections, to read and then share when something goes
          wrong.
        </Body>
      </Section>
    </Screen>
  );
}
