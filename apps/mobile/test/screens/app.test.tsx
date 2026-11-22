// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The screens, through Expo Router's own in-memory router and Testing Library: what a person sees
// and presses. The server behind them is a small fake (`fake-server.ts`); the logic underneath is
// tested against the real server in `test/lib/`.

import { afterEach, beforeEach, describe, expect, jest, test } from "@jest/globals";
import { Secret } from "@yardsort/remote-client";
import { router, Stack } from "expo-router";
import { act, fireEvent, renderRouter, screen, waitFor, within } from "expo-router/testing-library";
import {
  Alert,
  type AlertButton,
  AppState,
  DeviceEventEmitter,
  Linking,
  Share,
  StyleSheet,
} from "react-native";

import Account from "../../src/app/account";
import ComputerScreen from "../../src/app/computer";
import Connect from "../../src/app/connect";
import DiagnosticsScreen from "../../src/app/diagnostics";
import Start from "../../src/app/index";
import MemoryScreen from "../../src/app/memory";
import Notice from "../../src/app/notice/[id]";
import TerminalScreen from "../../src/app/terminal";
import Pair from "../../src/app/pair";
import Servers from "../../src/app/servers";
import Settings from "../../src/app/settings";
import ExportScreen from "../../src/app/export";
import DeleteAccountScreen from "../../src/app/delete-account";
import StartWork from "../../src/app/start";
import PullScreen from "../../src/app/pull";
import PullsScreen from "../../src/app/pulls";
import WorkspaceScreen from "../../src/app/workspace";
import { Accounts, type Browser, type Env } from "../../src/lib/accounts";
import type { PickImage } from "../../src/lib/attach";
import { AppProvider, type SaveFile } from "../../src/lib/context";
import { Diagnostics } from "../../src/lib/diagnostics";
import { folds } from "../../src/lib/folds";
import { Incoming } from "../../src/lib/pairing";
import { viewChoices } from "../../src/lib/views";
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
  pickImage = async () => null,
  saveFile = async () => true,
  diagnostics = new Diagnostics(),
}: {
  server?: FakeServer;
  vault?: MemoryVault;
  browser?: Browser;
  env?: Env;
  incoming?: Incoming;
  pickImage?: PickImage;
  saveFile?: SaveFile;
  diagnostics?: Diagnostics;
} = {}) {
  const accounts = new Accounts({ diagnostics, ...env }, vault);
  const app = {
    accounts,
    browser,
    deviceName: "iPhone",
    incoming,
    pickImage,
    saveFile,
    diagnostics,
    about: { app: "0.1.5", system: "ios 18.1" },
  };
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
      export: ExportScreen,
      "delete-account": DeleteAccountScreen,
      pair: Pair,
      computer: ComputerScreen,
      pull: PullScreen,
      pulls: PullsScreen,
      workspace: WorkspaceScreen,
      memory: MemoryScreen,
      terminal: TerminalScreen,
      start: StartWork,
      diagnostics: DiagnosticsScreen,
      "notice/[id]": Notice,
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

/** The servers on this phone are reached through Settings, not from the screen beneath it. */
async function openServers() {
  expect(
    within(screen.getByTestId("screen")).queryByRole("button", { name: "Servers on this phone" }),
  ).toBeNull();
  await fireEvent.press(await screen.findByRole("button", { name: "Settings" }));
  await fireEvent.press(await screen.findByRole("button", { name: "Servers on this phone" }));
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
  // Which projects are folded is the app's, and each test's alone.
  folds.start({ read: () => null, write: () => {} });
});

afterEach(() => {
  jest.restoreAllMocks();
});

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
    await openServers();
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

describe("Exporting the account", () => {
  async function onExport(saveFile: SaveFile, server = new FakeServer()) {
    const app = await start({ server, saveFile });
    await signIn();
    await act(async () => router.push("/settings"));
    await fireEvent.press(await screen.findByRole("button", { name: "Export my account’s data" }));
    await screen.findByText("Never a credential. Keep the file somewhere only you can read.");
    return app;
  }

  test("says what the file holds, then saves every page of it, and no credential", async () => {
    const saved: { name: string; text: string }[] = [];
    const { server } = await onExport(async (name, text) => {
      saved.push({ name, text });
      return true;
    });
    expect(server.asked).not.toContain("GET /v1/account/export");
    await fireEvent.press(screen.getByRole("button", { name: "Export" }));
    await screen.findByText(/^Saved yardsort-account-\d{4}-\d{2}-\d{2}\.json: 3 synced records/);
    expect(saved).toHaveLength(1);
    const file = JSON.parse(saved[0]!.text);
    expect(file.format).toBe(1);
    expect(file.serverName).toBe("Example Remote");
    expect(file.account.subject).toBe("ana");
    expect(file.records.map((r: { id: string }) => r.id)).toEqual(["a", "b", "c"]);
    expect(saved[0]!.text).not.toMatch(/access-|refresh-/);
  });

  test("a save the person backed out of says nothing, and an older server says why", async () => {
    const server = new FakeServer();
    await onExport(async () => false, server);
    await fireEvent.press(screen.getByRole("button", { name: "Export" }));
    await waitFor(() => expect(server.asked).toContain("GET /v1/account/export"));
    expect(screen.queryByText(/^Saved /)).toBeNull();

    server.exports = false;
    await fireEvent.press(screen.getByRole("button", { name: "Export" }));
    await screen.findByText(
      "Example Remote cannot export an account yet. Its operator has to update it.",
    );
  });
});

