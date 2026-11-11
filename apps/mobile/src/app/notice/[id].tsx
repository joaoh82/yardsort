// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One notification. A workflow question or approval opens the decision.
 * Installing a desktop update stays on the computer. The read tries every
 * account this phone is signed in to, not only the one on screen.
 */

import type { NoticeView } from "@yardsort/remote-client";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";

import { messageOf } from "../../lib/accounts";
import { useApp, useSnapshot } from "../../lib/context";
import { workflowTarget } from "../../lib/workflows";
import { Body, Problem, Screen, Waiting } from "../../ui/parts";

export default function Notice() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const signedIn = snapshot.standing?.kind === "signedIn";
  const [notice, setNotice] = useState<NoticeView | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    if (!signedIn || !id) return;
    let live = true;
    void accounts.notice(id).then(
      async ({ notice: next, profile }) => {
        if (!live) return;
        if (accounts.activeId() !== profile) await accounts.activate(profile);
        if (live) setNotice(next);
      },
      (error: unknown) => {
        if (live) setProblem(messageOf(error));
      },
    );
    return () => {
      live = false;
    };
  }, [accounts, id, signedIn]);

  const target = notice ? workflowTarget(notice) : null;
  useEffect(() => {
    if (target) router.replace(target);
  }, [router, target]);

  return (
    <Screen>
      <Stack.Screen options={{ title: "Notification" }} />
      {!signedIn ? (
        <Body muted>Sign in to open this notification.</Body>
      ) : target ? (
        <Waiting label="Opening the decision" />
      ) : notice ? (
        <>
          <Body>{notice.title}</Body>
          <Body muted>{notice.body}</Body>
        </>
      ) : problem ? (
        <Problem>{problem}</Problem>
      ) : (
        <Waiting label="Opening the notification" />
      )}
    </Screen>
  );
}
