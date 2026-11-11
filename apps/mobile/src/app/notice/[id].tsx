// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One notification. It opens the item. It does not answer, approve, or install.
 * The read tries every account this phone is signed in to, not only the one on screen.
 */

import type { NoticeView } from "@yardsort/remote-client";
import { Stack, useLocalSearchParams } from "expo-router";
import { useEffect, useState } from "react";

import { messageOf } from "../../lib/accounts";
import { useApp, useSnapshot } from "../../lib/context";
import { Body, Problem, Screen, Waiting } from "../../ui/parts";

export default function Notice() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { accounts } = useApp();
  const snapshot = useSnapshot();
  const signedIn = snapshot.standing?.kind === "signedIn";
  const [notice, setNotice] = useState<NoticeView | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    if (!signedIn || !id) return;
    let live = true;
    void accounts.notice(id).then(
      (next) => {
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

  return (
    <Screen>
      <Stack.Screen options={{ title: "Notification" }} />
      {!signedIn ? (
        <Body muted>Sign in to open this notification.</Body>
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
