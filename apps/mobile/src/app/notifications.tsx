// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** Which notices this phone receives. Answering and installing stay on the computer. */

import type { NotificationPreferences } from "@yardsort/remote-client";
import { Stack } from "expo-router";
import { useCallback, useEffect, useState } from "react";

import { messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { readPushToken, requestPushPermissionOnce } from "../lib/push-token";
import { Body, Group, Problem, Row, Screen, Section, Waiting } from "../ui/parts";

export default function Notifications() {
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const signedIn = snapshot.standing?.kind === "signedIn";
  const [preferences, setPreferences] = useState<NotificationPreferences | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const load = useCallback(() => {
    if (!signedIn) return;
    void accounts.notificationPreferences().then(
      (next) => {
        setPreferences(next);
        setProblem(null);
      },
      (error: unknown) => setProblem(messageOf(error)),
    );
  }, [accounts, signedIn]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (!signedIn) return;
    let live = true;
    void (async () => {
      const allowed = await requestPushPermissionOnce();
      if (!live) return;
      if (allowed) await accounts.registerForNotifications(readPushToken);
      else await accounts.refreshQueuedNotifications();
    })().catch(() => {});
    return () => {
      live = false;
    };
  }, [accounts, signedIn]);

  const toggle = (patch: Partial<NotificationPreferences>) => {
    if (!preferences) return;
    const next = { ...preferences, ...patch };
    setPreferences(next);
    void accounts.setNotificationPreferences(next).catch((error: unknown) => {
      setProblem(messageOf(error));
      load();
    });
  };

  return (
    <Screen>
      <Stack.Screen options={{ title: "Notifications" }} />
      {!signedIn ? (
        <Body muted>Sign in to choose what reaches this phone.</Body>
      ) : preferences ? (
        <>
          <Section title="This account">
            <Group>
              <Row
                title="Mute all"
                checked={preferences.muted}
                onPress={() => toggle({ muted: !preferences.muted })}
              />
            </Group>
          </Section>
          <Section title="Categories">
            <Group>
              <Row
                title="Tasks"
                checked={preferences.task}
                disabled={preferences.muted}
                onPress={() => toggle({ task: !preferences.task })}
              />
              <Row
                title="Reviews"
                checked={preferences.review}
                disabled={preferences.muted}
                onPress={() => toggle({ review: !preferences.review })}
              />
              <Row
                title="Failures"
                checked={preferences.failure}
                disabled={preferences.muted}
                onPress={() => toggle({ failure: !preferences.failure })}
              />
              <Row
                title="Desktop updates"
                checked={preferences.desktopUpdate}
                disabled={preferences.muted}
                onPress={() => toggle({ desktopUpdate: !preferences.desktopUpdate })}
              />
              <Row
                title="Workflow questions"
                checked={preferences.workflow}
                disabled={preferences.muted}
                onPress={() => toggle({ workflow: !preferences.workflow })}
              />
              {preferences.agent === undefined ? null : (
                <Row
                  title="Agents that need you"
                  checked={preferences.agent}
                  disabled={preferences.muted}
                  onPress={() => toggle({ agent: !preferences.agent })}
                />
              )}
            </Group>
            <Body muted>
              A notification opens the item here. An agent that asks for a permission opens its
              terminal, where you can take control and answer. You install an update on the
              computer.
            </Body>
          </Section>
        </>
      ) : problem ? (
        <Problem>{problem}</Problem>
      ) : (
        <Waiting label="Loading notification settings" />
      )}
    </Screen>
  );
}
