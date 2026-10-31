// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The screens, through Expo Router's own in-memory router and Testing Library: what a person sees
// and presses. The server behind them is a small fake (`fake-server.ts`); the logic underneath is
// tested against the real server in `test/lib/`.

import { afterEach, beforeEach, describe, expect, jest, test } from "@jest/globals";
import { Stack } from "expo-router";
import { act, fireEvent, renderRouter, screen, waitFor } from "expo-router/testing-library";
import { Alert, type AlertButton } from "react-native";

import Account from "../../src/app/account";
import Connect from "../../src/app/connect";
import Start from "../../src/app/index";
import Servers from "../../src/app/servers";
import { Accounts, type Browser } from "../../src/lib/accounts";
import { AppProvider } from "../../src/lib/context";
import { MemoryVault } from "../lib/memory-vault";
import { ADDRESS, envFor, FakeServer } from "./fake-server";

const signedInBrowser: Browser = async (_url, redirect) => `${redirect}?code=a-code`;

async function start({
  server = new FakeServer(),
  vault = new MemoryVault(),
  browser = signedInBrowser,
}: { server?: FakeServer; vault?: MemoryVault; browser?: Browser } = {}) {
  const accounts = new Accounts(envFor(server), vault);
  const app = { accounts, browser, deviceName: "iPhone" };
  void accounts.load();
  await renderRouter(
    {
      _layout: () => (
        <AppProvider value={app}>
          <Stack />
        </AppProvider>
      ),
      index: Start,
      connect: Connect,
      account: Account,
      servers: Servers,
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

beforeEach(() => {
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
});

afterEach(() => {
  jest.restoreAllMocks();
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
});
