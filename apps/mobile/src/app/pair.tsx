// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Pairing with a computer (31 §5): scan its code, type its eight characters, or come here from a
 * link. A link that names another server is asked about first, and never reuses this server's
 * sign-in. Then the app asks the computer, and waits for the person there to answer.
 */

import {
  displayCode,
  type GrantSummary,
  normalizeCode,
  type OfferKey,
  parsePairingLink,
  Secret,
} from "@yardsort/remote-client";
import { CameraView, useCameraPermissions } from "expo-camera";
import { Redirect, useRouter } from "expo-router";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Linking, StyleSheet, View } from "react-native";

import { messageOf } from "../lib/accounts";
import { useApp, useSnapshot } from "../lib/context";
import { placeOf, waitForAnswer } from "../lib/pairing";
import { Body, Button, Card, Field, Problem, Screen, Title, Waiting } from "../ui/parts";

type Stage =
  | { kind: "entry" }
  | { kind: "asking" }
  | { kind: "waiting"; grant: GrantSummary }
  | { kind: "answered"; grant: GrantSummary };

export default function Pair() {
  const { accounts, incoming } = useApp();
  const snapshot = useSnapshot();
  const router = useRouter();
  const held = useSyncExternalStore(incoming.subscribe, incoming.get);
  const [stage, setStage] = useState<Stage>({ kind: "entry" });
  const [problem, setProblem] = useState<string | null>(null);

  const { active, profiles, standing } = snapshot;
  const place = held ? placeOf(held.link, active, profiles) : null;
  const signedIn = standing?.kind === "signedIn";

  // Only the newest request moves this screen on. One overtaken stays at the server, on the account
  // screen, where it can be cancelled.
  const newest = useRef(0);
  const send = useCallback(
    (offer: OfferKey) => {
      newest.current += 1;
      const request = newest.current;
      return accounts.pair(offer).then(
        (grant) => {
          if (request === newest.current) setStage({ kind: "waiting", grant });
        },
        (error: unknown) => {
          if (request !== newest.current) return;
          setStage({ kind: "entry" });
          setProblem(messageOf(error));
        },
      );
    },
    [accounts],
  );

  // A link the person has agreed to, on the server in use. It is sent once, however often this
  // runs, and let go when the answer comes.
  const ready = held?.agreed === true && place?.kind === "here" && signedIn;
  useEffect(() => {
    if (!ready) return;
    const link = incoming.claim();
    if (link)
      void send({ kind: "secret", value: link.secret }).finally(() => incoming.finish(link));
  }, [ready, incoming, send]);

  // Waiting for the person at the computer, for as long as this screen is open.
  const waitingFor = stage.kind === "waiting" ? stage.grant : null;
  useEffect(() => {
    if (!waitingFor) return;
    const stop = new AbortController();
    waitForAnswer(accounts, waitingFor, { signal: stop.signal }).then(
      (grant) => {
        if (grant) setStage({ kind: "answered", grant });
      },
      (error: unknown) => {
        if (stop.signal.aborted) return;
        setStage({ kind: "entry" });
        setProblem(messageOf(error));
      },
    );
    return () => stop.abort();
  }, [waitingFor, accounts]);

  const home = () => router.replace("/");

  if (!snapshot.loaded) {
    return (
      <Screen>
        <Waiting label="Opening" />
      </Screen>
    );
  }
  if (!held && !active) return <Redirect href="/connect" />;

  if (ready) {
    return (
      <Screen>
        <Waiting label="Asking the computer" />
      </Screen>
    );
  }

  // A link not yet used comes before whatever this screen was doing, which waits behind it.
  if (held && place) {
    const cancel = () => {
      incoming.drop();
      if (!active) router.replace("/connect");
    };
    // Going on leaves any earlier request behind: it stays open at the server, and this screen stops
    // asking about it.
    const goOn = () => {
      newest.current += 1;
      setStage({ kind: "entry" });
      setProblem(null);
      incoming.agree();
    };
    const earlier = stage.kind === "waiting" && (
      <Body muted>
        If you go on, the request to {stage.grant.hostName} stays open, and you can cancel it on the
        account screen.
      </Body>
    );
    if (place.kind === "invalid") {
      return (
        <Screen>
          <Problem>
            This code names a server address this app will not use. Show a new code at the computer.
          </Problem>
          <Button title="OK" onPress={cancel} />
        </Screen>
      );
    }
    if (place.kind === "new") {
      return (
        <Screen>
          <Title>This code is for another server</Title>
          <Body>
            It is for {place.address}, and this phone is not signed in there. A sign-in on one
            server means nothing on another, so you would sign in there first.
          </Body>
          {earlier}
          <Button
            title="Sign in there"
            kind="primary"
            onPress={() => {
              goOn();
              router.push({ pathname: "/connect", params: { address: place.address } });
            }}
          />
          <Button title="Cancel" onPress={cancel} />
        </Screen>
      );
    }
    if (place.kind === "known") {
      return (
        <Screen>
          <Title>This code is for another server</Title>
          <Body>
            It is for {place.profile.serverName} ({place.profile.address}).
            {active ? ` This phone is using ${active.serverName}.` : ""}
          </Body>
          {earlier}
          <Button
            title={`Switch to ${place.profile.serverName} and pair`}
            kind="primary"
            onPress={() => {
              goOn();
              void accounts.activate(place.profile.id);
            }}
          />
          <Button title="Cancel" onPress={cancel} />
        </Screen>
      );
    }
    if (!signedIn) {
      return <NotReady serverName={active?.serverName} onBack={home} />;
    }
    if (!held.agreed) {
      return (
        <Screen>
          <Title>Pair with a computer?</Title>
          <Body>
            A link asks this phone to pair with a computer on {active?.serverName}. Nothing is given
            until you say yes at that computer.
          </Body>
          {earlier}
          <Button title="Ask the computer" kind="primary" onPress={goOn} />
          <Button title="Cancel" onPress={cancel} />
        </Screen>
      );
    }
  }

  if (stage.kind === "asking") {
    return (
      <Screen>
        <Waiting label="Asking the computer" />
      </Screen>
    );
  }

  if (stage.kind === "waiting") {
    const { grant } = stage;
    return (
      <Screen>
        <Title>Say yes at {grant.hostName}</Title>
        <Body>
          {grant.hostName} is asking whether to let this phone in, and what it may do. Nothing is
          given until you answer there.
        </Body>
        <Waiting label={`Waiting for ${grant.hostName}`} />
        <Button
          title="Cancel the request"
          onPress={() => {
            void accounts.unpair(grant.id).catch((error: unknown) => setProblem(messageOf(error)));
            setStage({ kind: "entry" });
          }}
        />
      </Screen>
    );
  }

  if (stage.kind === "answered") {
    return (
      <Answered grant={stage.grant} onAgain={() => setStage({ kind: "entry" })} onDone={home} />
    );
  }

  if (!signedIn) {
    return <NotReady serverName={active?.serverName} onBack={home} />;
  }

  return (
    <Entry
      serverName={active?.serverName ?? ""}
      problem={problem}
      onCode={(code) => {
        setProblem(null);
        setStage({ kind: "asking" });
        void send({ kind: "code", value: new Secret(code) });
      }}
      onLink={(scanned) => {
        setProblem(null);
        incoming.hold(scanned, true);
      }}
      onNotOurs={() => setProblem("That is not a Yardsort pairing code.")}
    />
  );
}

