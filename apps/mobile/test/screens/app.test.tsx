// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The screens, through Expo Router's own in-memory router and Testing Library: what a person sees
// and presses. The server behind them is a small fake (`fake-server.ts`); the logic underneath is
// tested against the real server in `test/lib/`.

import { afterEach, beforeEach, describe, expect, jest, test } from "@jest/globals";
import { Secret } from "@yardsort/remote-client";
import { router, Stack } from "expo-router";
import { act, fireEvent, renderRouter, screen, waitFor } from "expo-router/testing-library";
import { Alert, type AlertButton, AppState, Linking } from "react-native";

import Account from "../../src/app/account";
import ComputerScreen from "../../src/app/computer";
import Connect from "../../src/app/connect";
import Start from "../../src/app/index";
import MemoryScreen from "../../src/app/memory";
import TerminalScreen from "../../src/app/terminal";
import Pair from "../../src/app/pair";
import Servers from "../../src/app/servers";
import Settings from "../../src/app/settings";
import WorkspaceScreen from "../../src/app/workspace";
import { Accounts, type Browser, type Env } from "../../src/lib/accounts";
import { AppProvider } from "../../src/lib/context";
import { Incoming } from "../../src/lib/pairing";
import { MemoryVault } from "../lib/memory-vault";
import { ADDRESS, envFor, event, FakeServer } from "./fake-server";

const OTHER_ADDRESS = "https://other.example.com";

const signedInBrowser: Browser = async (_url, redirect) => `${redirect}?code=a-code`;

async function start({
  server = new FakeServer(),
  vault = new MemoryVault(),
  browser = signedInBrowser,
  env = envFor(server),
  incoming = new Incoming(),
}: {
  server?: FakeServer;
  vault?: MemoryVault;
  browser?: Browser;
  env?: Env;
  incoming?: Incoming;
} = {}) {
  const accounts = new Accounts(env, vault);
  const app = { accounts, browser, deviceName: "iPhone", incoming };
  void accounts.load();
  await renderRouter(
    {
      _layout: () => (
        <AppProvider value={app}>
          <Stack>
            <Stack.Screen name="pair" dangerouslySingular />
          </Stack>
        </AppProvider>
      ),
      index: Start,
      connect: Connect,
      account: Account,
      servers: Servers,
      settings: Settings,
      pair: Pair,
      computer: ComputerScreen,
      workspace: WorkspaceScreen,
      memory: MemoryScreen,
      terminal: TerminalScreen,
    },
    { initialUrl: "/" },
  );
  return { server, vault, accounts };
}

/** Sign in through Connect, as a person would. */
async function signIn(name = "Ana’s phone") {
  await fireEvent.changeText(
    await screen.findByLabelText("Server address"),
    "https://remote.example.com",
  );
  await fireEvent.press(screen.getByRole("button", { name: "Look up" }));
  await fireEvent.changeText(await screen.findByLabelText("Name this phone"), name);
  await fireEvent.press(screen.getByRole("button", { name: "Sign in" }));
  await screen.findByText(`Signed in to Example Remote as ${name}.`);
}

/** Press one of the buttons an alert offers. */
async function answerAlert(text: string) {
  const buttons = (jest.mocked(Alert.alert).mock.lastCall?.[2] ?? []) as AlertButton[];
  const button = buttons.find((b) => b.text === text);
  if (!button) throw new Error(`the alert has no ${text}`);
  await act(async () => button.onPress?.());
}

// The wait for a computer's answer asks every two seconds, as it does on a phone.
jest.setTimeout(30_000);
const ANSWER = { timeout: 5_000 };

