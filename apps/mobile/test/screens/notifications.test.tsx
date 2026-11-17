// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The Notifications list on the phone (46): the bell and its count, the counts on a computer's
// row, the list in its three parts, the marks on a computer's workspaces, and seen said back to
// the computer. Through the real router and the real protocol client, against the fake server.

import { afterEach, beforeEach, describe, expect, jest, test } from "@jest/globals";
import { router, Stack } from "expo-router";
import { act, fireEvent, renderRouter, screen, waitFor, within } from "expo-router/testing-library";
import { Alert } from "react-native";

import Account from "../../src/app/account";
import ComputerScreen from "../../src/app/computer";
import Connect from "../../src/app/connect";
import Start from "../../src/app/index";
import NotificationList from "../../src/app/notification-list";
import NotificationSettings from "../../src/app/notifications";
import Pair from "../../src/app/pair";
import Settings from "../../src/app/settings";
import TerminalScreen from "../../src/app/terminal";
import WorkflowScreen from "../../src/app/workflow";
import WorkspaceScreen from "../../src/app/workspace";
import { Accounts } from "../../src/lib/accounts";
import { AppProvider } from "../../src/lib/context";
import { Diagnostics } from "../../src/lib/diagnostics";
import { folds } from "../../src/lib/folds";
import { Incoming } from "../../src/lib/pairing";
import { MemoryVault } from "../lib/memory-vault";
import { envFor, FakeServer, LIST_CAPABILITIES, notification } from "./fake-server";

jest.setTimeout(30_000);
const ANSWER = { timeout: 5_000 };

beforeEach(() => {
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  folds.start({ read: () => null, write: () => {} });
});

afterEach(() => {
  jest.restoreAllMocks();
});

/** Signed in, with Workstation having said yes: on the account screen. */
async function paired(server = new FakeServer()) {
  const diagnostics = new Diagnostics();
  const accounts = new Accounts({ diagnostics, ...envFor(server) }, new MemoryVault());
  const app = {
    accounts,
    browser: async (_url: string, redirect: string) => `${redirect}?code=a-code`,
    deviceName: "iPhone",
    incoming: new Incoming(),
    pickImage: async () => null,
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
      settings: Settings,
      pair: Pair,
      computer: ComputerScreen,
      workspace: WorkspaceScreen,
      terminal: TerminalScreen,
      workflow: WorkflowScreen,
      "notification-list": NotificationList,
      notifications: NotificationSettings,
    },
    { initialUrl: "/" },
  );
  await fireEvent.changeText(
    await screen.findByLabelText("Server address"),
    "https://remote.example.com",
  );
  await fireEvent.press(screen.getByRole("button", { name: "Look up" }));
  await fireEvent.changeText(await screen.findByLabelText("Name this phone"), "Ana’s phone");
  await fireEvent.press(screen.getByRole("button", { name: "Sign in" }));
  await screen.findByText("Signed in to Example Remote as Ana’s phone.");
  await fireEvent.press(await screen.findByRole("button", { name: "Pair a computer" }));
  await fireEvent.changeText(await screen.findByLabelText("Pairing code"), "ABCD2345");
  await fireEvent.press(screen.getByRole("button", { name: "Pair with ABCD-2345" }));
  await screen.findByText("Say yes at Workstation");
  server.answer("approved");
  await fireEvent.press(await screen.findByRole("button", { name: "Done" }, ANSWER));
  await screen.findByRole("button", { name: /^Workstation, / });
  return { server, accounts };
}

/** A computer that keeps a list, with an agent asking, one finished, and one seen before. */
function listing(server = new FakeServer()) {
  server.computer.capabilities = [...LIST_CAPABILITIES];
  server.computer.notifications = [
    notification("asks", {
      kind: "agent.asks",
      part: "needs",
      at: Date.now() - 60_000,
      command: "git push --force-with-lease",
      workedMs: undefined,
    }),
    notification("done", { said: "Added the flag. 4 tests pass." }),
    notification("old", { part: "earlier", workedMs: 3 * 60_000, settledAt: Date.now() - 1_000 }),
  ];
  return server;
}