/** The active profile cannot ask anything yet: it is opening, or its server cannot be reached. */
function NotReady({ serverName, onBack }: { serverName: string | undefined; onBack: () => void }) {
  return (
    <Screen>
      <Waiting label={`Waiting for ${serverName ?? "the server"}`} />
      <Button title="Back" onPress={onBack} />
    </Screen>
  );
}

function Answered({
  grant,
  onAgain,
  onDone,
}: {
  grant: GrantSummary;
  onAgain: () => void;
  onDone: () => void;
}) {
  if (grant.state === "approved") {
    const may = grant.scopes.includes("terminal:control") ? "watch and type" : "watch";
    const where =
      grant.projects?.kind === "only"
        ? `${grant.projects.projects.length} of its projects`
        : "all of its projects";
    return (
      <Screen>
        <Title>{grant.hostName} let this phone in</Title>
        <Body>
          This phone may {may} on {where}. The computer can take that back whenever you choose.
        </Body>
        <Button title="Done" kind="primary" onPress={onDone} />
      </Screen>
    );
  }
  const says =
    grant.state === "denied"
      ? `${grant.hostName} said no.`
      : grant.state === "expired"
        ? `Nobody answered at ${grant.hostName} in time. Show a new code there, and try again.`
        : grant.state === "revoked"
          ? "The request was cancelled."
          : "The request ended, and the server’s answer is one this app does not know.";
  return (
    <Screen>
      <Problem>{says}</Problem>
      <Button title="Try again" kind="primary" onPress={onAgain} />
      <Button title="Back" onPress={onDone} />
    </Screen>
  );
}