beforeEach(() => {
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe("Screens", () => {
  test("start below the header, which from iOS 26 is laid over them", async () => {
    await start();
    expect(screen.getByTestId("screen").props.contentInsetAdjustmentBehavior).toBe("automatic");
  });
});

describe("Connect", () => {
  test("looks the server up and says what it can see before anything else is sent", async () => {
    const { server } = await start();
    await fireEvent.changeText(
      await screen.findByLabelText("Server address"),
      "https://remote.example.com",
    );
    await fireEvent.press(screen.getByRole("button", { name: "Look up" }));

    expect(await screen.findByText("Example Remote")).toBeTruthy();
    expect(screen.getByText(ADDRESS)).toBeTruthy();
    expect(screen.getByText("You sign in with auth.example.com.")).toBeTruthy();
    expect(screen.getByText(/can see what passes through it/)).toBeTruthy();
    expect(screen.getByText(/not end to end/)).toBeTruthy();
    expect(screen.getByText(/not linked to an account on any other server/)).toBeTruthy();
    expect(server.asked).toEqual(["GET /v1/server"]);
  });

  test("an address that is not a server's says why", async () => {
    await start();
    await fireEvent.changeText(
      await screen.findByLabelText("Server address"),
      "http://example.com",
    );
    await fireEvent.press(screen.getByRole("button", { name: "Look up" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/starts with https:\/\//);
  });

  test("signs in, and shows the account's devices with this phone among them", async () => {
    const { server, vault } = await start();
    await signIn();
    expect(await screen.findByText("Workstation")).toBeTruthy();
    expect(screen.getByText("Ana’s phone")).toBeTruthy();
    expect(screen.getByText("This phone")).toBeTruthy();
    expect(server.devices[0]).toMatchObject({
      name: "Ana’s phone",
      kind: "phone",
      platform: "ios",
    });
    expect(vault.credentials()).toHaveLength(1);
  });

  test("a sign-in closed before the end says so, and keeps nothing", async () => {
    const { vault } = await start({ browser: async () => null });
    await fireEvent.changeText(
      await screen.findByLabelText("Server address"),
      "https://remote.example.com",
    );
    await fireEvent.press(screen.getByRole("button", { name: "Look up" }));
    await fireEvent.press(await screen.findByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/closed before it finished/);
    expect(vault.kept.size).toBe(0);
  });

  test("a server this phone is signed in to already is opened, not signed in to twice", async () => {
    const server = new FakeServer();
    const vault = new MemoryVault();
    await start({ server, vault });
    await signIn();
    await fireEvent.press(screen.getByRole("button", { name: "Servers on this phone" }));
    await fireEvent.press(await screen.findByRole("button", { name: "Add a server" }));
    await fireEvent.changeText(
      await screen.findByLabelText("Server address"),
      "https://remote.example.com/",
    );
    await fireEvent.press(screen.getByRole("button", { name: "Look up" }));
    expect(
      await screen.findByText("This phone is already signed in to Example Remote."),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Sign in" })).toBeNull();
    await fireEvent.press(screen.getByRole("button", { name: "Open it" }));
    await screen.findByText("Signed in to Example Remote as Ana’s phone.");
    expect(server.asked.filter((r) => r === "POST /v1/token")).toHaveLength(1);
  });
});

describe("Account", () => {
  test("signing out asks first, and a no changes nothing", async () => {
    const { server, vault } = await start();
    await signIn();
    await fireEvent.press(screen.getByRole("button", { name: "Sign out" }));
    expect(Alert.alert).toHaveBeenCalledWith(
      "Sign out of Example Remote?",
      expect.stringContaining("stops letting it in"),
      expect.any(Array),
    );
    await answerAlert("Cancel");
    expect(screen.getByText("Signed in to Example Remote as Ana’s phone.")).toBeTruthy();
    expect(server.asked.some((r) => r.endsWith("/revoke"))).toBe(false);
    expect(vault.credentials()).toHaveLength(1);

    await fireEvent.press(screen.getByRole("button", { name: "Sign out" }));
    await answerAlert("Sign out");
    expect(await screen.findByText("Connect to a server")).toBeTruthy();
    expect(server.devices[0]?.revokedAt).not.toBeNull();
    expect(vault.credentials()).toEqual([]);
  });

  test("signing out with the server out of reach keeps the sign-in unless the person says", async () => {
    const { server, vault } = await start();
    await signIn();
    server.down = true;
    await fireEvent.press(screen.getByRole("button", { name: "Sign out" }));
    await answerAlert("Sign out");
    await waitFor(() =>
      expect(jest.mocked(Alert.alert).mock.lastCall?.[0]).toBe("Still signed in"),
    );
    await answerAlert("Cancel");
    expect(vault.credentials()).toHaveLength(1);

    await fireEvent.press(screen.getByRole("button", { name: "Sign out" }));
    await answerAlert("Sign out");
    await waitFor(() =>
      expect(jest.mocked(Alert.alert).mock.lastCall?.[0]).toBe("Still signed in"),
    );
    await answerAlert("Remove from this phone");
    expect(await screen.findByText("Connect to a server")).toBeTruthy();
    expect(vault.credentials()).toEqual([]);
  });

  test("a phone signed out at the server is told so on opening, and keeps nothing", async () => {
    const server = new FakeServer();
    const vault = new MemoryVault();
    const first = new Accounts(envFor(server), vault);
    await first.load();
    await first.signIn(await first.lookUp(ADDRESS), "Ana’s phone", signedInBrowser);
    server.revoke(first.getSnapshot().active?.device as string);

    await start({ server, vault });
    expect(await screen.findByText(/This phone was signed out of Example Remote/)).toBeTruthy();
    expect(screen.getByText("Connect to a server")).toBeTruthy();
    expect(vault.credentials()).toEqual([]);
  });

  test("a server out of reach on opening deletes nothing, and can be tried again", async () => {
    const server = new FakeServer();
    const vault = new MemoryVault();
    const first = new Accounts(envFor(server), vault);
    await first.load();
    await first.signIn(await first.lookUp(ADDRESS), "Ana’s phone", signedInBrowser);

    server.down = true;
    await start({ server, vault });
    expect(await screen.findByText(/Example Remote could not be reached/)).toBeTruthy();
    expect(vault.credentials()).toHaveLength(1);
    server.down = false;
    await fireEvent.press(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Signed in to Example Remote as Ana’s phone.");
  });

  test("one server's devices do not show under another's name", async () => {
    const server = new FakeServer();
    const other = new FakeServer();
    other.name = "Other Remote";
    other.server = "srv-other";
    other.devices[0]!.name = "Other workstation";
    const OTHER_ADDRESS = "https://other.example.com/";
    const env: Env = {
      ...envFor(server),
      fetch: ((input: string, init?: RequestInit) =>
        (String(input).startsWith(OTHER_ADDRESS) ? other : server).fetch(
          input,
          init,
        )) as typeof fetch,
    };
    const { accounts } = await start({ server, env });
    await signIn();
    await screen.findByText("Workstation");

    let answer = () => {};
    other.devicesAfter = new Promise((resolve) => (answer = resolve));
    await act(async () => {
      await accounts.signIn(await accounts.lookUp(OTHER_ADDRESS), "Ana’s phone", signedInBrowser);
    });
    await screen.findByText("Signed in to Other Remote as Ana’s phone.");
    expect(screen.queryByText("Workstation")).toBeNull();
    expect(screen.getByText("Asking the server")).toBeTruthy();

    await act(async () => answer());
    await screen.findByText("Other workstation");
  });
});

describe("Servers", () => {
  /** Signed in to two servers, the second in use, on the list of them. */
  async function onTwo() {
    const server = new FakeServer();
    const other = new FakeServer();
    other.name = "Other Remote";
    other.server = "srv-other";
    const env: Env = {
      ...envFor(server),
      fetch: ((input: string, init?: RequestInit) =>
        (String(input).startsWith(OTHER_ADDRESS) ? other : server).fetch(
          input,
          init,
        )) as typeof fetch,
    };
    const app = await start({ server, env });
    await signIn();
    await act(async () => {
      await app.accounts.signIn(
        await app.accounts.lookUp(OTHER_ADDRESS),
        "Ana’s phone",
        signedInBrowser,
      );
    });
    await screen.findByText("Signed in to Other Remote as Ana’s phone.");
    await fireEvent.press(await screen.findByRole("button", { name: "Servers on this phone" }));
    await screen.findByRole("button", { name: "Remove Example Remote" });
    return { ...app, server, other };
  }

  test("removing one asks first, and a no changes nothing", async () => {
    const { server, vault } = await onTwo();
    await fireEvent.press(screen.getByRole("button", { name: "Remove Example Remote" }));
    await answerAlert("Cancel");
    expect(vault.credentials()).toHaveLength(2);
    expect(server.devices.every((d) => !d.revokedAt)).toBe(true);
    expect(screen.getByRole("button", { name: "Example Remote" })).toBeTruthy();
  });

  test("removing the one not in use signs out of it there, and leaves the other", async () => {
    const { server, other, vault } = await onTwo();
    await fireEvent.press(screen.getByRole("button", { name: "Remove Example Remote" }));
    await answerAlert("Remove");
    await waitFor(() => expect(vault.credentials()).toHaveLength(1));
    expect(server.devices.find((d) => d.name === "Ana’s phone")?.revokedAt).not.toBeNull();
    expect(other.devices.find((d) => d.name === "Ana’s phone")?.revokedAt).toBeNull();
    expect(screen.queryByRole("button", { name: "Example Remote" })).toBeNull();
    expect(screen.getByRole("button", { name: "Other Remote, in use" })).toBeTruthy();
  });

  test("removing the one in use leaves the other to choose", async () => {
    const { vault } = await onTwo();
    await fireEvent.press(screen.getByRole("button", { name: "Remove Other Remote" }));
    await answerAlert("Remove");
    await screen.findByText("Choose one to use.");
    expect(vault.credentials()).toHaveLength(1);
    await fireEvent.press(screen.getByRole("button", { name: "Example Remote" }));
    expect(await screen.findByText("Signed in to Example Remote as Ana’s phone.")).toBeTruthy();
  });

  test("with its server out of reach it says so, and deletes only if told", async () => {
    const { server, vault } = await onTwo();
    server.down = true;
    await fireEvent.press(screen.getByRole("button", { name: "Remove Example Remote" }));
    await answerAlert("Remove");
    await waitFor(() =>
      expect(jest.mocked(Alert.alert).mock.lastCall?.[0]).toBe("Still signed in"),
    );
    expect(vault.credentials()).toHaveLength(2);
    await answerAlert("Cancel");
    expect(vault.credentials()).toHaveLength(2);

    await fireEvent.press(screen.getByRole("button", { name: "Remove Example Remote" }));
    await answerAlert("Remove");
    await waitFor(() =>
      expect(jest.mocked(Alert.alert).mock.lastCall?.[0]).toBe("Still signed in"),
    );
    await answerAlert("Remove from this phone");
    await waitFor(() => expect(vault.credentials()).toHaveLength(1));
    expect(screen.queryByRole("button", { name: "Example Remote" })).toBeNull();
  });

  test("switching shows nothing of the other server's computers or devices", async () => {
    const { server, other } = await onTwo();
    other.devices[0]!.name = "Only on the other";
    server.devices.find((d) => d.name === "Workstation")!.name = "Only on the first";
    await fireEvent.press(screen.getByRole("button", { name: "Example Remote" }));
    await screen.findByText("Only on the first");
    expect(screen.queryByText("Only on the other")).toBeNull();
    await fireEvent.press(screen.getByRole("button", { name: "Servers on this phone" }));
    await fireEvent.press(await screen.findByRole("button", { name: "Other Remote" }));
    await screen.findByText("Only on the other");
    expect(screen.queryByText("Only on the first")).toBeNull();
  });
});

describe("Computers", () => {
  test("none yet: it says how to start", async () => {
    await start();
    await signIn();
    expect(await screen.findByText(/No computer lets this phone in yet/)).toBeTruthy();
  });

  /** Signed in, with the computer having said yes. */
  async function pairedComputer(server = new FakeServer()) {
    const app = await start({ server });
    await signIn();
    await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
    await fireEvent.changeText(await screen.findByLabelText("Pairing code"), "abcd-2345");
    await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
    await screen.findByText("Say yes at Workstation");
    server.answer("approved");
    await screen.findByText("Workstation let this phone in", undefined, ANSWER);
    await fireEvent.press(screen.getByRole("button", { name: "Done" }));
    await screen.findByLabelText(/^Workstation, /);
    return app;
  }

  test("settings can be reached before there is any server", async () => {
    await start();
    await fireEvent.press(await screen.findByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("radio", { name: "Dark" })).toBeTruthy();
  });

  test("settings can be reached signed in", async () => {
    await start();
    await signIn();
    await fireEvent.press(await screen.findByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("radio", { name: "Dark" })).toBeTruthy();
  });

  test("done with pairing returns to the account, not to a second one on top", async () => {
    await pairedComputer();
    expect(router.canGoBack()).toBe(false);
  });

  test("a computer that answers is online", async () => {
    await pairedComputer();
    expect(await screen.findByText("Online")).toBeTruthy();
    expect(screen.queryByText(/Offline does not mean/)).toBeNull();
  });

  test("a computer the relay says is offline shows when it was last seen, and that its agents go on", async () => {
    const server = new FakeServer();
    server.offline.add("host-work");
    await pairedComputer(server);
    expect(await screen.findByText(/^Offline, last seen/)).toBeTruthy();
    expect(screen.getByText("Offline does not mean its agents stopped.")).toBeTruthy();
  });

  test("channels are held only while the list is on screen", async () => {
    const { server } = await pairedComputer();
    await screen.findByText("Online");
    expect(server.sockets.open).toBe(1);

    await act(async () => router.push("/pair"));
    await waitFor(() => expect(server.sockets.open).toBe(0));

    await act(async () => router.back());
    await waitFor(() => expect(server.sockets.open).toBe(1));
    expect(server.sockets.ever).toBe(2);
  });

  test("channels close in the background, and open again on coming back", async () => {
    const changes: ((state: string) => void)[] = [];
    jest.spyOn(AppState, "addEventListener").mockImplementation(((
      _: string,
      listener: (state: string) => void,
    ) => {
      changes.push(listener);
      return { remove: () => {} };
    }) as typeof AppState.addEventListener);

    const { server } = await pairedComputer();
    await screen.findByText("Online");
    expect(server.sockets.open).toBe(1);

    await act(async () => changes.forEach((tell) => tell("background")));
    await waitFor(() => expect(server.sockets.open).toBe(0));

    await act(async () => changes.forEach((tell) => tell("active")));
    await waitFor(() => expect(server.sockets.open).toBe(1));
    expect(server.sockets.ever).toBe(2);
  });

  test("unpairing asks first, and a no changes nothing", async () => {
    const { server } = await pairedComputer();
    await screen.findByText("Online");
    // On the computer's own screen, so that a list of computers holds nothing that ends one.
    expect(screen.queryByRole("button", { name: "Unpair Workstation" })).toBeNull();
    await fireEvent.press(screen.getByRole("button", { name: /^Workstation, / }));
    await fireEvent.press(await screen.findByRole("button", { name: "Unpair Workstation" }));
    await answerAlert("Cancel");
    expect(server.grants[0]?.state).toBe("approved");
    expect(screen.getByRole("button", { name: "Unpair Workstation" })).toBeTruthy();

    await fireEvent.press(screen.getByRole("button", { name: "Unpair Workstation" }));
    await answerAlert("Unpair");
    // Back at the account, with the computer gone from it.
    await screen.findByText(/No computer lets this phone in yet/);
    expect(server.grants[0]?.state).toBe("revoked");
    // Still signed in.
    expect(screen.getByText("Signed in to Example Remote as Ana’s phone.")).toBeTruthy();
  });

  test("unpairing that cannot reach the server stays, and says so", async () => {
    const { server } = await pairedComputer();
    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
    const button = await screen.findByRole("button", { name: "Unpair Workstation" });
    server.down = true;
    await fireEvent.press(button);
    await answerAlert("Unpair");
    await screen.findByRole("alert");
    expect(server.grants[0]?.state).toBe("approved");
    expect(screen.getByRole("button", { name: "Unpair Workstation" })).toBeTruthy();
  });

  test("a request still waiting is shown, and can be cancelled", async () => {
    const { server } = await start();
    await signIn();
    await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
    await fireEvent.changeText(await screen.findByLabelText("Pairing code"), "ABCD2345");
    await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
    await screen.findByText("Say yes at Workstation");

    await act(async () => router.back());
    await screen.findByText("Waiting for an answer at Workstation.");
    await fireEvent.press(screen.getByRole("button", { name: "Cancel the request" }));
    await screen.findByText(/No computer lets this phone in yet/);
    expect(server.grants[0]?.state).toBe("revoked");
  });
});

describe("Pairing", () => {
  async function pairScreen(options: Parameters<typeof start>[0] = {}) {
    const app = await start(options);
    await signIn();
    await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
    await screen.findByLabelText("Pairing code");
    return app;
  }

  test("a code that cannot be one cannot be sent", async () => {
    const { server } = await pairScreen();
    await fireEvent.changeText(screen.getByLabelText("Pairing code"), "ABC");
    expect(screen.getByRole("button", { name: "Pair" }).props.accessibilityState?.disabled).toBe(
      true,
    );
    expect(server.presented).toEqual([]);
  });

  test("a wrong or expired code says so, and the screen stays to try again", async () => {
    const { server } = await pairScreen();
    server.offers.clear();
    await fireEvent.changeText(screen.getByLabelText("Pairing code"), "ABCD2345");
    await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
    expect(await screen.findByText(/wrong or has expired/)).toBeTruthy();
    expect(screen.getByLabelText("Pairing code")).toBeTruthy();
  });

  for (const [state, says] of [
    ["denied", "Workstation said no."],
    ["expired", /Nobody answered at Workstation in time/],
  ] as const) {
    test(`${state} at the computer is told, and can be tried again`, async () => {
      const { server } = await pairScreen();
      await fireEvent.changeText(screen.getByLabelText("Pairing code"), "ABCD2345");
      await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
      await screen.findByText("Say yes at Workstation");
      server.answer(state);
      expect(await screen.findByText(says, undefined, ANSWER)).toBeTruthy();
      await fireEvent.press(screen.getByRole("button", { name: "Try again" }));
      expect(await screen.findByLabelText("Pairing code")).toBeTruthy();
    });
  }

  test("what it was given is said, as the computer chose", async () => {
    const { server } = await pairScreen();
    await fireEvent.changeText(screen.getByLabelText("Pairing code"), "ABCD2345");
    await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
    await screen.findByText("Say yes at Workstation");
    server.answer("approved");
    const grant = server.grants[0] as (typeof server.grants)[number];
    grant.scopes = ["workspace:read", "terminal:read"];
    await screen.findByText("Workstation let this phone in", undefined, ANSWER);
    expect(screen.getByText(/This phone may watch on all of its projects/)).toBeTruthy();
  });

  describe("scanning", () => {
    const LINK = "yardsort://pair?server=https%3A%2F%2Fremote.example.com&secret=ys1.of.scanned";

    async function scan(data: string) {
      await fireEvent(await screen.findByTestId("camera"), "barcodeScanned", { data });
    }

    test("the camera opens only when asked for, and a code asks at once", async () => {
      const server = new FakeServer();
      server.offers.add("ys1.of.scanned");
      await pairScreen({ server });
      expect(screen.queryByTestId("camera")).toBeNull();
      await fireEvent.press(screen.getByRole("button", { name: "Scan the code" }));
      await scan(LINK);
      await screen.findByText("Say yes at Workstation");
      expect(server.presented).toEqual(["ys1.of.scanned"]);
    });

    test("the same code seen many times is sent once", async () => {
      const server = new FakeServer();
      server.offers.add("ys1.of.scanned");
      await pairScreen({ server });
      await fireEvent.press(screen.getByRole("button", { name: "Scan the code" }));
      const camera = await screen.findByTestId("camera");
      for (let seen = 0; seen < 5; seen++) {
        await fireEvent(camera, "barcodeScanned", { data: LINK });
      }
      await screen.findByText("Say yes at Workstation");
      expect(server.presented).toEqual(["ys1.of.scanned"]);
    });

    test("anything else is told apart, and nothing is sent", async () => {
      const { server } = await pairScreen();
      await fireEvent.press(screen.getByRole("button", { name: "Scan the code" }));
      await scan("https://example.com/not-a-pairing");
      expect(await screen.findByText("That is not a Yardsort pairing code.")).toBeTruthy();
      expect(server.presented).toEqual([]);
    });
  });

  describe("a link from outside the app", () => {
    const secret = new Secret("ys1.of.linked");
    const OTHER = OTHER_ADDRESS;

    /** Two servers behind one phone, the phone signed in to the first. */
    async function twoServers(linkServer: string, incoming = new Incoming()) {
      const server = new FakeServer();
      server.offers.add("ys1.of.linked");
      const other = new FakeServer();
      other.name = "Other Remote";
      other.server = "srv-other";
      other.offers.add("ys1.of.linked");
      const env: Env = {
        ...envFor(server),
        fetch: ((input: string, init?: RequestInit) =>
          (String(input).startsWith(OTHER) ? other : server).fetch(input, init)) as typeof fetch,
        socket: server.socket,
      };
      const app = await start({ server, env, incoming });
      await signIn();
      incoming.hold({ server: linkServer, secret }, false);
      await act(async () => router.push("/pair"));
      return { ...app, server, other, incoming };
    }

    test("on the server in use: it asks first, and a no sends nothing", async () => {
      const { server, incoming } = await twoServers(ADDRESS);
      await screen.findByText("Pair with a computer?");
      expect(server.presented).toEqual([]);
      await fireEvent.press(screen.getByRole("button", { name: "Cancel" }));
      await screen.findByLabelText("Pairing code");
      expect(incoming.get()).toBeNull();
      expect(server.presented).toEqual([]);
    });

    test("on the server in use: a yes asks the computer", async () => {
      const { server, incoming } = await twoServers(ADDRESS);
      await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
      await screen.findByText("Say yes at Workstation");
      expect(server.presented).toEqual(["ys1.of.linked"]);
      expect(incoming.get()).toBeNull();
    });

    describe("arriving while the screen is busy with another", () => {
      const newer = new Secret("ys1.of.newer");

      /** The first link sent and answered by the server only when told. */
      async function firstHeldBack() {
        const app = await twoServers(ADDRESS);
        app.server.offers.add("ys1.of.newer");
        let release = () => {};
        app.server.pairAfter.set(
          "ys1.of.linked",
          new Promise<void>((resolve) => {
            release = resolve;
          }),
        );
        await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
        await waitFor(() => expect(app.server.presented).toEqual(["ys1.of.linked"]));
        await act(async () => app.incoming.hold({ server: ADDRESS, secret: newer }, false));
        await screen.findByText("Pair with a computer?");
        return { ...app, release };
      }

      /** Let the first link's answer come back, all the way to the screen. */
      async function answerFirst(app: { server: FakeServer; release: () => void }, before: number) {
        app.release();
        await waitFor(() => expect(app.server.grants).toHaveLength(before + 1));
        await act(async () => jest.advanceTimersByTimeAsync(20));
      }

      test("the first one's answer does not lose it", async () => {
        const app = await firstHeldBack();
        await answerFirst(app, 0);
        expect(app.incoming.get()?.link.secret).toBe(newer);
        expect(screen.getByText("Pair with a computer?")).toBeTruthy();
        await fireEvent.press(screen.getByRole("button", { name: "Ask the computer" }));
        await screen.findByText("Say yes at Workstation");
        expect(app.server.presented).toEqual(["ys1.of.linked", "ys1.of.newer"]);
        expect(app.incoming.get()).toBeNull();
      });

      test("the first one's late answer does not take the screen from it", async () => {
        const app = await firstHeldBack();
        await fireEvent.press(screen.getByRole("button", { name: "Ask the computer" }));
        await screen.findByText("Say yes at Workstation");
        (app.server.hosts[0] as { name: string }).name = "Late";
        await answerFirst(app, 1);
        expect(screen.getByText("Say yes at Workstation")).toBeTruthy();
      });

      test("waiting for a computer: it asks first, and a no goes back to waiting", async () => {
        const { server, incoming } = await twoServers(ADDRESS);
        server.offers.add("ys1.of.newer");
        await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
        await screen.findByText("Say yes at Workstation");

        await act(async () => incoming.hold({ server: ADDRESS, secret: newer }, false));
        await screen.findByText("Pair with a computer?");
        expect(screen.getByText(/the request to Workstation stays open/)).toBeTruthy();
        expect(server.presented).toEqual(["ys1.of.linked"]);

        await fireEvent.press(screen.getByRole("button", { name: "Cancel" }));
        await screen.findByText("Say yes at Workstation");
        expect(server.presented).toEqual(["ys1.of.linked"]);
      });

      test("waiting for a computer: a yes goes on, and the earlier request stays open", async () => {
        const { server, incoming } = await twoServers(ADDRESS);
        server.offers.add("ys1.of.newer");
        await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
        await screen.findByText("Say yes at Workstation");

        await act(async () => incoming.hold({ server: ADDRESS, secret: newer }, false));
        await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
        await waitFor(() => expect(server.presented).toEqual(["ys1.of.linked", "ys1.of.newer"]));
        await screen.findByText("Say yes at Workstation");
        expect(server.grants.map((grant) => grant.state)).toEqual(["pending", "pending"]);

        // The newest is the one this screen is waiting for.
        server.answer("denied");
        expect(await screen.findByText("Workstation said no.", undefined, ANSWER)).toBeTruthy();
      });

      test("showing an answer: it asks first, and a no goes back to the answer", async () => {
        const { server, incoming } = await twoServers(ADDRESS);
        await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
        await screen.findByText("Say yes at Workstation");
        server.answer("approved");
        await screen.findByText("Workstation let this phone in", undefined, ANSWER);

        await act(async () => incoming.hold({ server: ADDRESS, secret: newer }, false));
        await screen.findByText("Pair with a computer?");
        await fireEvent.press(screen.getByRole("button", { name: "Cancel" }));
        await screen.findByText("Workstation let this phone in");
      });

      test("naming another server: going on stops asking about the earlier one", async () => {
        const { server, accounts, incoming } = await twoServers(ADDRESS);
        await fireEvent.press(await screen.findByRole("button", { name: "Cancel" }));
        await act(async () => {
          await accounts.signIn(await accounts.lookUp(OTHER), "Ana’s phone", signedInBrowser);
        });
        await act(async () => incoming.hold({ server: OTHER, secret }, false));
        await fireEvent.press(await screen.findByRole("button", { name: "Ask the computer" }));
        await screen.findByText("Say yes at Workstation");

        server.offers.add("ys1.of.newer");
        let release = () => {};
        server.pairAfter.set(
          "ys1.of.newer",
          new Promise<void>((resolve) => {
            release = resolve;
          }),
        );
        await act(async () => incoming.hold({ server: ADDRESS, secret: newer }, false));
        expect(await screen.findByText(/the request to Workstation stays open/)).toBeTruthy();
        await fireEvent.press(
          screen.getByRole("button", { name: "Switch to Example Remote and pair" }),
        );
        await waitFor(() => expect(server.presented).toEqual(["ys1.of.newer"]));

        // Longer than the wait between asks: the earlier grant means nothing to this server.
        await act(async () => jest.advanceTimersByTimeAsync(2_500));
        expect(server.asked.filter((asked) => asked.startsWith("GET /v1/grants/"))).toEqual([]);

        release();
        await screen.findByText("Say yes at Workstation");
      });
    });

    test("on a server this phone is not signed in to: nothing goes to either until it has signed in there", async () => {
      const { server, other } = await twoServers(OTHER);
      await screen.findByText("This code is for another server");
      expect(screen.getByText(/It is for https:\/\/other\.example\.com\//)).toBeTruthy();
      expect(server.presented).toEqual([]);

      await fireEvent.press(screen.getByRole("button", { name: "Sign in there" }));
      // Connect, with the address filled in and what that server can see still to read.
      await fireEvent.press(await screen.findByRole("button", { name: "Look up" }));
      await screen.findByText(/Whoever runs this server can see/);
      expect(other.presented).toEqual([]);
      await fireEvent.press(screen.getByRole("button", { name: "Sign in" }));

      await screen.findByText("Say yes at Workstation");
      expect(other.presented).toEqual(["ys1.of.linked"]);
      expect(server.presented).toEqual([]);
      // The first server's sign-in was never shown to the second.
      expect(other.asked).not.toContain("GET /v1/grants");
    });

    test("on another server this phone is signed in to: it switches only when told", async () => {
      const { server, other, accounts, incoming } = await twoServers(ADDRESS);
      await fireEvent.press(await screen.findByRole("button", { name: "Cancel" }));
      await act(async () => {
        await accounts.signIn(await accounts.lookUp(OTHER), "Ana’s phone", signedInBrowser);
      });
      expect(accounts.getSnapshot().active?.serverName).toBe("Other Remote");

      await act(async () => incoming.hold({ server: ADDRESS, secret }, false));
      await screen.findByText(/It is for Example Remote/);
      expect(accounts.getSnapshot().active?.serverName).toBe("Other Remote");
      expect(server.presented).toEqual([]);

      await fireEvent.press(
        screen.getByRole("button", { name: "Switch to Example Remote and pair" }),
      );
      await screen.findByText("Say yes at Workstation");
      expect(accounts.getSnapshot().active?.serverName).toBe("Example Remote");
      expect(server.presented).toEqual(["ys1.of.linked"]);
      expect(other.presented).toEqual([]);
    });

    test("naming an address the app would not talk to goes nowhere", async () => {
      const { server, incoming } = await twoServers("http://plain.example.com");
      await screen.findByText(/names a server address this app will not use/);
      await fireEvent.press(screen.getByRole("button", { name: "OK" }));
      expect(incoming.get()).toBeNull();
      expect(server.presented).toEqual([]);
    });
  });
});

describe("A computer", () => {
  /** Signed in, with the computer letting this phone in, and something to keep things on. */
  async function paired(shelf = new MemoryVault(), server = new FakeServer()) {
    const app = await start({ server, env: { ...envFor(server), shelf } });
    await signIn();
    await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
    await fireEvent.changeText(await screen.findByLabelText("Pairing code"), "ABCD2345");
    await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
    await screen.findByText("Say yes at Workstation");
    server.answer("approved");
    await fireEvent.press(await screen.findByRole("button", { name: "Done" }, ANSWER));
    await screen.findByRole("button", { name: /^Workstation, / });
    return { ...app, server, shelf };
  }

  async function open() {
    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
    return screen.findByRole("button", { name: /^fix-login/ });
  }

  test("lists its workspaces by project, and goes into one", async () => {
    await paired();
    const row = await open();
    expect(row.props.accessibilityLabel).toBe("fix-login, ys/fix-login · 1 running");
    expect(screen.getByText("demo")).toBeTruthy();
    expect(screen.getByText("Online")).toBeTruthy();

    await fireEvent.press(row);
    expect(await screen.findByText("The login page times out")).toBeTruthy();
    expect(screen.getByText("claude · a-model")).toBeTruthy();
    // Quiet says only that nothing printed, never that it waits for anything.
    expect(screen.getByText("Nothing printed for 3 min")).toBeTruthy();
  });

  test("more than a page is asked for when wanted", async () => {
    const server = new FakeServer();
    server.computer.moreWorkspaces = [
      { ...server.computer.workspaces[0], id: "w2", name: "second" },
    ];
    await paired(new MemoryVault(), server);
    await open();
    expect(screen.queryByRole("button", { name: /^second/ })).toBeNull();
    await fireEvent.press(screen.getByRole("button", { name: "Show more" }));
    expect(await screen.findByRole("button", { name: /^second/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /^fix-login/ })).toBeTruthy();
  });

  test("what a grant does not reach says so, in the computer's words", async () => {
    const server = new FakeServer();
    server.computer.refuses.set("workspace.list", "This phone may not see workspaces here.");
    await paired(new MemoryVault(), server);
    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
    expect(await screen.findByText("This phone may not see workspaces here.")).toBeTruthy();
  });

  describe("a workspace", () => {
    async function inside(server = new FakeServer()) {
      const app = await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await screen.findByText("Following as it is recorded.");
      return app;
    }

    test("shows what was recorded, and what is recorded next as it happens", async () => {
      const { server } = await inside();
      expect(screen.getByText("claude started")).toBeTruthy();
      await act(async () =>
        server.record(event("event-2", "tool.failed", { tool: "Bash", exitCode: 1 })),
      );
      expect(await screen.findByText("Bash failed")).toBeTruthy();
      // The same event told again is shown once.
      await act(async () =>
        server.record(event("event-2", "tool.failed", { tool: "Bash", exitCode: 1 })),
      );
      expect(screen.getAllByText("Bash failed")).toHaveLength(1);
    });

    test("says when an agent reports only its start and end, until one reports more", async () => {
      const { server } = await inside();
      const only = /^Only starts and ends show/;
      expect(screen.queryByText(only)).toBeNull();
      await act(async () =>
        server.record(
          event("event-4", "process.started", { runKind: "harness", harness: "claude" }),
        ),
      );
      expect(await screen.findByText(only)).toBeTruthy();
      await act(async () =>
        server.record(
          event("event-5", "process.started", {
            runKind: "harness",
            harness: "claude",
            capture: "hook",
          }),
        ),
      );
      await waitFor(() => expect(screen.queryByText(only)).toBeNull());
    });

    test("a kind this build does not know is shown by its name", async () => {
      const { server } = await inside();
      await act(async () => server.record(event("event-3", "something.new")));
      expect(await screen.findByText("something.new")).toBeTruthy();
    });

    test("older activity is read when asked for", async () => {
      const server = new FakeServer();
      server.computer.olderEvents = [event("event-0", "turn.started", { turnNumber: 1 })];
      await inside(server);
      expect(screen.queryByText("agent started a turn")).toBeNull();
      await fireEvent.press(screen.getByRole("button", { name: "Show older" }));
      expect(await screen.findByText("agent started a turn")).toBeTruthy();
      expect(screen.getByText("claude started")).toBeTruthy();
    });

    test("ended conversations show when asked for", async () => {
      const server = new FakeServer();
      server.computer.endedSessions = [
        {
          ...server.computer.sessions[0],
          id: "session-2",
          title: "An older conversation",
          endedAt: 1_700_000_000_500,
          exitCode: 2,
          process: "ended",
          terminal: null,
        },
      ];
      await inside(server);
      expect(screen.queryByText("An older conversation")).toBeNull();
      await fireEvent.press(screen.getByRole("button", { name: "Show ended conversations" }));
      expect(await screen.findByText("An older conversation")).toBeTruthy();
      expect(screen.getByText("Ended with 2")).toBeTruthy();
    });

    test("approved memory can be read, and nothing there changes it", async () => {
      const { server } = await inside();
      await fireEvent.press(screen.getByRole("button", { name: "Approved memory" }));
      expect(await screen.findByText("Migrations are append-only.")).toBeTruthy();
      expect(screen.queryByRole("button", { name: /approve|reject|edit|delete/i })).toBeNull();
      expect(server.operations.filter((o) => o.op.startsWith("memory."))).toEqual([
        { op: "memory.list", params: expect.objectContaining({ project: "project-1" }) },
      ]);
    });

    test("holds one channel at a time, to the screen being looked at", async () => {
      const { server } = await inside();
      expect(server.sockets.open).toBe(1);
      await act(async () => router.back());
      await screen.findByRole("button", { name: /^fix-login/ });
      await waitFor(() => expect(server.sockets.open).toBe(1));
    });
  });

  describe("a terminal", () => {
    const told = () =>
      (jest.requireMock("react-native-webview") as { told: string[] }).told as string[];
    const page = () => screen.getByTestId("terminal");
    const pageSays = (message: unknown): Promise<unknown> =>
      act(async () =>
        fireEvent(page(), "message", { nativeEvent: { data: JSON.stringify(message) } }),
      );
    const outputs = () =>
      told()
        .map((text) => JSON.parse(text))
        .filter((m) => m.t === "output")
        .map((m) => Buffer.from(m.data, "base64").toString());
    const kinds = () => told().map((text) => JSON.parse(text).t);

    /** On the terminal's screen, with its page started. */
    /** Watching on one server, with a second signed in to, and the first in use. */
    async function watchingOnTwoServers() {
      told().length = 0;
      const server = new FakeServer();
      const other = new FakeServer();
      other.name = "Other Remote";
      other.server = "srv-other";
      const env: Env = {
        ...envFor(server),
        fetch: ((input: string, init?: RequestInit) =>
          (String(input).startsWith(OTHER_ADDRESS) ? other : server).fetch(
            input,
            init,
          )) as typeof fetch,
      };
      const app = await start({ server, env });
      await signIn();
      const { accounts } = app;
      await act(async () => {
        await accounts.signIn(await accounts.lookUp(OTHER_ADDRESS), "Ana’s phone", signedInBrowser);
        await accounts.activate(accounts.getSnapshot().profiles[0]?.id as string);
      });
      await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
      await fireEvent.changeText(await screen.findByLabelText("Pairing code"), "ABCD2345");
      await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
      await screen.findByText("Say yes at Workstation");
      server.answer("approved");
      await fireEvent.press(await screen.findByRole("button", { name: "Done" }, ANSWER));
      await fireEvent.press(await open());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("terminal");
      await pageSays({ t: "ready" });
      await screen.findByText(/^Watching/);
      return { ...app, server, other };
    }

    async function watching(server = new FakeServer()) {
      told().length = 0;
      const app = await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("terminal");
      await pageSays({ t: "ready" });
      await screen.findByText(/^Watching/);
      await waitFor(() => expect(outputs()).toEqual(["the screen as it is"]));
      return app;
    }

    test("what the WebView is told not to detect is a list, which Android's native side needs", async () => {
      // A single word goes through to Android's native side as it is, and aborts the whole app.
      await watching();
      expect(page().props.dataDetectorTypes).toEqual(["none"]);
    });

    test("a conversation opens its terminal, cleared to the computer's size, then what it prints", async () => {
      const { server } = await watching();
      expect(JSON.parse(told()[0] as string)).toEqual({ t: "reset", cols: 120, rows: 40 });
      await act(async () => server.print("and then this"));
      expect(outputs()).toEqual(["the screen as it is", "and then this"]);
    });

    describe("taking control", () => {
      const button = (name: string | RegExp) => screen.findByRole("button", { name });

      /** On the terminal's screen, with this phone in control. */
      async function inControl(server = new FakeServer()) {
        const app = await watching(server);
        await fireEvent.press(await button("Take control"));
        await screen.findByTestId("composer");
        return app;
      }
      const typed = (server: FakeServer) => server.inputs.map((input) => input.data);
      const type = async (text: string) => {
        await fireEvent.changeText(screen.getByTestId("composer"), text);
      };
      const sent = () => act(async () => jest.advanceTimersByTimeAsync(500));
      /** What a view has been asked about where it is, for a test to answer as the system would. */
      const measured = (view: ReturnType<typeof screen.getByTestId>) => {
        type Answer = (x: number, y: number, width: number, height: number) => void;
        type Fiber = {
          return: Fiber;
          stateNode?: { measureInWindow?: jest.Mock<(answer: Answer) => void> };
        };
        let fiber = (view as unknown as { unstable_fiber: Fiber }).unstable_fiber;
        while (!fiber.stateNode?.measureInWindow) fiber = fiber.return;
        const measure = fiber.stateNode.measureInWindow;
        return () => measure.mock.calls.map(([answer]) => answer);
      };

      test("is only ever asked for, and nothing can be typed until then", async () => {
        const { server } = await watching();
        await screen.findByText("Watching. Nobody has control.");
        expect(screen.queryByTestId("composer")).toBeNull();
        expect(server.acquires).toEqual([]);

        await fireEvent.press(await button("Take control"));
        await screen.findByText(/^You have control/);
        expect(server.acquires).toEqual([false]);
        expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
      });

      test("Send types the line and then Enter, and empties the field", async () => {
        const { server } = await inControl();
        await type("hello there");
        await fireEvent.press(await button("Send"));
        await sent();
        expect(server.inputs).toEqual([
          { data: "hello there", paste: false },
          { data: "\r", paste: false },
        ]);
        expect(screen.getByTestId("composer").props.value).toBe("");
      });

      test("Return does the same, and nothing in the field is Enter alone", async () => {
        const { server } = await inControl();
        await type("by return");
        await fireEvent(screen.getByTestId("composer"), "submitEditing");
        await sent();
        expect(typed(server)).toEqual(["by return", "\r"]);
        await fireEvent(screen.getByTestId("composer"), "submitEditing");
        await sent();
        expect(typed(server)).toEqual(["by return", "\r", "\r"]);
      });

      test("Type sends what is in the field with no Enter, and an empty field sends nothing", async () => {
        const { server } = await inControl();
        await fireEvent.press(await button("Type without Enter"));
        await sent();
        expect(typed(server)).toEqual([]);

        await type("1");
        await fireEvent.press(await button("Type without Enter"));
        await sent();
        expect(server.inputs).toEqual([{ data: "1", paste: false }]);
        expect(screen.getByTestId("composer").props.value).toBe("");
      });

      test("several lines go as one paste, for the computer to bracket", async () => {
        const { server } = await inControl();
        await type("line one\nline two");
        await fireEvent.press(await button("Send"));
        await sent();
        expect(server.inputs[0]).toEqual({ data: "line one\nline two", paste: true });
      });

      test("the keys a phone lacks go at once, with the field left alone", async () => {
        const { server } = await inControl();
        await type("half a thought");
        for (const name of ["Escape", "Tab", "Up arrow", "Left arrow", "Backspace", "Enter"]) {
          await fireEvent.press(await button(name));
          await sent();
        }
        expect(typed(server)).toEqual(["\x1b", "\t", "\x1b[A", "\x1b[D", "\x7f", "\r"]);
        expect(screen.getByTestId("composer").props.value).toBe("half a thought");
      });

      test("arrows follow what the program asked for", async () => {
        const { server } = await inControl();
        await pageSays({ t: "cursorKeys", application: true });
        await fireEvent.press(await button("Up arrow"));
        await sent();
        await pageSays({ t: "cursorKeys", application: false });
        await fireEvent.press(await button("Up arrow"));
        await sent();
        expect(typed(server)).toEqual(["\x1bOA", "\x1b[A"]);
      });

      test("Ctrl and a typed letter send the control character, once, and not into the field", async () => {
        const { server } = await inControl();
        await type("kept");
        await fireEvent.press(await screen.findByRole("togglebutton", { name: "Control" }));
        await type("keptc");
        await sent();
        expect(typed(server)).toEqual(["\x03"]);
        expect(screen.getByTestId("composer").props.value).toBe("kept");

        await type("keptc");
        expect(screen.getByTestId("composer").props.value).toBe("keptc");
        await sent();
        expect(typed(server)).toEqual(["\x03"]);
      });

      test("Ctrl with something that has no control character types it", async () => {
        const { server } = await inControl();
        await fireEvent.press(await screen.findByRole("togglebutton", { name: "Control" }));
        await type("1");
        await sent();
        expect(typed(server)).toEqual([]);
        expect(screen.getByTestId("composer").props.value).toBe("1");
      });

      test("Ctrl changes the next arrow, then lets go", async () => {
        const { server } = await inControl();
        await fireEvent.press(await screen.findByRole("togglebutton", { name: "Control" }));
        await fireEvent.press(await button("Left arrow"));
        await sent();
        await fireEvent.press(await button("Left arrow"));
        await sent();
        expect(typed(server)).toEqual(["\x1b[1;5D", "\x1b[D"]);
      });

      test("once in control it fits the room, and watching never asks", async () => {
        const { server } = await watching();
        await act(async () =>
          fireEvent(screen.getByTestId("terminal-room"), "layout", {
            nativeEvent: { layout: { x: 0, y: 0, width: 400, height: 600 } },
          }),
        );
        await pageSays({ t: "cell", width: 8, height: 16 });
        await sent();
        expect(server.resizes).toEqual([]);

        await fireEvent.press(await button("Take control"));
        await screen.findByTestId("composer");
        await sent();
        // Under a see-through header, whose 44 points cover the first rows.
        expect(server.resizes).toEqual([{ cols: 50, rows: 34 }]);

        // A keyboard coming up.
        await act(async () =>
          fireEvent(screen.getByTestId("terminal-room"), "layout", {
            nativeEvent: { layout: { x: 0, y: 0, width: 400, height: 300 } },
          }),
        );
        await sent();
        expect(server.resizes.at(-1)).toEqual({ cols: 50, rows: 16 });
      });

      test("where the room starts, answered late, does not bring back its old size", async () => {
        const { server } = await watching();
        const room = screen.getByTestId("terminal-room");
        const layout = (height: number) =>
          act(async () =>
            fireEvent(room, "layout", {
              nativeEvent: { layout: { x: 0, y: 0, width: 400, height } },
            }),
          );
        const answers = measured(room);
        await layout(600);
        await pageSays({ t: "cell", width: 8, height: 16 });
        await fireEvent.press(await button("Take control"));
        await screen.findByTestId("composer");
        await sent();
        expect(server.resizes.at(-1)).toEqual({ cols: 50, rows: 34 });

        // A keyboard coming up: two layouts, and the first's answer comes after the second's.
        await layout(450);
        await layout(300);
        const [first, second] = answers().slice(-2);
        await act(async () => {
          second?.(0, 44, 400, 300);
          first?.(0, 44, 400, 450);
        });
        await sent();
        expect(server.resizes.at(-1)).toEqual({ cols: 50, rows: 18 });
      });

      test("handing back says so to the computer, and offers it again", async () => {
        const { server } = await inControl();
        await fireEvent.press(await button("Hand back"));
        await sent();
        expect(server.releases).toBe(1);
        await button("Take control");
        expect(screen.queryByTestId("composer")).toBeNull();
      });

      test("the person at the computer taking it clears the way, and keeps what was being written", async () => {
        const { server } = await inControl();
        await type("half written");
        await act(async () => server.controlNow({ kind: "local" }));
        await screen.findByText("The person at the computer took control.");
        await screen.findByText("Watching. The person at the computer has control.");
        // Still there to send by hand later, and it cannot be sent now.
        const field = screen.getByTestId("composer");
        expect(field.props.value).toBe("half written");
        expect(field.props.editable).toBe(false);
        expect(screen.queryByRole("button", { name: "Up arrow" })).toBeNull();
        await fireEvent.press(await button("Send"));
        await sent();
        expect(typed(server)).toEqual([]);

        // Refused while they type. Taken by hand once they stop, and the text goes only then.
        server.refuseControl = true;
        await fireEvent.press(await button("Take control"));
        await screen.findByText(/is typing here/);
        server.refuseControl = false;
        await act(async () => server.controlNow({ kind: "nobody" }));
        await fireEvent.press(await button("Take control"));
        await screen.findByText(/^You have control/);
        await sent();
        expect(typed(server)).toEqual([]);
        await fireEvent.press(await button("Send"));
        await sent();
        expect(typed(server)).toEqual(["half written", "\r"]);
      });

      test("another device's control is named, and only taken over by asking", async () => {
        const { server } = await watching();
        await act(async () =>
          server.controlNow({ kind: "device", device: "device-other", name: "The tablet" }),
        );
        await screen.findByText("Watching. The tablet has control.");
        expect(server.acquires).toEqual([]);
        await fireEvent.press(await button("Take over from The tablet"));
        await screen.findByText(/^You have control/);
        expect(server.acquires).toEqual([true]);
      });

      test("another device it cannot name yet is still offered, as what it is", async () => {
        const { server } = await watching();
        await act(async () => server.controlNow({ kind: "device", device: "device-x", name: "" }));
        await screen.findByText("Watching. Another device has control.");
        await button("Take over from another device");
      });

      test("a drop on its way says it may not have arrived, sends nothing again, and takes nothing back", async () => {
        const { server } = await inControl();
        server.unanswered.add("terminal.input");
        await type("maybe arrived");
        await fireEvent.press(await button("Send"));
        await act(async () => jest.advanceTimersByTimeAsync(10));
        expect(typed(server)).toEqual(["maybe arrived"]);

        await act(async () => {
          for (const socket of [...server.relaySockets]) socket.close(1006);
          await jest.advanceTimersByTimeAsync(10);
        });
        await screen.findByText(/may not have reached the computer.*maybe arrived/);
        await screen.findByText(/connection dropped/);
        expect(screen.queryByTestId("composer")).toBeNull();

        server.unanswered.clear();
        await screen.findByText(/^Watching/, {}, { timeout: 60_000 });
        await sent();
        expect(typed(server)).toEqual(["maybe arrived"]);
        expect(server.acquires).toEqual([false]);

        // Only the person says to try again, and only into the field.
        await fireEvent.press(await button("Put it back"));
        expect(screen.queryByText(/may not have reached/)).toBeNull();
        expect(screen.getByTestId("composer").props.value).toBe("maybe arrived");
        await button("Take control");
        await sent();
        expect(typed(server)).toEqual(["maybe arrived"]);
      });

      test("what may not have arrived can be let go of, leaving the field alone", async () => {
        const { server } = await inControl();
        server.unanswered.add("terminal.input");
        await fireEvent.press(await button("Escape"));
        await act(async () => jest.advanceTimersByTimeAsync(10));
        await act(async () => {
          for (const socket of [...server.relaySockets]) socket.close(1006);
          await jest.advanceTimersByTimeAsync(10);
        });
        await screen.findByText(/may not have reached the computer.*Escape/);
        // A key has nothing to put back.
        expect(screen.queryByRole("button", { name: "Put it back" })).toBeNull();
        await fireEvent.press(await button("Dismiss"));
        expect(screen.queryByText(/may not have reached/)).toBeNull();
        expect(screen.queryByTestId("composer")).toBeNull();
      });

      test("a page started anew forgets what the last one was told about arrows", async () => {
        const { server } = await inControl();
        await pageSays({ t: "cursorKeys", application: true });
        await pageSays({ t: "ready" });
        await screen.findByText(/^You have control/);
        await fireEvent.press(await button("Up arrow"));
        await sent();
        expect(typed(server)).toEqual(["\x1b[A"]);
      });

      test("the app going to the background lets go first, and coming back does not take it again", async () => {
        const changes: ((state: string) => void)[] = [];
        jest.spyOn(AppState, "addEventListener").mockImplementation(((
          _: string,
          listener: (state: string) => void,
        ) => {
          changes.push(listener);
          return { remove: () => {} };
        }) as typeof AppState.addEventListener);
        const { server } = await inControl();
        await act(async () => changes.forEach((tell) => tell("background")));
        await sent();
        expect(server.operations.at(-1)?.op).toBe("terminal.release");
        expect(server.releases).toBe(1);

        await act(async () => changes.forEach((tell) => tell("active")));
        await screen.findByText(/^Watching/, {}, { timeout: 60_000 });
        await sent();
        expect(server.acquires).toEqual([false]);
        await button("Take control");
      });

      test("a program's end takes the field away, with whatever was in it", async () => {
        const { server } = await inControl();
        await type("never to be sent");
        await act(async () => server.programEnded({ code: 0, success: true }));
        await screen.findByText("The program ended.");
        expect(screen.queryByTestId("composer")).toBeNull();
        expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
      });
    });

    test("nothing is asked for until the page has started", async () => {
      const server = new FakeServer();
      told().length = 0;
      await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("terminal");
      await act(async () => jest.advanceTimersByTimeAsync(100));
      expect(server.attaches).toEqual([]);
      expect(told()).toEqual([]);
    });

    test("a route never carries a conversation's title", async () => {
      await watching();
      const route = screen as unknown as {
        getPathname(): string;
        getSearchParams(): Record<string, string>;
      };
      expect(route.getPathname()).toBe("/terminal");
      expect(route.getSearchParams()).toEqual({
        host: "host-work",
        terminal: "terminal-1",
        label: "claude",
      });
    });

    test("a dropped channel resumes where it was, and clears nothing", async () => {
      const { server } = await watching();
      await act(async () => server.print("before"));
      await act(async () => {
        for (const socket of [...server.relaySockets]) socket.close(1006);
      });
      await screen.findByText(/^Reconnecting/);
      await act(async () => jest.advanceTimersByTimeAsync(2_500));
      await screen.findByText(/^Watching/);
      expect(server.attaches).toEqual([null, { epoch: 1, seq: 6 }]);
      expect(kinds().filter((k) => k === "reset")).toHaveLength(1);
      await act(async () => server.print("after"));
      expect(outputs()).toEqual(["the screen as it is", "before", "after"]);
    });

    test("a new epoch replaces the screen", async () => {
      const { server } = await watching();
      await act(async () => {
        server.computer.terminalEpoch = 2;
        server.computer.terminalScreen = "a whole new screen";
        for (const socket of [...server.relaySockets]) socket.close(1006);
      });
      await act(async () => jest.advanceTimersByTimeAsync(2_500));
      await waitFor(() => expect(outputs()).toContain("a whole new screen"));
      expect(kinds().filter((k) => k === "reset")).toHaveLength(2);
    });

    test("the app in the background lets go, and resumes on coming back", async () => {
      const changes: ((state: string) => void)[] = [];
      jest.spyOn(AppState, "addEventListener").mockImplementation(((
        _type: string,
        tell: (state: string) => void,
      ) => {
        changes.push(tell);
        return { remove: () => {} };
      }) as typeof AppState.addEventListener);
      const { server } = await watching();
      await act(async () => changes.forEach((tell) => tell("background")));
      await waitFor(() => expect(server.sockets.open).toBe(0));
      await act(async () => changes.forEach((tell) => tell("active")));
      await screen.findByText(/^Watching/);
      expect(server.attaches.at(-1)).toEqual({ epoch: 1, seq: 5 });
      expect(kinds().filter((k) => k === "reset")).toHaveLength(1);
    });

    test("a page that started anew gets a new snapshot", async () => {
      const { server } = await watching();
      await pageSays({ t: "ready" });
      await waitFor(() => expect(kinds().filter((k) => k === "reset")).toHaveLength(2));
      expect(server.attaches).toEqual([null, null]);
    });

    test("a WebView's process ending starts the page again", async () => {
      const { server } = await watching();
      await act(async () => fireEvent(page(), "contentProcessDidTerminate"));
      await act(async () => server.print("nowhere to draw this"));
      expect(outputs()).not.toContain("nowhere to draw this");
      await pageSays({ t: "ready" });
      await waitFor(() => expect(server.attaches).toEqual([null, null]));
    });

    /** The page saying it drew everything it has been sent. */
    async function pageDrawsAll() {
      for (const { id } of told()
        .map((text) => JSON.parse(text))
        .filter((m) => m.t === "output")) {
        await pageSays({ t: "drawn", id });
      }
    }

    const settle = () => act(async () => jest.advanceTimersByTimeAsync(10));

    test("a snapshot followed by another, again and again, gives up and says so", async () => {
      const { server } = await watching();
      for (let i = 0; i < 8; i += 1) {
        await pageDrawsAll();
        await act(async () => server.needsSnapshot());
        await settle();
      }
      expect(await screen.findByText(/could not be followed/)).toBeTruthy();
      expect(server.attaches).toHaveLength(6);
    });

    test("falling behind goes on while the page keeps catching up with what is new", async () => {
      const { server } = await watching();
      for (let i = 0; i < 8; i += 1) {
        await act(async () => server.print("something new"));
        await pageDrawsAll();
        await act(async () => server.needsSnapshot());
        await settle();
      }
      expect(screen.queryByText(/could not be followed/)).toBeNull();
      expect(server.attaches).toHaveLength(9);
    });

    test("a page that has too much waiting starts over from a snapshot", async () => {
      const { server } = await watching();
      const chunk = "x".repeat(60_000);
      for (let i = 0; i < 80 && server.attaches.length < 2; i += 1) {
        await act(async () => server.print(chunk));
      }
      await settle();
      expect(server.attaches).toEqual([null, null]);
      expect(kinds().filter((k) => k === "reset")).toHaveLength(2);
    });

    test("a program's end is said, and watching goes no further", async () => {
      const { server } = await watching();
      await act(async () => server.programEnded({ code: 3, success: false }));
      expect(screen.getByText("The program ended with exit code 3.")).toBeTruthy();
    });

    test("an https link asks first, and goes nowhere on a no", async () => {
      await watching();
      const open = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
      const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
      await pageSays({ t: "link", url: "https://example.com/a" });
      expect(open).not.toHaveBeenCalled();
      const [, url, buttons] = alert.mock.calls.at(-1) as [string, string, AlertButton[]];
      expect(url).toBe("https://example.com/a");
      expect(buttons.map((b) => b.text)).toEqual(["Cancel", "Open"]);
      buttons.find((b) => b.text === "Cancel")?.onPress?.();
      expect(open).not.toHaveBeenCalled();
      buttons.find((b) => b.text === "Open")?.onPress?.();
      expect(open).toHaveBeenCalledWith("https://example.com/a");
    });

    test("a link to anything else is not even asked about", async () => {
      await watching();
      const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
      alert.mockClear();
      await pageSays({ t: "link", url: "javascript:alert(1)" });
      await pageSays({ t: "link", url: "http://example.com" });
      expect(alert).not.toHaveBeenCalled();
    });

    test("the page goes nowhere else", async () => {
      await watching();
      const go = page().props.onShouldStartLoadWithRequest as (r: { url: string }) => boolean;
      expect(go({ url: "about:blank" })).toBe(true);
      for (const url of ["https://example.com", "file:///etc/passwd", "yardsort://pair"]) {
        expect(go({ url })).toBe(false);
      }
      expect(page().props.originWhitelist).toEqual(["about:blank"]);
      expect(page().props.incognito).toBe(true);
      expect(page().props.allowFileAccess).toBe(false);
    });

    test("an ended conversation with no terminal has nothing to open", async () => {
      const server = new FakeServer();
      const first = server.computer.sessions[0] as Record<string, unknown>;
      first.terminal = null;
      await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await screen.findByText("The login page times out");
      expect(screen.queryByRole("button", { name: "The login page times out" })).toBeNull();
    });

    test("a computer that took the phone back takes the terminal with it", async () => {
      const { server } = await watching();
      server.grants[0]!.state = "revoked";
      await act(async () => {
        for (const socket of [...server.relaySockets]) socket.close(4003);
      });
      expect(await screen.findByText("This computer no longer lets this phone in.")).toBeTruthy();
      expect(screen.queryByTestId("terminal")).toBeNull();
    });

    test("a phone revoked while watching is signed out, and nothing of the terminal stays", async () => {
      const { server, vault, accounts } = await watching();
      server.revoke(accounts.getSnapshot().active?.device as string);
      await act(async () => server.closeChannelsTo("host-work", 4003));

      expect(await screen.findByText(/This phone was signed out of Example Remote/)).toBeTruthy();
      expect(screen.getByText("Connect to a server")).toBeTruthy();
      expect(screen.queryByTestId("terminal")).toBeNull();
      expect(screen.queryByText("This computer no longer lets this phone in.")).toBeNull();
      expect(vault.credentials()).toEqual([]);
    });

    test("a refusal the server cannot explain leaves the terminal, and it comes back", async () => {
      const { server, vault } = await watching();
      server.down = true;
      await act(async () => server.closeChannelsTo("host-work", 4003));

      expect(await screen.findByText(/^Cannot be reached\. What is on the computer/)).toBeTruthy();
      expect(screen.getByTestId("terminal")).toBeTruthy();
      expect(screen.queryByText("This computer no longer lets this phone in.")).toBeNull();
      expect(vault.credentials()).toHaveLength(1);

      server.down = false;
      expect(await screen.findByText(/^Watching/, undefined, ANSWER)).toBeTruthy();
    });

    test("a screen left behind by switching servers shows nothing of the first", async () => {
      const { accounts, server } = await watchingOnTwoServers();
      await act(async () => accounts.activate(accounts.getSnapshot().profiles[1]?.id as string));

      expect(await screen.findByText("Signed in to Other Remote as Ana’s phone.")).toBeTruthy();
      expect(screen.queryByTestId("terminal")).toBeNull();
      expect(screen.queryByText("Signed in to Example Remote as Ana’s phone.")).toBeNull();
      await waitFor(() => expect(server.sockets.open).toBe(0));
    });

    test("an address that now answers as another server takes the terminal with it", async () => {
      const { server, accounts } = await watching();
      server.server = "srv-replacement";
      server.name = "Replacement";
      server.refuseAccess = true;
      await act(async () => {
        await accounts.computers().catch(() => {});
      });

      expect(await screen.findByText(/now answers as another server/)).toBeTruthy();
      expect(screen.queryByTestId("terminal")).toBeNull();
    });
  });

  describe("no longer reached", () => {
    for (const [where, go] of [
      ["a computer", async () => {}],
      [
        "a workspace",
        async () => {
          await fireEvent.press(await screen.findByRole("button", { name: /^fix-login/ }));
          await screen.findByText("The login page times out");
        },
      ],
      [
        "approved memory",
        async () => {
          await fireEvent.press(await screen.findByRole("button", { name: /^fix-login/ }));
          await fireEvent.press(await screen.findByRole("button", { name: "Approved memory" }));
          await screen.findByText("Migrations are append-only.");
        },
      ],
    ] as const) {
      test(`a phone revoked while on ${where} is signed out, and nothing of it stays`, async () => {
        const { server, vault, accounts } = await paired();
        await open();
        await go();
        server.revoke(accounts.getSnapshot().active?.device as string);
        await act(async () => server.closeChannelsTo("host-work", 4003));

        expect(await screen.findByText(/This phone was signed out of Example Remote/)).toBeTruthy();
        expect(screen.getByText("Connect to a server")).toBeTruthy();
        expect(screen.queryByText("fix-login")).toBeNull();
        expect(screen.queryByText("Migrations are append-only.")).toBeNull();
        expect(screen.queryByText(/no longer lets this phone in/)).toBeNull();
        expect(vault.credentials()).toEqual([]);
      });
    }

    test("taken off the account says so, and not that it took the phone back", async () => {
      const { server, shelf } = await paired();
      await open();
      await waitFor(() => expect([...shelf.kept.values()].join()).toContain("fix-login"));
      server.hosts = [];
      await act(async () => server.closeChannelsTo("host-work", 4003));

      expect(await screen.findByText("This computer is no longer on this account.")).toBeTruthy();
      expect(screen.queryByText("This computer no longer lets this phone in.")).toBeNull();
      expect(screen.queryByRole("button", { name: /^fix-login/ })).toBeNull();
      await waitFor(() => expect([...shelf.kept.values()].join()).not.toContain("fix-login"));

      await fireEvent.press(screen.getByRole("button", { name: "Back to the account" }));
      expect(await screen.findByText(/^Workstation is no longer on this account\./)).toBeTruthy();
      expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
    });

    test("a refusal the server cannot explain shows what was seen, and deletes nothing", async () => {
      const { server, shelf, vault } = await paired();
      await open();
      await waitFor(() => expect([...shelf.kept.values()].join()).toContain("fix-login"));
      server.down = true;
      await act(async () => server.closeChannelsTo("host-work", 4003));

      expect(await screen.findByText("Cannot be reached")).toBeTruthy();
      expect(screen.getByText(/the server could not say why/)).toBeTruthy();
      expect(screen.getByRole("button", { name: /^fix-login/ })).toBeTruthy();
      expect([...shelf.kept.values()].join()).toContain("fix-login");
      expect(vault.credentials()).toHaveLength(1);
    });

    test("one computer taking the phone back leaves the other working", async () => {
      const { server, vault } = await paired();
      const grant = server.grants[0] as (typeof server.grants)[number];
      server.hosts.push({ ...(server.hosts[0] as (typeof server.hosts)[number]), id: "host-home" });
      (server.hosts[1] as { name: string }).name = "Laptop";
      server.grants.push({ ...grant, id: "grant-home", host: "host-home", hostName: "Laptop" });
      await act(async () => screen.getByTestId("screen").props.refreshControl.props.onRefresh());
      await screen.findByLabelText("Laptop, Online");
      await screen.findByLabelText(/^Workstation, Online/);

      grant.state = "revoked";
      await act(async () => server.closeChannelsTo("host-work", 4003));

      expect(await screen.findByText(/^Workstation no longer lets this phone in\./)).toBeTruthy();
      await waitFor(() =>
        expect(screen.queryByRole("button", { name: /^Workstation, / })).toBeNull(),
      );
      expect(screen.getByLabelText("Laptop, Online")).toBeTruthy();
      expect(server.sockets.open).toBe(1);
      expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
      expect(vault.credentials()).toHaveLength(1);

      await fireEvent.press(screen.getByRole("button", { name: "OK" }));
      expect(screen.queryByText(/^Workstation no longer lets/)).toBeNull();
      expect(screen.getByLabelText("Laptop, Online")).toBeTruthy();
    });

    test("found on opening the app, it is told there too", async () => {
      const { server, shelf, vault } = await paired();
      server.grants[0]!.state = "revoked";
      await start({ server, vault, env: { ...envFor(server), shelf } });
      expect(await screen.findByText(/^Workstation no longer lets this phone in\./)).toBeTruthy();
    });

    test("unpairing here tells nobody what they just did", async () => {
      await paired();
      await fireEvent.press(await screen.findByLabelText(/^Workstation, Online/));
      await fireEvent.press(await screen.findByRole("button", { name: "Unpair Workstation" }));
      await answerAlert("Unpair");
      await screen.findByText(/No computer lets this phone in yet/);
      expect(screen.queryByText(/^Workstation no longer lets/)).toBeNull();
    });
  });

  describe("what was last seen", () => {
    test("shows when the computer is offline, marked as such", async () => {
      const { server } = await paired();
      await open();
      await act(async () => router.dismissTo("/account"));
      server.offline.add("host-work");
      await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
      expect(await screen.findByText(/^Offline, last seen/)).toBeTruthy();
      expect(screen.getByText("Offline does not mean its agents stopped.")).toBeTruthy();
      expect(await screen.findByText(/^As of /)).toBeTruthy();
      expect(screen.getByRole("button", { name: "fix-login, ys/fix-login" })).toBeTruthy();
    });

    test("holds names and branches, and never a title, activity or memory", async () => {
      const { shelf } = await paired();
      await fireEvent.press(await open());
      await screen.findByText("The login page times out");
      await fireEvent.press(screen.getByRole("button", { name: "Approved memory" }));
      await screen.findByText("Migrations are append-only.");

      const kept = [...shelf.kept.values()].join();
      expect(kept).toContain("fix-login");
      expect(kept).not.toContain("The login page times out");
      expect(kept).not.toContain("Migrations are append-only");
      expect(kept).not.toContain("process.started");
      expect(kept).not.toContain("a-model");
    });

    test("shows before the server answers on a cold start", async () => {
      const { shelf, vault, server } = await paired();
      await open();
      await act(async () => router.dismissTo("/account"));
      server.down = true;
      await start({ server, vault, env: { ...envFor(server), shelf } });
      expect(await screen.findByText(/could not be reached/)).toBeTruthy();
      const row = await screen.findByRole("button", { name: /^Workstation, As of / });
      await fireEvent.press(row);
      expect(await screen.findByRole("button", { name: "fix-login, ys/fix-login" })).toBeTruthy();
      expect(screen.getByText(/^As of /)).toBeTruthy();
    });

    test("goes when the computer takes this phone back, and the phone stays signed in", async () => {
      const { shelf, server } = await paired();
      await open();
      await waitFor(() => expect([...shelf.kept.values()].join()).toContain("fix-login"));
      server.grants[0]!.state = "revoked";
      await act(async () => {
        for (const socket of server.relaySockets) socket.onclose?.({ code: 4003, reason: "" });
      });
      expect(await screen.findByText("This computer no longer lets this phone in.")).toBeTruthy();
      await waitFor(() => expect([...shelf.kept.values()].join()).not.toContain("fix-login"));
      expect(screen.queryByRole("button", { name: /^fix-login/ })).toBeNull();
      await fireEvent.press(screen.getByRole("button", { name: "Back to the account" }));
      expect(await screen.findByRole("button", { name: "Sign out" })).toBeTruthy();
    });

    test("goes with the profile", async () => {
      const { shelf, accounts } = await paired();
      await open();
      await waitFor(() => expect(shelf.kept.size).toBeGreaterThan(0));
      await act(async () => accounts.signOut());
      await waitFor(() => expect(shelf.kept.size).toBe(0));
    });
  });
});