/** A second computer on the account that has let this phone in. */
function alsoPaired(server: FakeServer, id: string, name: string) {
  server.hosts.push({
    id,
    name,
    createdAt: 1_700_000_000_000,
    lastSeenAt: 1_700_000_100_000,
    current: false,
  });
  server.grants.push({
    ...(server.grants[0] as (typeof server.grants)[number]),
    id: `grant-${id}`,
    host: id,
    hostName: name,
  });
}

/** Where the router is: the testing library's screen says, and its type does not. */
const route = () =>
  screen as unknown as { getPathname(): string; getSearchParams(): Record<string, string> };

const bell = (name: string | RegExp = /^Notifications/) => screen.findByRole("button", { name });
const row = (name: RegExp) => screen.findByRole("button", { name });
const asked = (server: FakeServer, op: string) =>
  server.operations.filter((o) => o.op === op).length;

describe("Notifications on the phone", () => {
  test("the bell counts what needs you, and a computer's row says what waits there", async () => {
    await paired(listing());
    // The bell is in the header's corner beside Settings, not in the screen.
    expect(await bell("Notifications, 1 needs you")).toBeTruthy();
    expect(
      within(screen.getByTestId("screen")).queryByRole("button", { name: /^Notifications/ }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Settings" })).toBeTruthy();
    expect(
      await screen.findByRole("button", { name: "Workstation, Online, 1 needs you, 1 finished" }),
    ).toBeTruthy();
  });

  test("a computer with nothing waiting says only that it is online, and the bell has no count", async () => {
    const server = new FakeServer();
    server.computer.capabilities = [...LIST_CAPABILITIES];
    await paired(server);
    await waitFor(() => expect(asked(server, "notification.list")).toBeGreaterThan(0));
    expect(await screen.findByRole("button", { name: "Workstation, Online" })).toBeTruthy();
    expect(await bell("Notifications")).toBeTruthy();
  });

  test("a computer from before the list is not asked for one, and the bell stays", async () => {
    const { server } = await paired();
    expect(await screen.findByRole("button", { name: "Workstation, Online" })).toBeTruthy();
    expect(await bell("Notifications")).toBeTruthy();
    expect(asked(server, "notification.list")).toBe(0);

    await fireEvent.press(await bell());
    expect(
      await screen.findByText(/Workstation runs an earlier Yardsort, which keeps no list/),
    ).toBeTruthy();
  });

  test("the list has three parts, each row with its computer, its words and its line", async () => {
    await paired(listing());
    await fireEvent.press(await bell("Notifications, 1 needs you"));

    const asks = await row(/^fix-login, demo · Workstation, claude asks for a permission/);
    expect(asks.props.accessibilityLabel).toBe(
      "fix-login, demo · Workstation, claude asks for a permission, git push --force-with-lease, 1m, opens the conversation",
    );
    expect(
      await row(
        /^fix-login, demo · Workstation, claude finished after 12 min, “Added the flag. 4 tests pass.”, 4m/,
      ),
    ).toBeTruthy();
    expect(await row(/claude finished after 3 min/)).toBeTruthy();

    // The headings, with how many: Earlier is not counted.
    const heading = (name: string) => screen.getByRole("header", { name });
    expect(heading("Needs you")).toBeTruthy();
    expect(heading("Finished, not seen")).toBeTruthy();
    expect(heading("Earlier")).toBeTruthy();
    expect(screen.getByText(/A row leaves Needs you when it is answered/)).toBeTruthy();

    // What is pushed to this phone is chosen elsewhere, and reached from the foot of the list.
    await fireEvent.press(screen.getByRole("button", { name: "Choose what reaches this phone" }));
    await waitFor(() => expect(route().getPathname()).toBe("/notifications"));
  });

  test("opening a finished row marks it seen on the computer, and opens its conversation", async () => {
    const { server } = await paired(listing());
    await fireEvent.press(await bell("Notifications, 1 needs you"));
    await fireEvent.press(await row(/claude finished after 12 min/));

    await waitFor(() =>
      expect(server.seen).toEqual([{ host: "host-work", ids: ["done"], all: false }]),
    );
    // The conversation it is about, on this computer.
    expect(route().getPathname()).toBe("/terminal");
    expect(route().getSearchParams()).toEqual({
      host: "host-work",
      workspace: "workspace-1",
      session: "session-1",
      terminal: "terminal-1",
      label: "claude",
    });

    await act(async () => router.back());
    // Back in the list it is under Earlier, and nothing is unseen.
    await waitFor(() =>
      expect(screen.queryByRole("header", { name: "Finished, not seen" })).toBeNull(),
    );
    expect(screen.getByRole("header", { name: "Needs you" })).toBeTruthy();
  });

  test("a row that waits is not marked by opening it: it leaves when it is answered", async () => {
    const { server } = await paired(listing());
    await fireEvent.press(await bell("Notifications, 1 needs you"));
    await fireEvent.press(await row(/claude asks for a permission/));
    await waitFor(() => expect(route().getPathname()).toBe("/terminal"));
    // Its terminal is this agent's, which also finished: that row is seen by looking, and only it.
    await waitFor(() =>
      expect(server.seen).toEqual([{ host: "host-work", ids: ["done"], all: false }]),
    );

    // Answered at the computer: it says its list changed, and the row is gone from the list.
    server.computer.notifications = server.computer.notifications.filter((n) => n.id !== "asks");
    await act(async () => router.back());
    await waitFor(() => expect(screen.queryByRole("header", { name: "Needs you" })).toBeNull());
    expect(await screen.findByText("Nothing needs you.")).toBeTruthy();
  });

  test("the list is read again when the computer says it changed", async () => {
    const { server } = await paired(listing());
    await fireEvent.press(await bell("Notifications, 1 needs you"));
    await row(/claude asks for a permission/);

    server.computer.notifications = [
      notification("flow", {
        kind: "workflow.question",
        part: "needs",
        at: Date.now() - 30_000,
        session: undefined,
        terminal: undefined,
        harness: undefined,
        workedMs: undefined,
        title: "release-check",
        run: "run-1",
        step: "pick-version",
        said: "Should this be 0.9.0 or 1.0.0?",
      }),
      ...server.computer.notifications,
    ];
    await act(async () => server.listChanged());
    const flow = await row(/Workflow release-check asks at pick-version/);
    expect(flow.props.accessibilityLabel).toContain("“Should this be 0.9.0 or 1.0.0?”");
    expect(flow.props.accessibilityLabel).toContain("opens the decision");
  });

  test("nothing waiting says so, and a computer that is offline shows a line and no rows", async () => {
    const server = listing();
    server.listsOf.set("host-box", [
      notification("box", { kind: "agent.asks", part: "needs", workspaceName: "bump-parser" }),
    ]);
    server.computer.notifications = server.computer.notifications.filter((n) => n.id === "old");
    const { accounts } = await paired(server);
    alsoPaired(server, "host-box", "linux-box");
    server.offline.add("host-box");
    void accounts;

    await fireEvent.press(await bell());
    expect(
      await screen.findByText(
        "linux-box is offline. What waits there shows when it is back.",
        undefined,
        ANSWER,
      ),
    ).toBeTruthy();
    expect(await screen.findByText("Nothing needs you.")).toBeTruthy();
    expect(screen.getByText(/show up here/)).toBeTruthy();
    // What left before still shows, and nothing of the computer that is away.
    expect(await row(/claude finished after 3 min/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /bump-parser/ })).toBeNull();
    expect(screen.queryByText(/A row leaves Needs you/)).toBeNull();
  });

  test("with no computer reached, it does not say that nothing needs you", async () => {
    const server = listing();
    await paired(server);
    await bell("Notifications, 1 needs you");
    // The computer goes away: its rows go, and so does its count on the bell.
    server.offline.add("host-work");
    server.closeChannelsTo("host-work", 4001);
    await fireEvent.press(await bell());
    expect(
      await screen.findByText(
        "Workstation is offline. What waits there shows when it is back.",
        undefined,
        ANSWER,
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /claude asks for a permission/ })).toBeNull();
    // Nothing is known of what waits there, so nothing is claimed.
    expect(screen.queryByText("Nothing needs you.")).toBeNull();
  });

  test("two computers are one list, and the bell counts both", async () => {
    const server = listing();
    server.listsOf.set("host-box", [
      notification("box", {
        kind: "test.waits",
        part: "needs",
        at: Date.now() - 14 * 60_000,
        workspaceName: "flaky-upload-test",
        title: "sign-in",
        session: undefined,
        terminal: undefined,
        harness: undefined,
        workedMs: undefined,
      }),
    ]);
    await paired(server);
    alsoPaired(server, "host-box", "linux-box");

    // On a computer's own screen the bell still counts every computer's.
    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
    await fireEvent.press(await bell("Notifications, 2 need you"));

    expect(await row(/^fix-login, demo · Workstation, claude asks for a permission/)).toBeTruthy();
    const test = await row(/^flaky-upload-test, demo · linux-box/);
    expect(test.props.accessibilityLabel).toContain(
      "Smoke test sign-in waits for you to let it run, 14m, opens its workspace, to answer it",
    );
  });

  test("a computer marks its workspaces: needs you before finished, in place of running", async () => {
    const server = listing();
    server.computer.workspaces.push({
      ...(server.computer.workspaces[0] as (typeof server.computer.workspaces)[number]),
      id: "workspace-2",
      name: "add-option",
      branch: { label: "ys/add-option", detached: false, unborn: false },
      runningSessions: 0,
    });
    server.computer.notifications.push(
      notification("other", { workspace: "workspace-2", workspaceName: "add-option" }),
    );
    await paired(server);
    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));

    // fix-login has an agent asking and one finished, and one running: what waits is said.
    expect(await row(/^fix-login, ys\/fix-login · needs you$/)).toBeTruthy();
    expect(await row(/^add-option, ys\/add-option · finished$/)).toBeTruthy();

    // Folded, the project still says what waits inside it.
    await fireEvent.press(screen.getByRole("button", { name: "demo, 2" }));
    expect(
      screen.getByRole("button", { name: "demo, 2, 1 needs you · 1 finished · 1 running" }),
    ).toBeTruthy();
  });

  test("a workspace with nothing listed keeps saying what runs there", async () => {
    const server = new FakeServer();
    server.computer.capabilities = [...LIST_CAPABILITIES];
    await paired(server);
    await fireEvent.press(await screen.findByRole("button", { name: /^Workstation, / }));
    expect(await row(/^fix-login, ys\/fix-login · 1 running$/)).toBeTruthy();
  });

  test("a smoke test's row opens its workspace, where its card is answered", async () => {
    const server = new FakeServer();
    server.computer.capabilities = [...LIST_CAPABILITIES, "test.waiting", "test.decide"];
    server.computer.notifications = [
      notification("test", {
        kind: "test.waits",
        part: "needs",
        title: "sign-in",
        session: undefined,
        terminal: undefined,
        harness: undefined,
        workedMs: undefined,
      }),
    ];
    server.computer.tests = [
      {
        run: "run-test-1",
        name: "sign-in",
        workspace: "workspace-1",
        askedBy: "app",
        askedAt: Date.now() - 60_000,
        inputs: [],
        steps: [{ id: "open", phase: "steps", words: "Open the sign-in page" }],
        env: [],
        runsAProgram: false,
        environment: [],
        approvedBefore: false,
        contentHash: "abc123",
        whole: true,
      },
    ];
    await paired(server);
    await fireEvent.press(await bell("Notifications, 1 needs you"));
    await fireEvent.press(await row(/Smoke test sign-in waits for you to let it run/));

    expect(await screen.findByText("Run “sign-in”?", undefined, ANSWER)).toBeTruthy();
    expect(screen.getByText(/Asked by you, at the computer/)).toBeTruthy();
    await fireEvent.press(screen.getByRole("button", { name: "Don’t run" }));
    await waitFor(() => expect(server.decisions.map((d) => d.decision)).toEqual(["refuse"]));
  });
});