describe("Deleting the account", () => {
  async function onDelete(server = new FakeServer(), browser: Browser = signedInBrowser) {
    const opened: string[] = [];
    let signedIn = false;
    const app = await start({
      server,
      browser: async (url, redirect) => {
        if (!signedIn) return signedInBrowser(url, redirect);
        opened.push(url);
        return browser(url, redirect);
      },
    });
    await signIn();
    signedIn = true;
    await act(async () => router.push("/settings"));
    await fireEvent.press(await screen.findByRole("button", { name: "Delete my account" }));
    await screen.findByText("• every synced project, note and workspace name");
    return { ...app, opened };
  }

  test("says what goes and what stays, then signs in again and deletes, and the phone forgets it", async () => {
    const { server, opened, vault, accounts } = await onDelete();
    expect(screen.getByText(/Everything on your computers/)).toBeTruthy();
    const go = screen.getByRole("button", { name: "Sign in again and delete" });
    expect(go).toBeDisabled();
    await fireEvent.changeText(screen.getByLabelText("Type delete to confirm"), "delet");
    expect(go).toBeDisabled();
    expect(opened).toHaveLength(0);
    await fireEvent.changeText(screen.getByLabelText("Type delete to confirm"), "Delete");
    await fireEvent.press(go);
    await screen.findByText(
      "Your account at Example Remote was deleted, and every device signed out. Everything on your computers is still there.",
    );
    expect(new URL(opened[0]!).searchParams.get("purpose")).toBe("confirm");
    expect(server.deletions).toEqual([{ code: "a-code", verifier: expect.any(String) }]);
    expect(accounts.getSnapshot().profiles).toHaveLength(0);
    expect([...vault.kept.values()].join("\n")).not.toContain("refresh-");
  });

  test("closing the browser deletes nothing, and an older server says why first", async () => {
    const server = new FakeServer();
    await onDelete(server, async () => null);
    await fireEvent.changeText(screen.getByLabelText("Type delete to confirm"), "delete");
    await fireEvent.press(screen.getByRole("button", { name: "Sign in again and delete" }));
    await screen.findByText("You did not sign in again, so nothing was deleted.");
    expect(server.deletions).toHaveLength(0);

    server.deletes = false;
    await fireEvent.press(screen.getByRole("button", { name: "Sign in again and delete" }));
    await screen.findByText(
      "Example Remote cannot delete an account yet. Its operator has to update it.",
    );
    expect(server.deletions).toHaveLength(0);
  });

  test("deleted from another device, the phone says so when it next asks", async () => {
    const server = new FakeServer();
    await start({ server });
    await signIn();
    server.deleted = true;
    await act(async () => router.push("/settings"));
    await fireEvent.press(await screen.findByRole("button", { name: "Export my account’s data" }));
    await fireEvent.press(await screen.findByRole("button", { name: "Export" }));
    await screen.findByText(
      "Your account at Example Remote was deleted, so this phone is signed out. Everything on your computers is still there.",
    );
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
    await openServers();
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
    await openServers();
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

  /** Settings is a gear in the header's corner, not a button at the foot of the screen. */
  async function openSettingsFromTheCorner() {
    const corner = await screen.findByRole("button", { name: "Settings" });
    expect(
      within(screen.getByTestId("screen")).queryByRole("button", { name: "Settings" }),
    ).toBeNull();
    await fireEvent.press(corner);
  }

  test("settings can be reached before there is any server", async () => {
    await start();
    await openSettingsFromTheCorner();
    expect(await screen.findByRole("radio", { name: "Dark" })).toBeTruthy();
    // With no server yet, there are none to list.
    expect(screen.queryByRole("button", { name: "Servers on this phone" })).toBeNull();
  });

  test("settings can be reached signed in", async () => {
    await start();
    await signIn();
    await openSettingsFromTheCorner();
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
  async function paired(
    shelf = new MemoryVault(),
    server = new FakeServer(),
    pickImage?: PickImage,
  ) {
    const app = await start({
      server,
      env: { ...envFor(server), shelf },
      ...(pickImage ? { pickImage } : {}),
    });
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

  test("a project's heading folds its workspaces away, and says that agents run inside", async () => {
    await paired();
    await open();
    const heading = screen.getByRole("button", { name: "demo, 1" });
    expect(heading.props.accessibilityState.expanded).toBe(true);

    await fireEvent.press(heading);
    expect(screen.queryByRole("button", { name: /^fix-login/ })).toBeNull();
    const folded = screen.getByRole("button", { name: "demo, 1, 1 running" });
    expect(folded.props.accessibilityState.expanded).toBe(false);

    await fireEvent.press(folded);
    expect(screen.getByRole("button", { name: /^fix-login/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: "demo, 1" })).toBeTruthy();
  });

  test("a folded project is still folded when the computer is opened again", async () => {
    await paired();
    await open();
    await fireEvent.press(screen.getByRole("button", { name: "demo, 1" }));
    await act(async () => router.back());

    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
    expect(await screen.findByRole("button", { name: "demo, 1, 1 running" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^fix-login/ })).toBeNull();
  });

  describe("pull requests", () => {
    const button = (name: string | RegExp) => screen.findByRole("button", { name });
    /** One with a failed check, changes asked for and conflicts, from the workspace. */
    const failing = () => ({
      project: "project-1",
      number: 61,
      title: "Retry uploads with backoff",
      url: "https://github.com/orchard/orchard/pull/61",
      branch: "ys/fix-login",
      base: "main",
      state: "open",
      draft: false,
      checks: { passed: 4, failed: 2, running: 0 },
      review: "changesRequested",
      conflicts: true,
      additions: 184,
      deletions: 22,
      author: "juno-p",
      updatedAt: Date.now() - 5 * 60_000,
      workspaces: ["workspace-1"],
    });
    /** A draft in another repository, which the workspace follows. */
    const followed = () => ({
      ...failing(),
      number: 7,
      followed: "follow-1",
      repo: "orchard/lib",
      title: "Accept knots in parse()",
      url: "https://github.com/orchard/lib/pull/7",
      branch: "knots",
      draft: true,
      checks: { passed: 2, failed: 0, running: 3 },
      review: "none",
      conflicts: false,
    });
    /** A computer that reads pull requests, with these. */
    function reading(...pulls: Record<string, unknown>[]) {
      const server = new FakeServer();
      server.computer.capabilities = ["pull.list", "pull.describe"];
      server.computer.pulls = pulls;
      return server;
    }
    const asked = (server: FakeServer, op: string) =>
      server.operations.filter((o) => o.op === op).map((o) => o.params);

    test("a workspace's row carries its pull request, in words as well as colour, and how many more it has", async () => {
      const server = reading(failing(), followed());
      await paired(new MemoryVault(), server);
      await open();
      expect(await screen.findByText("#61 +1")).toBeTruthy();
      expect(screen.getByRole("button", { name: /^fix-login/ }).props.accessibilityLabel).toBe(
        "fix-login, ys/fix-login · 1 running · pull request #61, needs attention",
      );
      expect(
        screen.getByRole("button", {
          name: "Show pull request #61, needs attention, and 1 more",
        }),
      ).toBeTruthy();
      expect(screen.getByRole("button", { name: "Pull requests · 2 open" })).toBeTruthy();
      // What a workspace list shows, and nothing a list of them all would add.
      expect(asked(server, "pull.list")[0]).toMatchObject({ of: "workspaces" });
    });

    test("a merged one says so in place of its number", async () => {
      await paired(new MemoryVault(), reading({ ...failing(), state: "merged", conflicts: false }));
      await open();
      expect(await screen.findByText("merged")).toBeTruthy();
      expect(screen.getByRole("button", { name: /^fix-login/ }).props.accessibilityLabel).toMatch(
        /pull request #61, merged$/,
      );
      // None is open: the way to the list is there, without a count.
      expect(screen.getByRole("button", { name: "Pull requests" })).toBeTruthy();
    });

    test("a computer that does not tell of pull requests is not asked, and shows nothing of them", async () => {
      const server = new FakeServer();
      server.computer.pulls = [failing()];
      await paired(new MemoryVault(), server);
      const row = await open();
      expect(row.props.accessibilityLabel).toBe("fix-login, ys/fix-login · 1 running");
      expect(screen.queryByRole("button", { name: /^Pull requests/ })).toBeNull();
      await fireEvent.press(row);
      await screen.findByText("The login page times out");
      expect(screen.queryByText("Pull request")).toBeNull();
      expect(asked(server, "pull.list")).toEqual([]);
      expect(asked(server, "pull.describe")).toEqual([]);
    });

    test("a computer still reading them is asked again in a few seconds, and then each minute", async () => {
      const server = reading();
      server.computer.pullsReading = 1;
      await paired(new MemoryVault(), server);
      await open();
      await waitFor(() => expect(asked(server, "pull.list")).toHaveLength(1));
      expect(screen.queryByRole("button", { name: /^Pull requests/ })).toBeNull();

      // What it read arrives at the computer while the phone waits.
      server.computer.pulls = [failing()];
      await act(async () => jest.advanceTimersByTimeAsync(4_100));
      expect(await screen.findByText("#61")).toBeTruthy();
      expect(asked(server, "pull.list")).toHaveLength(2);

      // A check finishes without anyone pressing anything.
      server.computer.pulls = [
        {
          ...failing(),
          checks: { passed: 6, failed: 0, running: 0 },
          review: "approved",
          conflicts: false,
        },
      ];
      await act(async () => jest.advanceTimersByTimeAsync(30_000));
      expect(asked(server, "pull.list")).toHaveLength(2);
      await act(async () => jest.advanceTimersByTimeAsync(31_000));
      await waitFor(() =>
        expect(screen.getByRole("button", { name: /^fix-login/ }).props.accessibilityLabel).toMatch(
          /pull request #61, open$/,
        ),
      );
    });

    test("a workspace shows its pull request's card, each check by name, and opens its page on the forge", async () => {
      const server = reading(failing(), followed());
      server.computer.pullDetails["#61"] = {
        checks: [
          { name: "CI / test (ubuntu)", state: "failed" },
          { name: "CI / test (windows)", state: "failed" },
          { name: "CI / build", state: "passed" },
        ],
        moreChecks: 3,
        reviews: [{ login: "mira-k", state: "changesRequested" }],
        waitingOn: [],
      };
      await paired(new MemoryVault(), server);
      await fireEvent.press(await open());

      const card = await screen.findByLabelText("Pull request #61");
      expect(within(card).getByText("Retry uploads with backoff")).toBeTruthy();
      expect(within(card).getByText("Open")).toBeTruthy();
      expect(within(card).getByText("updated 5 min ago")).toBeTruthy();
      expect(within(card).getByText("ys/fix-login → main")).toBeTruthy();
      expect(within(card).getByLabelText("184 lines added, 22 removed")).toBeTruthy();
      expect(within(card).getByText("4 of 6 checks passed, 2 failed")).toBeTruthy();
      // Something failed, so its checks are open without a press: the failed ones lead.
      expect(await within(card).findByText("CI / test (ubuntu)")).toBeTruthy();
      expect(within(card).getAllByText("failed")).toHaveLength(2);
      expect(within(card).getByText("and 3 more")).toBeTruthy();
      expect(within(card).getByText("Changes requested by mira-k")).toBeTruthy();
      expect(within(card).getByText("Conflicts with main")).toBeTruthy();
      expect(asked(server, "pull.describe").at(-1)).toEqual({ project: "project-1", number: 61 });

      await fireEvent.press(within(card).getByRole("button", { name: "Hide the checks" }));
      expect(within(card).queryByText("CI / build")).toBeNull();
      await fireEvent.press(within(card).getByRole("button", { name: "Show the checks" }));
      expect(within(card).getByText("CI / build")).toBeTruthy();

      const openURL = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
      await fireEvent.press(within(card).getByRole("button", { name: "Open on GitHub" }));
      expect(openURL).toHaveBeenCalledWith("https://github.com/orchard/orchard/pull/61");
      // Left as it was found: a later test counts what is opened.
      openURL.mockClear();
      openURL.mockRestore();

      // The one it follows in another repository is listed under it, and opens as its own card.
      expect(screen.getByText("Also opened from this workspace")).toBeTruthy();
      const other = screen.getByRole("button", {
        name: "Accept knots in parse(), pull request lib#7, a draft",
      });
      expect(within(other).getByText("orchard/lib · checks running")).toBeTruthy();
      expect(within(other).getByText("lib#7")).toBeTruthy();
      await fireEvent.press(other);
      const second = await screen.findByLabelText("Pull request lib#7");
      expect(within(second).getByText("Draft")).toBeTruthy();
      expect(within(second).getByText("2 of 5 checks passed, 3 still running")).toBeTruthy();
      expect(asked(server, "pull.describe").at(-1)).toEqual({
        project: "project-1",
        number: 7,
        followed: "follow-1",
      });
      expect(await screen.findByText("Followed from")).toBeTruthy();
    });

    test("a link that is not to a web page is no link, and a card has no way out by it", async () => {
      const server = reading({ ...failing(), url: "intent://evil#Intent;end" });
      await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      const card = await screen.findByLabelText("Pull request #61");
      expect(within(card).getByText("Conflicts with main")).toBeTruthy();
      expect(within(card).queryByRole("button", { name: /^Open / })).toBeNull();
    });

    describe("merging one from the phone (48)", () => {
      const HEAD = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
      /** Its checks passed and its review is in: nothing stands in the way. */
      const ready = () => ({
        ...failing(),
        checks: { passed: 6, failed: 0, running: 0 },
        review: "approved",
        conflicts: false,
      });
      /** A computer that merges, with the workspace's pull request at HEAD. */
      function merging(...pulls: Record<string, unknown>[]) {
        const server = reading(...pulls);
        server.computer.capabilities = [
          "pull.list",
          "pull.describe",
          "workspace.publish",
          "pull_request.merge",
        ];
        server.computer.publish = {
          branch: "ys/fix-login",
          base: "main",
          gh: true,
          pullRequest: {
            number: 61,
            url: "https://github.com/orchard/orchard/pull/61",
            title: "Retry uploads with backoff",
            state: "open",
            checks: "passed",
            headOid: HEAD,
            // What the forge says just now, which the card goes by.
            ...(pulls[0]?.draft ? { draft: true } : {}),
            ...(pulls[0]?.conflicts ? { conflicting: true } : {}),
          },
        };
        return server;
      }

      test("asks first, naming the commit, and the computer merges exactly that one", async () => {
        const server = merging(ready());
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        const card = await screen.findByLabelText("Pull request #61");
        await fireEvent.press(await within(card).findByRole("button", { name: "Merge…" }));
        // The workspace's standing was asked of the forge, for the commit at its head.
        expect(asked(server, "workspace.publish").at(-1)).toEqual({
          workspace: "workspace-1",
          forge: true,
        });

        expect(await screen.findByText("Merge #61?")).toBeTruthy();
        expect(screen.getByText("a1b2c3d · the commit shown now")).toBeTruthy();
        expect(screen.getByText("One commit on main, with the pull request's title.")).toBeTruthy();
        // All its checks passed: no warning.
        expect(screen.queryByText(/The forge decides whether it may merge/)).toBeNull();
        await fireEvent.press(screen.getByRole("radio", { name: "Rebase" }));
        expect(screen.getByRole("button", { name: "Rebase and merge" })).toBeTruthy();
        await fireEvent.press(screen.getByRole("radio", { name: "Squash" }));
        expect(asked(server, "pull_request.merge")).toEqual([]);

        await fireEvent.press(screen.getByRole("button", { name: "Squash and merge" }));
        expect(
          await screen.findByText(
            "Squashed into main at a1b2c3d from this phone. The branch ys/fix-login is kept.",
          ),
        ).toBeTruthy();
        expect(asked(server, "pull_request.merge")).toEqual([
          { workspace: "workspace-1", number: 61, headOid: HEAD, method: "squash" },
        ]);
        // Read again: it is merged now, and there is nothing left to merge.
        expect(await within(card).findByText("Merged")).toBeTruthy();
        expect(within(card).queryByRole("button", { name: "Merge…" })).toBeNull();
      });

      test("Cancel sends nothing", async () => {
        const server = merging(ready());
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        await fireEvent.press(await screen.findByRole("button", { name: "Merge…" }));
        await fireEvent.press(await screen.findByRole("button", { name: "Cancel" }));
        expect(asked(server, "pull_request.merge")).toEqual([]);
      });

      test("checks that did not all pass warn in the sheet, and do not stop it", async () => {
        const server = merging({ ...ready(), checks: { passed: 5, failed: 0, running: 1 } });
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        await fireEvent.press(await screen.findByRole("button", { name: "Merge…" }));
        expect(
          await screen.findByText(
            "1 check is still running. The forge decides whether it may merge.",
          ),
        ).toBeTruthy();
        expect(screen.getByRole("button", { name: "Squash and merge" })).toBeTruthy();
      });

      test("a refusal is the computer's own sentence, and is not sent again", async () => {
        const server = merging(ready());
        const said =
          "The pull request changed or is no longer ready to merge. Read it again and look before merging.";
        server.computer.refuses.set("pull_request.merge", said);
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        await fireEvent.press(await screen.findByRole("button", { name: "Merge…" }));
        await fireEvent.press(await screen.findByRole("button", { name: "Squash and merge" }));
        expect(await screen.findByText(said)).toBeTruthy();
        expect(asked(server, "pull_request.merge")).toHaveLength(1);
      });

      test("conflicts show Merge… that cannot be pressed, and say why", async () => {
        const server = merging({ ...ready(), conflicts: true });
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        const card = await screen.findByLabelText("Pull request #61");
        expect(
          await within(card).findByText(
            "Conflicts with main. Resolve them in the workspace first.",
          ),
        ).toBeTruthy();
        const merge = within(card).getByRole("button", { name: "Merge…" });
        expect(merge.props.accessibilityState.disabled).toBe(true);
      });

      test("a draft says so, and a computer that does not merge offers nothing", async () => {
        const server = merging({ ...ready(), draft: true });
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        expect(
          await screen.findByText("It is a draft. Mark it ready for review on the forge first."),
        ).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Merge…" })).toBeNull();
      });

      test("a computer from before merging is not asked where the workspace stands", async () => {
        const server = reading(ready());
        await paired(new MemoryVault(), server);
        await fireEvent.press(await open());
        await screen.findByLabelText("Pull request #61");
        expect(screen.queryByRole("button", { name: "Merge…" })).toBeNull();
        expect(asked(server, "workspace.publish")).toEqual([]);
      });
    });

    test("pressing a row's badge opens that pull request, with the workspace it came from", async () => {
      const server = reading(failing());
      await paired(new MemoryVault(), server);
      await open();
      await fireEvent.press(await button("Show pull request #61, needs attention"));
      const card = await screen.findByLabelText("Pull request #61");
      expect(within(card).getByText("Retry uploads with backoff")).toBeTruthy();
      expect(await screen.findByText("Opened from")).toBeTruthy();
      await fireEvent.press(await button("fix-login"));
      expect(await screen.findByText("The login page times out")).toBeTruthy();
    });

    test("the list of them all, by state, and one opens as its card", async () => {
      const merged = {
        ...failing(),
        number: 39,
        title: "Send rate-limit headers",
        state: "merged",
      };
      const server = reading(failing(), followed(), {
        ...merged,
        conflicts: false,
        workspaces: [],
      });
      await paired(new MemoryVault(), server);
      await open();
      await fireEvent.press(await button("Pull requests · 2 open"));

      expect(
        (await screen.findByRole("tab", { name: "Open · 2" })).props.accessibilityState,
      ).toEqual({ selected: true });
      const row = await button(/^Retry uploads with backoff, pull request #61, needs attention/);
      expect(row.props.accessibilityLabel).toBe(
        "Retry uploads with backoff, pull request #61, needs attention, 4 of 6 checks passed, 2 failed, conflicts, in fix-login",
      );
      expect(within(row).getByText("4/6")).toBeTruthy();
      expect(within(row).getByText("In fix-login")).toBeTruthy();
      expect(within(row).getByText("5 min")).toBeTruthy();
      // One in another repository says which, and whose project follows it.
      const elsewhere = await button(/^Accept knots in parse\(\)/);
      expect(within(elsewhere).getByText("demo · orchard/lib")).toBeTruthy();
      expect(screen.queryByText("Send rate-limit headers")).toBeNull();
      expect(asked(server, "pull.list").at(-1)).toMatchObject({ of: "open" });

      await fireEvent.press(screen.getByRole("tab", { name: "Merged" }));
      expect(await screen.findByText("Send rate-limit headers")).toBeTruthy();
      expect(screen.queryByText("Retry uploads with backoff")).toBeNull();
      expect(asked(server, "pull.list").at(-1)).toMatchObject({ of: "merged" });

      await fireEvent.press(screen.getByRole("tab", { name: "Closed" }));
      expect(
        await screen.findByText("No closed pull requests among the newest the computer read."),
      ).toBeTruthy();
      expect(screen.queryByText("Send rate-limit headers")).toBeNull();

      await fireEvent.press(screen.getByRole("tab", { name: "Open · 2" }));
      await fireEvent.press(await button(/^Retry uploads with backoff/));
      const card = await screen.findByLabelText("Pull request #61");
      expect(within(card).getByText("4 of 6 checks passed, 2 failed")).toBeTruthy();
    });

    test("one the computer no longer has among those it read says so", async () => {
      const server = reading(failing());
      await paired(new MemoryVault(), server);
      await open();
      const badge = await button("Show pull request #61, needs attention");
      server.computer.pulls = [];
      await fireEvent.press(badge);
      expect(
        await screen.findByText(
          /^The computer no longer has this pull request among those it read/,
        ),
      ).toBeTruthy();
      expect(screen.queryByLabelText("Pull request #61")).toBeNull();
    });
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

    test("a computer gone offline says when it was last seen, and not that the program runs on", async () => {
      const { server } = await watching();
      server.offline.add("host-work");
      await act(async () => server.closeChannelsTo("host-work", 4004));
      expect(
        await screen.findByText(
          /^Offline, last seen .+\. That does not mean the program stopped\.$/,
        ),
      ).toBeTruthy();
      expect(screen.queryByText(/keeps running/)).toBeNull();
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

      test("a development build times a keystroke to what comes back", async () => {
        const { server } = await inControl();
        expect(screen.getByTestId("timings")).toHaveTextContent(
          "Echo not yet · back to live not yet",
        );
        await type("ls");
        await fireEvent.press(screen.getByRole("button", { name: "Send" }));
        await waitFor(() => expect(typed(server).length).toBeGreaterThan(0));
        await act(async () => server.print("ls"));
        await waitFor(() =>
          expect(screen.getByTestId("timings")).toHaveTextContent(/^Echo \d+ ms/),
        );
      });

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

      describe("a keyboard paired with the phone", () => {
        const keyboard = () => screen.getByTestId("hardware-keys");
        const hardware = async (
          key: string,
          mods: { ctrl?: boolean; alt?: boolean; shift?: boolean } = {},
        ) => {
          await act(async () => {
            keyboard().props.onKey({
              nativeEvent: { key, ctrl: false, alt: false, shift: false, ...mods },
            });
          });
          await sent();
        };

        test("is heard only while this phone has control", async () => {
          const { server } = await inControl();
          await type("kept for later");
          expect(keyboard().props.enabled).toBe(true);
          await fireEvent.press(await button("Hand back"));
          // The draft keeps the composer, but no key is taken from it.
          await waitFor(() => expect(keyboard().props.enabled).toBe(false));
          expect(typed(server)).toEqual([]);
        });

        test("sends Esc, Ctrl-C, Alt-B and Shift-Tab to the terminal, with the field left alone", async () => {
          const { server } = await inControl();
          await type("half a thought");
          await hardware("escape");
          await hardware("c", { ctrl: true });
          await hardware("b", { alt: true });
          await hardware("tab", { shift: true });
          await hardware("left", { ctrl: true });
          expect(typed(server)).toEqual(["\x1b", "\x03", "\x1bb", "\x1b[Z", "\x1b[1;5D"]);
          expect(screen.getByTestId("composer").props.value).toBe("half a thought");
        });

        test("takes the arrows, Tab and Backspace on their own only while the field is empty", async () => {
          const { server } = await inControl();
          expect(keyboard().props.plain).toBe(true);
          await hardware("down");
          await pageSays({ t: "cursorKeys", application: true });
          await hardware("up");
          await hardware("tab");
          await hardware("backspace");
          expect(typed(server)).toEqual(["\x1b[B", "\x1bOA", "\t", "\x7f"]);

          await type("a draft");
          expect(keyboard().props.plain).toBe(false);
        });

        test("a key that may not have arrived is named as it was pressed", async () => {
          const { server } = await inControl();
          server.unanswered.add("terminal.input");
          await act(async () => {
            keyboard().props.onKey({
              nativeEvent: { key: "c", ctrl: true, alt: false, shift: false },
            });
            await jest.advanceTimersByTimeAsync(10);
          });
          await act(async () => {
            for (const socket of [...server.relaySockets]) socket.close(1006);
            await jest.advanceTimersByTimeAsync(10);
          });
          await screen.findByText(/may not have reached the computer.*Ctrl-C/);
        });
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
        // Its workspace's id, for a picture to go to: an id, like the others.
        workspace: "workspace-1",
      });
    });

    test("an agent's notice opens its terminal, named by ids alone", async () => {
      const server = new FakeServer();
      const id = "22222222-2222-4222-8222-222222222222";
      server.notices.set(id, {
        id,
        category: "agent",
        kind: "approval",
        title: "An agent needs you",
        body: "Open its terminal on this phone. It waits for you there.",
        deepLink: `yardsort://notice/agent/${id}`,
        workspaceId: "workspace-1",
        terminalId: "terminal-1",
        hostId: "host-work",
      });
      await paired(new MemoryVault(), server);
      await act(async () => router.push(`/notice/${id}`));
      await screen.findByTestId("terminal");
      const route = screen as unknown as {
        getPathname(): string;
        getSearchParams(): Record<string, string>;
      };
      expect(route.getPathname()).toBe("/terminal");
      expect(route.getSearchParams()).toEqual({
        host: "host-work",
        terminal: "terminal-1",
        workspace: "workspace-1",
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
      expect(screen.getByText("Not watching this terminal.")).toBeTruthy();
      expect(screen.queryByText("Connecting to the terminal.")).toBeNull();
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

    test("a computer that cannot share its terminals says so, and nothing says it is connecting", async () => {
      const server = new FakeServer();
      server.computer.refuses.set("terminal.attach", "Settings on the computer can replace it.");
      await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("terminal");
      await pageSays({ t: "ready" });

      expect(await screen.findByText("Settings on the computer can replace it.")).toBeTruthy();
      expect(screen.getByText("Not watching this terminal.")).toBeTruthy();
      expect(screen.queryByText("Connecting to the terminal.")).toBeNull();
      expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
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

  test("a channel lost as the network changes is opened again at once, not after its wait", async () => {
    const networkIs = (globalThis as unknown as { networkIs: (state: object) => void }).networkIs;
    const { server } = await paired();
    await open();
    await act(async () => networkIs({ type: "WIFI", isConnected: true }));
    expect(server.sockets.open).toBe(1);
    await act(async () => server.closeChannelsTo("host-work", 1006));
    expect(await screen.findByText(/^Reconnecting/)).toBeTruthy();
    // The first wait is two seconds; the change has it tried well within one.
    await act(async () => networkIs({ type: "CELLULAR", isConnected: true }));
    expect(await screen.findByText("Online", undefined, { timeout: 900 })).toBeTruthy();
    expect(server.sockets.open).toBe(1);
  });

  describe("a conversation in Chat", () => {
    /** What a computer answers that reads conversations and takes answers to their questions. */
    const READS = ["conversation.read", "approval.answer"];

    beforeEach(() => {
      // Which view each conversation was last seen in is the app's, and this test's alone.
      viewChoices.start({ read: () => null, write: () => {} });
    });

    /** Long enough for every answer to come, and for an Enter after its text. */
    const settle = () => act(async () => jest.advanceTimersByTimeAsync(500));

    async function chatting(server = new FakeServer(), capabilities = READS) {
      server.computer.capabilities = [...capabilities];
      const app = await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("chat");
      return app;
    }

    describe("where the screen starts", () => {
      const WINDOW = 800;
      const KEYBOARD = 300;
      /** The screen starts this far down the window, and then a keyboard comes up. */
      async function typingFrom(top: number) {
        await chatting();
        await act(async () =>
          measured(screen.getByTestId("above-keyboard"))().at(-1)?.(0, top, 400, WINDOW - top),
        );
        const clear = StyleSheet.flatten(screen.getByTestId("above-keyboard").props.style)
          .paddingTop as number;
        await act(async () =>
          fireEvent(screen.getByTestId("lifted"), "layout", {
            persist: () => {},
            nativeEvent: {
              layout: { x: 0, y: clear, width: 400, height: WINDOW - top - clear },
            },
          }),
        );
        await act(async () => {
          DeviceEventEmitter.emit("keyboardWillShow", {
            duration: 0,
            easing: "keyboard",
            endCoordinates: {
              screenX: 0,
              screenY: WINDOW - KEYBOARD,
              width: 400,
              height: KEYBOARD,
            },
          });
        });
        const lift = StyleSheet.flatten(screen.getByTestId("lifted").props.style).paddingBottom;
        return { clear, lift };
      }

      test("under a header laid over it, as from iOS 26: the bar goes below the header, and a keyboard lifts the field by its own height", async () => {
        // The header in these tests is 44 points.
        expect(await typingFrom(0)).toEqual({ clear: 44, lift: KEYBOARD });
      });

      test("below the header, as on Android and older iOS: nothing is added, and the lift is the same", async () => {
        expect(await typingFrom(44)).toEqual({ clear: 0, lift: KEYBOARD });
      });
    });

    test("a conversation opens as Chat, what was said and written back, with no terminal", async () => {
      const { server } = await chatting();
      expect(await screen.findByText("Add an --imperial option to weather-cli")).toBeTruthy();
      expect(screen.getByText("Units go through render.js.")).toBeTruthy();
      expect(screen.getByText(/flag\.$/)).toBeTruthy();
      expect(screen.getByText("src/cli.js")).toBeTruthy();
      expect(screen.queryByTestId("terminal")).toBeNull();
      const asked = server.operations.map((o) => o.op);
      expect(asked).toContain("conversation.read");
      // Nothing of the terminal is asked for, and its size is the computer's.
      expect(asked).not.toContain("terminal.attach");
      expect(server.resizes).toEqual([]);
    });

    describe("copying", () => {
      /** What the app put on the clipboard, newest last. */
      const copied = () => jest.requireMock<{ copied: string[] }>("expo-clipboard").copied;
      beforeEach(() => {
        copied().length = 0;
      });

      test("Copy under the reply puts the whole of it on the clipboard as it was written, and says so", async () => {
        await chatting();
        const button = await screen.findByRole("button", { name: "Copy the reply" });
        await fireEvent.press(button);
        await settle();
        expect(copied()).toEqual(["I added the `--imperial` flag.\n\n- `src/cli.js` parses it"]);
        expect(within(button).getByText("Copied")).toBeTruthy();
        await act(async () => jest.advanceTimersByTimeAsync(2_000));
        expect(within(button).getByText("Copy")).toBeTruthy();
      });

      test("a long reply folded behind Show all is copied whole", async () => {
        const server = new FakeServer();
        const long = Array.from({ length: 40 }, (_, n) => `Line ${n + 1}`).join("\n\n");
        server.computer.turns = [
          { n: 1, at: 1_700_000_000_000, user: "Go", said: [long], cut: false },
        ];
        await chatting(server);
        await screen.findByText(/Show all/);
        await fireEvent.press(screen.getByRole("button", { name: "Copy the reply" }));
        await settle();
        expect(copied()).toEqual([long]);
      });

      test("code crossing the fold is copied whole, independently of earlier code blocks", async () => {
        const server = new FakeServer();
        const code = Array.from({ length: 40 }, (_, n) => `echo line${n + 1}`).join("\n");
        server.computer.turns = [
          {
            n: 1,
            at: 1_700_000_000_000,
            user: "Go",
            said: ["```sh\necho first\n```\n\n```sh\n" + code + "\n```"],
            cut: false,
          },
        ];
        await chatting(server);
        const more = await screen.findByText(/Show all/);
        const buttons = screen.getAllByRole("button", { name: "Copy the code" });
        expect(buttons).toHaveLength(2);
        for (const button of buttons) await fireEvent.press(button);
        await settle();
        expect(copied()).toEqual(["echo first", code]);
        await fireEvent.press(more);
        for (const button of screen.getAllByRole("button", { name: "Copy the code" }))
          await fireEvent.press(button);
        await settle();
        expect(copied()).toEqual(["echo first", code, "echo first", code]);
      });

      test("your own message and a block of code each have a Copy", async () => {
        const server = new FakeServer();
        server.computer.turns = [
          {
            n: 1,
            at: 1_700_000_000_000,
            user: "How do I run it?",
            said: ["Run this:\n\n```sh\nnpm test -- --run render\n```"],
            cut: false,
          },
        ];
        await chatting(server);
        await fireEvent.press(await screen.findByRole("button", { name: "Copy your message" }));
        await fireEvent.press(screen.getByRole("button", { name: "Copy the code" }));
        await settle();
        expect(copied()).toEqual(["How do I run it?", "npm test -- --run render"]);
      });

      test("what the agent wrote can be selected, not only its code", async () => {
        /** Whether a piece of text, or the paragraph it is a span of, can be selected. */
        const selectable = (text: { props: { selectable?: boolean }; parent: unknown }) => {
          for (let at: typeof text | null = text; at; at = at.parent as typeof text | null) {
            if (at.props.selectable !== undefined) return at.props.selectable;
          }
          return false;
        };
        await chatting();
        expect(selectable(await screen.findByText("Units go through render.js."))).toBe(true);
        expect(selectable(screen.getByText(/flag\.$/))).toBe(true);
        expect(selectable(screen.getByText("src/cli.js"))).toBe(true);
      });
    });

    test("Terminal shows the terminal, and is where the conversation opens next time", async () => {
      await chatting();
      await fireEvent.press(screen.getByRole("tab", { name: "Terminal" }));
      await screen.findByTestId("terminal");
      expect(screen.queryByTestId("chat")).toBeNull();
      await act(async () => router.back());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("terminal");
      await fireEvent.press(screen.getByRole("tab", { name: "Chat" }));
      await screen.findByTestId("chat");
    });

    test("a question the agent asked shows what it would run, and Allow once answers it", async () => {
      const server = new FakeServer();
      server.computer.asks = [
        {
          id: "ask-1",
          askedAt: 1_700_000_100_000,
          tool: "Bash",
          command: "npm test -- --run render",
        },
      ];
      await chatting(server);
      expect(await screen.findByText("npm test -- --run render")).toBeTruthy();
      expect(screen.getByText("claude asks to use Bash")).toBeTruthy();
      await fireEvent.press(screen.getByRole("button", { name: "Allow once" }));
      await settle();
      expect(server.answers).toEqual([{ ask: "ask-1", decision: "allow" }]);
      expect(screen.queryByText("npm test -- --run render")).toBeNull();
    });

    test("an answer that did not reach the computer leaves the question to answer again", async () => {
      const server = new FakeServer();
      server.computer.asks = [
        {
          id: "ask-1",
          askedAt: 1_700_000_100_000,
          tool: "Bash",
          command: "npm test -- --run render",
        },
      ];
      server.computer.refuses.set("approval.answer", "Not now.");
      await chatting(server);
      await screen.findByText("npm test -- --run render");
      await fireEvent.press(screen.getByRole("button", { name: "Deny" }));
      await settle();
      expect(screen.getByText("Not now.")).toBeTruthy();
      expect(screen.getByText("npm test -- --run render")).toBeTruthy();
    });

    describe("a smoke test run waiting to be let go", () => {
      /** A run the conversation's own agent asked for, of a file changed since it was approved. */
      const WAITING = {
        run: "run-1",
        name: "Sign-in works",
        askedBy: "agent",
        session: "session-1",
        askedAt: 1_700_000_100_000,
        inputs: [{ name: "user", value: "ana" }],
        steps: [
          { id: "server", phase: "setup", words: "start `npm run dev`, and keep it running" },
          {
            id: "sign-in",
            phase: "steps",
            words: "run `node scripts/sign-in.mjs`, expecting exit 0",
          },
        ],
        env: [{ name: "TEST_PASSWORD", set: true }],
        runsAProgram: true,
        environment: ["BASE_URL", "TEST_PASSWORD"],
        approvedBefore: false,
        previous: "name: Sign-in works\nsteps:\n  - run: node sign-in.mjs\n",
        definition: "name: Sign-in works\nsteps:\n  - run: node scripts/sign-in.mjs\n",
        contentHash: "abc123",
        whole: true,
      };

      async function waiting(tests: Record<string, unknown>[], capable = true) {
        const server = new FakeServer();
        server.computer.tests = tests;
        await chatting(server, capable ? [...READS, "test.waiting", "test.decide"] : READS);
        return server;
      }

      test("shows what the file does, what it can read and what changed, and Run lets it go", async () => {
        const server = await waiting([WAITING, { ...WAITING, run: "run-2" }]);
        expect(await screen.findByText("Run “Sign-in works”?")).toBeTruthy();
        expect(screen.getByText(/Asked by claude, with user = ana\./)).toBeTruthy();
        expect(screen.getByText("start `npm run dev`, and keep it running")).toBeTruthy();
        expect(screen.getByText("run `node scripts/sign-in.mjs`, expecting exit 0")).toBeTruthy();
        expect(screen.getByText("BASE_URL, TEST_PASSWORD")).toBeTruthy();
        expect(screen.getByText("-   - run: node sign-in.mjs")).toBeTruthy();
        expect(screen.getByText("+   - run: node scripts/sign-in.mjs")).toBeTruthy();
        expect(screen.getByText(/1 more run waits behind it\./)).toBeTruthy();
        // Its own agent waits on it: the conversation needs you.
        expect(screen.getByText("Needs you")).toBeTruthy();

        await fireEvent.press(screen.getByRole("button", { name: "Run" }));
        await settle();
        expect(server.decisions).toEqual([
          { run: "run-1", contentHash: "abc123", decision: "run" },
        ]);
        // The one behind it is next.
        expect(screen.queryByText(/more run waits behind it/)).toBeNull();
        expect(screen.getByText("Run “Sign-in works”?")).toBeTruthy();
      });

      test("Don’t run refuses it, and the card goes", async () => {
        const server = await waiting([WAITING]);
        await screen.findByText("Run “Sign-in works”?");
        await fireEvent.press(screen.getByRole("button", { name: "Don’t run" }));
        await settle();
        expect(server.decisions).toEqual([
          { run: "run-1", contentHash: "abc123", decision: "refuse" },
        ]);
        expect(screen.queryByText("Run “Sign-in works”?")).toBeNull();
      });

      test("one the computer could not send whole says why, and can only be refused", async () => {
        await waiting([
          {
            ...WAITING,
            previous: undefined,
            definition: undefined,
            whole: false,
            whyNot: "This test is too long to show whole here. Run it at the computer.",
          },
        ]);
        expect(await screen.findByText(/too long to show whole here/)).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Run" })).toBeNull();
        expect(screen.getByRole("button", { name: "Don’t run" })).toBeTruthy();
      });

      test("an answer the computer refused leaves the card, with why", async () => {
        const server = await waiting([WAITING]);
        server.computer.refuses.set("test.decide", "This phone may only watch here.");
        await screen.findByText("Run “Sign-in works”?");
        await fireEvent.press(screen.getByRole("button", { name: "Run" }));
        await settle();
        expect(screen.getByText("This phone may only watch here.")).toBeTruthy();
        expect(screen.getByText("Run “Sign-in works”?")).toBeTruthy();
      });

      test("another agent's run says so, and does not make this conversation need you", async () => {
        await waiting([{ ...WAITING, session: "other" }]);
        expect(await screen.findByText(/Asked by another agent/)).toBeTruthy();
        expect(screen.queryByText("Needs you")).toBeNull();
      });

      test("a computer from before this is not asked", async () => {
        const server = await waiting([WAITING], false);
        await settle();
        expect(screen.queryByText("Run “Sign-in works”?")).toBeNull();
        expect(server.operations.map((o) => o.op)).not.toContain("test.waiting");
      });
    });

    test("a message is typed into its terminal with Enter, and control given back at once", async () => {
      const { server } = await chatting();
      await fireEvent.changeText(screen.getByLabelText("Message claude"), "commit this");
      await fireEvent.press(screen.getByRole("button", { name: "Send" }));
      await settle();
      expect(server.releases).toBe(1);
      expect(server.acquires).toEqual([false]);
      expect(server.inputs.map((i) => i.data)).toEqual(["commit this", "\r"]);
      expect(server.resizes).toEqual([]);
      expect(screen.getByLabelText("Message claude").props.value).toBe("");
    });

    test("a terminal the person at the computer is typing in keeps the message, and says so", async () => {
      const server = new FakeServer();
      server.refuseControl = true;
      await chatting(server);
      await fireEvent.changeText(screen.getByLabelText("Message claude"), "commit this");
      await fireEvent.press(screen.getByRole("button", { name: "Send" }));
      await settle();
      expect(screen.getByText("Held.")).toBeTruthy();
      expect(server.inputs).toEqual([]);
      expect(screen.getByLabelText("Message claude").props.value).toBe("commit this");
    });

    test("a computer from before Chat opens the terminal, with no switch", async () => {
      const server = new FakeServer();
      await paired(new MemoryVault(), server);
      await fireEvent.press(await open());
      await fireEvent.press(
        await screen.findByRole("button", { name: "The login page times out" }),
      );
      await screen.findByTestId("terminal");
      expect(screen.queryByRole("tab", { name: "Chat" })).toBeNull();
      expect(server.operations.map((o) => o.op)).not.toContain("conversation.read");
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
    test("a workspace on a computer gone offline says when it was last seen", async () => {
      const { server } = await paired();
      await fireEvent.press(await open());
      await screen.findByText("The login page times out");
      server.offline.add("host-work");
      await act(async () => server.closeChannelsTo("host-work", 4004));
      expect(await screen.findByText(/^Offline, last seen/)).toBeTruthy();
      expect(screen.getByText(/^As of /)).toBeTruthy();
    });

    test("memory read before the computer went offline says when it was read", async () => {
      const { server } = await paired();
      await fireEvent.press(await open());
      await fireEvent.press(await screen.findByRole("button", { name: "Approved memory" }));
      await screen.findByText("Migrations are append-only.");
      expect(screen.queryByText(/^As of /)).toBeNull();
      server.offline.add("host-work");
      await act(async () => server.closeChannelsTo("host-work", 4004));
      expect(await screen.findByText(/^Offline, last seen/)).toBeTruthy();
      expect(screen.getByText(/^As of /)).toBeTruthy();
      expect(screen.getByText("Migrations are append-only.")).toBeTruthy();
    });

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

  describe("diagnostics", () => {
    async function readThem(diagnostics: Diagnostics) {
      const server = new FakeServer();
      const app = await start({
        server,
        diagnostics,
        env: { ...envFor(server), shelf: new MemoryVault() },
      });
      await signIn();
      await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
      await fireEvent.changeText(await screen.findByLabelText("Pairing code"), "ABCD2345");
      await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
      await screen.findByText("Say yes at Workstation");
      app.server.answer("approved");
      await screen.findByText("Workstation let this phone in", undefined, ANSWER);
      await fireEvent.press(screen.getByRole("button", { name: "Done" }));
      await fireEvent.press(await open());
      await screen.findByText("The login page times out");
      app.server.offline.add("host-work");
      await act(async () => app.server.closeChannelsTo("host-work", 4004));
      await screen.findByText(/^Offline, last seen/);
      await act(async () => router.push("/settings"));
      await fireEvent.press(await screen.findByRole("button", { name: "Diagnostics" }));
      return app;
    }

    test("show versions, the server and device, and how each computer stood, with codes", async () => {
      const { accounts } = await readThem(new Diagnostics());
      const shown = (await screen.findByText(/^Yardsort phone diagnostics/)).props
        .children as string;
      const active = accounts.getSnapshot().active;
      expect(shown).toContain("App 0.1.5");
      expect(shown).toContain("System ios 18.1");
      expect(shown).toMatch(/Protocol \d+\.\d+/);
      expect(shown).toContain(`Server ${active?.server}`);
      expect(shown).toContain(`Device ${active?.device}`);
      expect(shown).toMatch(/Z {2}sign-in signedIn/);
      expect(shown).toMatch(/Z {2}computer host-work online/);
      expect(shown).toMatch(/Z {2}computer host-work offline \(hostOffline:4004\)/);
    });

    test("hold no name, title or branch, and share exactly what is shown", async () => {
      const share = jest
        .spyOn(Share, "share")
        .mockResolvedValue({ action: "sharedAction", activityType: undefined });
      await readThem(new Diagnostics());
      const shown = (await screen.findByText(/^Yardsort phone diagnostics/)).props
        .children as string;
      for (const content of [
        "Workstation",
        "Ana’s phone",
        "The login page times out",
        "fix-login",
        "Example Remote",
      ]) {
        expect(shown).not.toContain(content);
      }
      await fireEvent.press(screen.getByRole("button", { name: "Share" }));
      expect(share).toHaveBeenCalledWith({ message: shown });
    });
  });

  describe("starting work", () => {
    const START = [
      "harness.list",
      "workspace.start",
      "session.start",
      "upload.begin",
      "upload.chunk",
    ];
    const able = () => {
      const server = new FakeServer();
      server.computer.capabilities = START;
      return server;
    };
    const asked = (server: FakeServer, op: string) =>
      server.operations.filter((o) => o.op === op).map((o) => o.params);

    test("a computer from before offers nothing to start", async () => {
      const { server } = await paired();
      await open();
      expect(screen.queryByRole("button", { name: "New workspace" })).toBeNull();
      expect(asked(server, "harness.list")).toEqual([]);
    });

    test("a new workspace starts with the agent and message chosen, and opens its terminal", async () => {
      const { server } = await paired(new MemoryVault(), able());
      await open();
      await fireEvent.press(screen.getByRole("button", { name: "New workspace" }));
      // One project: it is the one. The computer's default agent is chosen until another is.
      const checked = (name: string) =>
        screen.getByRole("radio", { name }).props.accessibilityState as { checked: boolean };
      await screen.findByRole("radio", { name: "demo" });
      expect(checked("demo")).toMatchObject({ checked: true });
      expect(checked("Claude Code")).toMatchObject({ checked: true });
      expect(checked("Codex")).toMatchObject({ checked: false });
      const go = screen.getByRole("button", { name: "Start workspace" });
      expect(go).toBeDisabled();
      await fireEvent.press(screen.getByRole("radio", { name: "Codex" }));
      await fireEvent.changeText(
        screen.getByLabelText("What should it do?"),
        "Fix the spacing on small screens",
      );
      await fireEvent.press(screen.getByRole("button", { name: "Start workspace" }));
      await screen.findByTestId("terminal");
      expect(asked(server, "workspace.start")).toEqual([
        { project: "project-1", prompt: "Fix the spacing on small screens", harness: "codex" },
      ]);
    });

    test("a refusal says why, and nothing opens", async () => {
      const server = able();
      server.computer.refuses.set("workspace.start", "That agent is not one it can start.");
      await paired(new MemoryVault(), server);
      await open();
      await fireEvent.press(screen.getByRole("button", { name: "New workspace" }));
      await fireEvent.changeText(await screen.findByLabelText("What should it do?"), "Do it");
      await fireEvent.press(screen.getByRole("button", { name: "Start workspace" }));
      expect(await screen.findByText("That agent is not one it can start.")).toBeTruthy();
      expect(screen.queryByTestId("terminal")).toBeNull();
      expect(asked(server, "workspace.start")).toHaveLength(1);
    });

    test("a project whose workspaces are all on a later page can be chosen", async () => {
      const server = able();
      server.computer.moreProjects = [{ id: "project-2", name: "site", missing: false }];
      server.computer.moreWorkspaces = [
        { ...server.computer.workspaces[0], id: "workspace-9", project: "project-2", name: "site" },
      ];
      await paired(new MemoryVault(), server);
      await open();
      await fireEvent.press(screen.getByRole("button", { name: "New workspace" }));
      await fireEvent.press(await screen.findByRole("radio", { name: "site" }));
      expect(screen.getByRole("radio", { name: "demo" })).toBeTruthy();
      expect(asked(server, "workspace.list").map((p) => (p as { cursor: unknown }).cursor)).toEqual(
        expect.arrayContaining([null, "k1.more"]),
      );
      await fireEvent.changeText(screen.getByLabelText("What should it do?"), "New page");
      await fireEvent.press(screen.getByRole("button", { name: "Start workspace" }));
      await screen.findByTestId("terminal");
      expect(asked(server, "workspace.start")).toEqual([
        { project: "project-2", prompt: "New page", harness: "claude" },
      ]);
    });

    test("another agent joins a workspace, with no message unless one is given", async () => {
      const { server } = await paired(new MemoryVault(), able());
      await fireEvent.press(await open());
      await fireEvent.press(await screen.findByRole("button", { name: "New agent" }));
      expect(await screen.findByText(/^Another agent in fix-login/)).toBeTruthy();
      await fireEvent.press(screen.getByRole("button", { name: "Start agent" }));
      await screen.findByTestId("terminal");
      expect(asked(server, "session.start")).toEqual([
        { workspace: "workspace-1", harness: "claude" },
      ]);
    });

    describe("resuming", () => {
      const ended = (server: FakeServer, resumable: boolean) => {
        server.computer.endedSessions = [
          {
            ...server.computer.sessions[0],
            id: "session-2",
            title: "An older conversation",
            endedAt: 1_700_000_000_500,
            exitCode: 0,
            process: "ended",
            terminal: null,
            resumable,
          },
        ];
      };
      const showEnded = async () => {
        await fireEvent.press(await open());
        await fireEvent.press(
          await screen.findByRole("button", { name: "Show ended conversations" }),
        );
        await screen.findByText("An older conversation");
      };

      test("an ended conversation resumes, with a message if one is given, and its terminal opens", async () => {
        const server = able();
        server.computer.capabilities = [...START, "session.resume"];
        ended(server, true);
        await paired(new MemoryVault(), server);
        await showEnded();
        await fireEvent.press(
          screen.getByRole("button", { name: "An older conversation, Ended, resume" }),
        );
        expect(await screen.findByText(/^An older conversation, in fix-login\./)).toBeTruthy();
        await fireEvent.changeText(
          screen.getByLabelText("A message to go on with, if any"),
          "Carry on with the tests",
        );
        await fireEvent.press(screen.getByRole("button", { name: "Resume" }));
        await screen.findByTestId("terminal");
        expect(asked(server, "session.resume")).toEqual([
          { session: "session-2", prompt: "Carry on with the tests" },
        ]);
      });

      test("a refusal says why, and nothing opens", async () => {
        const server = able();
        server.computer.capabilities = [...START, "session.resume"];
        server.computer.refuses.set("session.resume", "This session is already running.");
        ended(server, true);
        await paired(new MemoryVault(), server);
        await showEnded();
        await fireEvent.press(
          screen.getByRole("button", { name: "An older conversation, Ended, resume" }),
        );
        await fireEvent.press(await screen.findByRole("button", { name: "Resume" }));
        expect(await screen.findByText("This session is already running.")).toBeTruthy();
        expect(screen.queryByTestId("terminal")).toBeNull();
        expect(asked(server, "session.resume")).toEqual([{ session: "session-2" }]);
      });

      test.each([
        // A phone let only watch is one: the computer lists nothing as resumable to it.
        ["one the computer does not offer this phone to resume", true, false],
        ["a computer from before minor 6", false, true],
      ])("%s offers no resuming", async (_, answers, resumable) => {
        const server = able();
        if (answers) server.computer.capabilities = [...START, "session.resume"];
        ended(server, resumable);
        await paired(new MemoryVault(), server);
        await showEnded();
        expect(screen.queryByText("Resume")).toBeNull();
        expect(screen.queryByRole("button", { name: /resume$/ })).toBeNull();
      });
    });

    describe("a picture", () => {
      const told = () =>
        (jest.requireMock("react-native-webview") as { told: string[] }).told as string[];

      async function inControl(server: FakeServer, pickImage: PickImage) {
        told().length = 0;
        await paired(new MemoryVault(), server, pickImage);
        await fireEvent.press(await open());
        await fireEvent.press(
          await screen.findByRole("button", { name: "The login page times out" }),
        );
        // Once the computer has said which conversation this is.
        const page = await screen.findByTestId("terminal");
        await act(async () =>
          fireEvent(page, "message", {
            nativeEvent: { data: JSON.stringify({ t: "ready" }) },
          }),
        );
        await fireEvent.press(await screen.findByRole("button", { name: "Take control" }));
        await screen.findByTestId("composer");
      }

      test("goes to the computer, and its path there is pasted, quoted, with no Enter", async () => {
        const server = able();
        const photo = Uint8Array.from({ length: 100_000 }, (_, n) => n % 251);
        await inControl(server, async () => ({ name: "IMG_4141.jpg", bytes: photo }));
        await fireEvent.press(screen.getByRole("button", { name: "Give it a picture" }));
        await waitFor(() => expect(server.inputs).toHaveLength(1));
        expect(server.uploads).toHaveLength(1);
        expect(server.uploads[0]).toMatchObject({
          workspace: "workspace-1",
          name: "IMG_4141.jpg",
          size: photo.length,
        });
        expect(Uint8Array.from(server.uploads[0]!.bytes)).toEqual(photo);
        expect(server.inputs).toEqual([
          { data: "/home/ana/My\\ Data/uploads/workspace-1/1-photo.jpg ", paste: true },
        ]);
      });

      test("the picker sending the app to the background keeps the channel and control", async () => {
        // On Android the picker is another activity, and the app is told it is in the background.
        const changes: ((state: string) => void)[] = [];
        jest.spyOn(AppState, "addEventListener").mockImplementation(((
          _: string,
          listener: (state: string) => void,
        ) => {
          changes.push(listener);
          return { remove: () => {} };
        }) as typeof AppState.addEventListener);
        const server = able();
        let choose: (picked: { name: string; bytes: Uint8Array }) => void = () => {};
        await inControl(
          server,
          () =>
            new Promise((resolve) => {
              choose = resolve;
            }),
        );
        await fireEvent.press(screen.getByRole("button", { name: "Give it a picture" }));
        await act(async () => changes.forEach((tell) => tell("background")));
        await act(async () => changes.forEach((tell) => tell("active")));
        await act(async () => choose({ name: "IMG_1.jpg", bytes: Uint8Array.of(1, 2, 3) }));
        await waitFor(() => expect(server.inputs).toHaveLength(1));
        expect(server.releases).toBe(0);
        expect(server.uploads).toHaveLength(1);
        expect(server.inputs[0]).toMatchObject({ paste: true });
        expect(screen.queryByText(/did not reach the computer/)).toBeNull();

        // Once it is closed, the background is the background again.
        await act(async () => changes.forEach((tell) => tell("background")));
        await waitFor(() => expect(server.releases).toBe(1));
      });

      test("none chosen sends nothing", async () => {
        const server = able();
        await inControl(server, async () => null);
        await fireEvent.press(screen.getByRole("button", { name: "Give it a picture" }));
        await act(async () => {});
        expect(asked(server, "upload.begin")).toEqual([]);
        expect(server.inputs).toEqual([]);
      });

      test("is not offered by a computer that cannot take one", async () => {
        await inControl(new FakeServer(), async () => null);
        expect(screen.queryByRole("button", { name: "Give it a picture" })).toBeNull();
      });
    });
  });
});
