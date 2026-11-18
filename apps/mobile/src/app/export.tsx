// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Exporting the active profile's account: what the file holds, said before it is made, then the
 * phone's own way of saving a file. The server is asked for nothing until the person presses
 * Export.
 */

import { Redirect } from "expo-router";
import { useState } from "react";

import { messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { Body, Button, Problem, Screen, Section } from "../ui/parts";

export default function Export() {
  const { accounts, saveFile } = useApp();
  const snapshot = useSnapshot();
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  if (!snapshot.loaded) return <Screen>{null}</Screen>;
  const profile = snapshot.active;
  if (!profile) return <Redirect href="/connect" />;
  const signedIn = snapshot.standing?.kind === "signedIn";

  const exportIt = async () => {
    setBusy(true);
    setProblem(null);
    setSaved(null);
    try {
      const exported = await accounts.exportAccount();
      const name = fileName(new Date(exported.exportedAt));
      if (await saveFile(name, `${JSON.stringify(exported, null, 2)}\n`)) {
        setSaved(
          `Saved ${name}: ${counted(exported.records.length, "synced record")} from ${profile.serverName}.`,
        );
      }
    } catch (error) {
      setProblem(
        (error as { code?: string }).code === "not_found"
          ? `${profile.serverName} cannot export an account yet. Its operator has to update it.`
          : messageOf(error),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Screen>
      <Section title="What the file holds">
        <Body>Everything {profile.serverName} keeps of your account:</Body>
        <Body muted>
          • your account: the identity provider, your id there, and when it was made
        </Body>
        <Body muted>• your phones and computers, and which devices each lets in</Body>
        <Body muted>• which phones get notifications, never their tokens, and your settings</Body>
        <Body muted>• every synced project, note and workspace name, in plain text</Body>
        <Body muted>
          • how much you used each day: devices, connections and bytes, never their content
        </Body>
        <Body muted>Never a credential. Keep the file somewhere only you can read.</Body>
      </Section>
      {problem && <Problem>{problem}</Problem>}
      {saved && <Body>{saved}</Body>}
      {signedIn ? (
        <Button title="Export" kind="primary" busy={busy} onPress={() => void exportIt()} />
      ) : (
        <Body muted>Sign in to {profile.serverName} first, from Account.</Body>
      )}
    </Screen>
  );
}

/** `yardsort-account-2026-11-17.json`, by the local date. */
function fileName(at: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `yardsort-account-${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}.json`;
}

function counted(count: number, what: string): string {
  return `${count} ${what}${count === 1 ? "" : "s"}`;
}
