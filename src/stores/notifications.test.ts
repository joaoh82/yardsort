import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  notificationList: vi.fn(),
  notificationSeen: vi.fn(),
  notificationSeenTerminal: vi.fn(),
  notificationSeenAll: vi.fn(),
  uiStateSave: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { notification } from "@/test/fixtures";
import { counts, GROUPING_KEY, listedIn, useNotificationsStore } from "./notifications";
import { useTerminalStore, type TerminalTab } from "./terminals";

const tab = (id: string, extra: Partial<TerminalTab> = {}): TerminalTab => ({
  id,
  workspaceId: "w-app-fix-login",
  title: "claude",
  exit: null,
  recordId: `session-${id}`,
  busy: false,
  attention: false,
  toldAbout: null,
  toldAt: null,
  busySince: null,
  ...extra,
});

const ringed = () =>
  useTerminalStore
    .getState()
    .tabs.filter((t) => t.attention)
    .map((t) => t.id);

let focused = true;

beforeEach(() => {
  vi.clearAllMocks();
  focused = true;
  vi.spyOn(document, "hasFocus").mockImplementation(() => focused);
  useNotificationsStore.setState({
    items: [],
    loaded: false,
    error: null,
    grouping: "kind",
    inFront: null,
  });
  useTerminalStore.setState({ tabs: [tab("pty-a"), tab("pty-b"), tab("pty-c")], active: {} });
  core.notificationSeen.mockResolvedValue(null);
  core.notificationSeenTerminal.mockResolvedValue(null);
  core.notificationSeenAll.mockResolvedValue(null);
  core.uiStateSave.mockResolvedValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
});

const asks = notification("a", { terminalId: "pty-a" });
const finishedB = notification("b", {
  kind: "agent.finished",
  part: "finished",
  terminalId: "pty-b",
});
const finishedC = notification("c", {
  kind: "agent.finished",
  part: "finished",
  terminalId: "pty-c",
  projectId: "p-site",
  projectName: "site",
});
const earlier = notification("d", {
  kind: "agent.finished",
  part: "earlier",
  settledAt: 5,
  terminalId: null,
});

describe("the notifications store", () => {
  it("loads the list, counts it, and rings the tabs whose agents finished unseen", async () => {
    core.notificationList.mockResolvedValue([asks, finishedB, finishedC, earlier]);
    // A ring the window drew on its own, for something the core says was seen, goes.
    useTerminalStore.setState({ tabs: [tab("pty-a", { attention: true }), tab("pty-b")] });
    await useNotificationsStore.getState().load();
    const { items, loaded } = useNotificationsStore.getState();
    expect(loaded).toBe(true);
    expect(counts(items)).toEqual({ needs: 1, finished: 2 });
    expect(listedIn(items, "p-app")).toBe(2);
    expect(listedIn(items, "p-site")).toBe(1);
    expect(ringed()).toEqual(["pty-b"]);
    expect(core.notificationSeenTerminal).not.toHaveBeenCalled();
  });

  it("marks a row seen at once when its terminal is the one in front", async () => {
    useNotificationsStore.setState({ inFront: "pty-b" });
    core.notificationList.mockResolvedValue([finishedB, finishedC]);
    await useNotificationsStore.getState().load();
    expect(core.notificationSeenTerminal).toHaveBeenCalledExactlyOnceWith("pty-b");
    const parts = useNotificationsStore.getState().items.map((item) => [item.id, item.part]);
    expect(parts).toEqual([
      ["b", "earlier"],
      ["c", "finished"],
    ]);
    expect(ringed()).toEqual(["pty-c"]);
  });

  it("does not count a terminal as looked at while the window is not in front", async () => {
    focused = false;
    useNotificationsStore.setState({ inFront: "pty-b" });
    core.notificationList.mockResolvedValue([finishedB]);
    await useNotificationsStore.getState().load();
    expect(core.notificationSeenTerminal).not.toHaveBeenCalled();
    expect(ringed()).toEqual(["pty-b"]);

    // Coming back to the window and its terminal is looking.
    focused = true;
    useNotificationsStore.getState().looking("pty-b");
    expect(core.notificationSeenTerminal).toHaveBeenCalledExactlyOnceWith("pty-b");
    expect(ringed()).toEqual([]);
  });

  it("asks nothing of the core when the terminal in front has nothing unseen", () => {
    useNotificationsStore.setState({ items: [asks, finishedC] });
    useNotificationsStore.getState().looking("pty-a");
    useNotificationsStore.getState().looking(null);
    expect(core.notificationSeenTerminal).not.toHaveBeenCalled();
    expect(useNotificationsStore.getState().inFront).toBeNull();
  });

  it("moves a finished row to Earlier when it is marked seen, and never a question", async () => {
    useNotificationsStore.setState({ items: [asks, finishedB, finishedC] });
    await useNotificationsStore.getState().seen(["a", "b"]);
    expect(core.notificationSeen).toHaveBeenCalledWith(["a", "b"]);
    const parts = useNotificationsStore.getState().items.map((item) => item.part);
    expect(parts).toEqual(["needs", "earlier", "finished"]);
    await useNotificationsStore.getState().seenAll();
    expect(core.notificationSeenAll).toHaveBeenCalledTimes(1);
    expect(counts(useNotificationsStore.getState().items)).toEqual({ needs: 1, finished: 0 });
  });

  it("reads the list again when the core would not mark a row seen", async () => {
    useNotificationsStore.setState({ items: [finishedB] });
    core.notificationSeen.mockRejectedValue({ code: "internal", message: "The disk is full." });
    core.notificationList.mockResolvedValue([finishedB]);
    await useNotificationsStore.getState().seen(["b"]);
    expect(useNotificationsStore.getState().error).toBe("The disk is full.");
    await vi.waitFor(() => expect(core.notificationList).toHaveBeenCalledTimes(1));
  });

  it("remembers how the view is grouped", () => {
    useNotificationsStore.getState().setGrouping("project");
    expect(useNotificationsStore.getState().grouping).toBe("project");
    expect(core.uiStateSave).toHaveBeenCalledWith(GROUPING_KEY, "project");
  });
});
