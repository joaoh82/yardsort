import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  ptyList: vi.fn(),
  onHostEvent: vi.fn(),
  onSessionStarted: vi.fn(),
  notificationList: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useTerminalStore } from "@/stores/terminals";
import { useNotificationsStore } from "@/stores/notifications";
import { notification } from "@/test/fixtures";
import { handleHostEvent, useTerminalSessions } from "./useTerminalSessions";

function session(id: string, workspace: string): SessionInfo {
  return {
    id,
    program: "claude",
    args: [],
    cwd: null,
    pid: 1,
    size: { cols: 120, rows: 30 },
    labels: { workspace, record: `record-${id}` },
    state: { status: "running" },
    hasOutput: true,
    busy: true,
    idleMs: 0,
    busyMs: 0,
  };
}

function Probe() {
  useTerminalSessions();
  return null;
}

describe("useTerminalSessions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useTerminalStore.setState({ tabs: [], active: {}, error: null });
    core.onHostEvent.mockResolvedValue(() => {});
    core.onSessionStarted.mockResolvedValue(() => {});
  });

  it("gives a session a workflow started a tab, without taking over the one on screen", async () => {
    core.ptyList.mockResolvedValueOnce([session("mine", "ws")]);
    render(<Probe />);
    await vi.waitFor(() => expect(useTerminalStore.getState().tabs).toHaveLength(1));
    const [started] = core.onSessionStarted.mock.calls[0]!;

    core.ptyList.mockResolvedValueOnce([session("mine", "ws"), session("reviewer", "ws")]);
    started("ws");
    await vi.waitFor(() => expect(useTerminalStore.getState().tabs).toHaveLength(2));
    const { tabs, active } = useTerminalStore.getState();
    expect(tabs.map((t) => t.id)).toEqual(["mine", "reviewer"]);
    expect(active).toEqual({ ws: "mine" });
  });

  it("asks the core what an agent finished when its terminal goes quiet, and not for a shell", async () => {
    const tab = (id: string, recordId: string | null) => ({
      id,
      workspaceId: "ws",
      title: id,
      exit: null,
      recordId,
      busy: true,
      attention: false,
      toldAbout: null,
      toldAt: null,
      busySince: null,
    });
    useTerminalStore.setState({ tabs: [tab("agent", "record-1"), tab("shell", null)] });
    useNotificationsStore.setState({ items: [], loaded: false, inFront: null });
    core.notificationList.mockResolvedValue([
      notification("n1", { kind: "agent.finished", part: "finished", terminalId: "agent" }),
    ]);

    handleHostEvent({ type: "quiet", id: "shell", busyMs: 60_000 });
    expect(core.notificationList).not.toHaveBeenCalled();

    handleHostEvent({ type: "quiet", id: "agent", busyMs: 60_000 });
    await vi.waitFor(() => expect(useNotificationsStore.getState().items).toHaveLength(1));
    // The ring is the list's to draw: on the agent's tab, since the core says nobody saw it.
    const ringed = useTerminalStore.getState().tabs.filter((t) => t.attention);
    expect(ringed.map((t) => t.id)).toEqual(["agent"]);
  });
});
