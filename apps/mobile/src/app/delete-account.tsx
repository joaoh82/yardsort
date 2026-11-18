// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Deleting the active profile's account, for every device: what goes and what stays, said
 * first; then the person types the confirmation and signs in again in the browser, which is what
 * deletes. Nothing is asked of the server until they press the button.
 */

import { confirmsDeletion } from "@yardsort/remote-client";
import { Redirect, router } from "expo-router";
import { useState } from "react";

import { messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { Body, Button, Field, Problem, Screen, Section } from "../ui/parts";

export default function DeleteAccount() {
  const { accounts, browser } = useApp();
  const snapshot = useSnapshot();
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  if (!snapshot.loaded) return <Screen>{null}</Screen>;
  const profile = snapshot.active;
  if (!profile) return <Redirect href="/connect" />;
  const signedIn = snapshot.standing?.kind === "signedIn";

  const deleteIt = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const deleted = await accounts.deleteAccount(browser);
      if (deleted === null) {
        setProblem("You did not sign in again, so nothing was deleted.");
        return;
      }
      // The profile is gone from this phone now; the notice says what happened.
      router.dismissTo("/account");
    } catch (error) {
      setProblem(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Screen>
      <Section title="What goes">
        <Body>Everything {profile.serverName} keeps of your account, for every device:</Body>
        <Body muted>• every phone and computer signed in to it, each of them signed out</Body>
        <Body muted>• the devices each computer lets in, and which phones get notifications</Body>
        <Body muted>• every synced project, note and workspace name</Body>
      </Section>
      <Section title="What stays">
        <Body>
          Everything on your computers: projects, workspaces and agents carry on, and Yardsort keeps
          working there without an account.
        </Body>
        <Body muted>
          It cannot be undone. Export your account first to keep a copy. To confirm it is you, you
          sign in again in the browser.
        </Body>
      </Section>
      {problem && <Problem>{problem}</Problem>}
      {signedIn ? (
        <>
          <Field
            label="Type delete to confirm"
            value={typed}
            onChangeText={setTyped}
            autoCapitalize="none"
            autoCorrect={false}
            editable={!busy}
          />
          <Button
            title="Sign in again and delete"
            kind="danger"
            busy={busy}
            disabled={!confirmsDeletion(typed)}
            onPress={() => void deleteIt()}
          />
        </>
      ) : (
        <Body muted>Sign in to {profile.serverName} first, from Account.</Body>
      )}
    </Screen>
  );
}