function Entry({
  serverName,
  problem,
  onCode,
  onLink,
  onNotOurs,
}: {
  serverName: string;
  problem: string | null;
  onCode: (code: string) => void;
  onLink: (link: NonNullable<ReturnType<typeof parsePairingLink>>) => void;
  onNotOurs: () => void;
}) {
  const [typed, setTyped] = useState("");
  const [scanning, setScanning] = useState(false);
  const code = normalizeCode(typed);

  return (
    <Screen>
      <Body muted>
        On the computer, open Yardsort’s Settings and choose Pair a device. It shows a code that is
        good for five minutes.
      </Body>
      {problem && <Problem>{problem}</Problem>}

      {scanning ? (
        <Scanner onLink={onLink} onNotOurs={onNotOurs} />
      ) : (
        <Button title="Scan the code" kind="primary" onPress={() => setScanning(true)} />
      )}

      <Card label="Type the code">
        <Field
          label="Pairing code"
          hint={`Eight letters and digits. It goes to ${serverName}.`}
          placeholder="ABCD-EFGH"
          value={typed}
          onChangeText={setTyped}
          autoCapitalize="characters"
          autoCorrect={false}
          returnKeyType="go"
          onSubmitEditing={() => code && onCode(code)}
        />
        <Button
          title={code ? `Pair with ${displayCode(code)}` : "Pair"}
          disabled={!code}
          onPress={() => code && onCode(code)}
        />
      </Card>
    </Screen>
  );
}

function Scanner({
  onLink,
  onNotOurs,
}: {
  onLink: (link: NonNullable<ReturnType<typeof parsePairingLink>>) => void;
  onNotOurs: () => void;
}) {
  const [permission, request] = useCameraPermissions();
  // The camera reports the same code many times a second.
  const last = useRef<string | null>(null);

  if (!permission) return <Waiting label="Asking for the camera" />;
  if (!permission.granted) {
    return (
      <>
        <Body>The camera is off for this app. You can type the code below instead.</Body>
        <Button
          title={permission.canAskAgain ? "Allow the camera" : "Open Settings"}
          onPress={() => void (permission.canAskAgain ? request() : Linking.openSettings())}
        />
      </>
    );
  }
  return (
    <View style={styles.camera}>
      <CameraView
        style={StyleSheet.absoluteFill}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={({ data }) => {
          if (last.current === data) return;
          last.current = data;
          const link = parsePairingLink(data);
          if (link) onLink(link);
          else onNotOurs();
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  camera: { height: 280, borderRadius: 12, overflow: "hidden" },
});
