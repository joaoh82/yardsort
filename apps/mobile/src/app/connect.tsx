// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Connect: a server's address, what that server is and can see, then sign-in through the system
 * browser (proposal §7, §9). Nothing is sent to a server but `GET /v1/server` until the person
 * chooses to sign in.
 */

import { useLocalSearchParams, useRouter } from "expo-router";
import { useState, useSyncExternalStore } from "react";

import { type Found, messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { issuerHost } from "../lib/format";
import { Body, Button, Card, Field, Mono, Problem, Screen, Title } from "../ui/parts";

export default function Connect() {
  const { accounts, browser, deviceName: suggested, incoming } = useApp();
  const snapshot = useSnapshot();
  const router = useRouter();
  // The server a pairing link named, when that is what brought the person here.
  const params = useLocalSearchParams<{ address?: string }>();
  const held = useSyncExternalStore(incoming.subscribe, incoming.get);
  // With a link waiting, back to the pair screen it was asked about on, not to a second one.
  const onward = () => (held ? router.dismissTo("/pair") : router.replace("/account"));
  const [address, setAddress] = useState(params.address ?? "");
  const [found, setFound] = useState<Found | null>(null);
  const [deviceName, setDeviceName] = useState(suggested);
  const [busy, setBusy] = useState<"lookingUp" | "signingIn" | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const lookUp = async () => {
    setBusy("lookingUp");
    setProblem(null);
    setFound(null);
    try {
      setFound(await accounts.lookUp(address));
    } catch (error) {
      setProblem(messageOf(error));
    } finally {
      setBusy(null);
    }
  };

  const signIn = async (server: Found) => {
    setBusy("signingIn");
    setProblem(null);
    try {
      if (await accounts.signIn(server, deviceName, browser)) onward();
      else setProblem("The sign-in was closed before it finished.");
    } catch (error) {
      setProblem(messageOf(error));
    } finally {
      setBusy(null);
    }
  };

  const open = async (id: string) => {
    await accounts.activate(id);
    onward();
  };

  const known = found ? snapshot.profiles.find((p) => p.address === found.client.address) : null;

  return (
    <Screen>
      {snapshot.notice && (
        <Card label="Notice">
          <Body>{snapshot.notice}</Body>
          <Button title="OK" onPress={() => accounts.dismissNotice()} />
        </Card>
      )}

      <Title>Connect to a server</Title>
      <Body muted>
        Yardsort reaches your computers through a server you sign in to: one you run yourself, or
        one run for you. Type its address as its operator gave it.
      </Body>
      {held && <Body>After you sign in, this phone asks the computer to let it in.</Body>}

      <Field
        label="Server address"
        placeholder="https://remote.example.com"
        value={address}
        onChangeText={(text) => {
          setAddress(text);
          setFound(null);
        }}
        autoCapitalize="none"
        autoCorrect={false}
        keyboardType="url"
        textContentType="URL"
        returnKeyType="go"
        onSubmitEditing={() => void lookUp()}
        editable={busy === null}
      />
      {!found && (
        <Button
          title="Look up"
          kind="primary"
          busy={busy === "lookingUp"}
          disabled={address.trim() === ""}
          onPress={() => void lookUp()}
        />
      )}

      {found && (
        <Card label="The server">
          <Title>{found.description.name}</Title>
          <Mono>{found.client.address}</Mono>
          <Body>You sign in with {issuerHost(found.description.issuer)}.</Body>
          <Body>
            Your account here is this server’s own. It is not linked to an account on any other
            server, even one with the same email address. A computer lets this phone in only after
            you say yes at that computer.
          </Body>
          <Body>
            Whoever runs this server can see what passes through it, your terminals and what you
            type into them included. The connection is encrypted on the way, but not end to end.
          </Body>
        </Card>
      )}

      {found && known && (
        <>
          <Body>This phone is already signed in to {known.serverName}.</Body>
          <Button title="Open it" kind="primary" onPress={() => void open(known.id)} />
        </>
      )}

      {found && !known && (
        <>
          <Field
            label="Name this phone"
            hint="What your computers will call it."
            value={deviceName}
            onChangeText={setDeviceName}
            autoCorrect={false}
            editable={busy === null}
          />
          <Button
            title="Sign in"
            kind="primary"
            busy={busy === "signingIn"}
            disabled={deviceName.trim() === ""}
            onPress={() => void signIn(found)}
          />
        </>
      )}

      {problem && <Problem>{problem}</Problem>}

      {snapshot.profiles.length > 0 && (
        <Button title="Servers on this phone" onPress={() => router.push("/servers")} />
      )}
      <Button title="Settings" kind="quiet" onPress={() => router.push("/settings")} />
    </Screen>
  );
}
