import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { record } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  sessionsList: vi.fn(),
  sessionResume: vi.fn(),
  sessionFork: vi.fn(),
  sessionForget: vi.fn(),
  ptyClose: vi.fn(),
}));
const native = vi.hoisted(() => ({ notify: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@/lib/native", () => ({ native }));

import { useAppStore } from "@/stores/app";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { project } from "@/test/fixtures";
import { EndedBar } from "./EndedBar";
import { SessionHistory } from "./SessionHistory";
import { describeEnd, timeAgo } from "./sessionText";
import { handleHostEvent } from "./useTerminalSessions";

const live = (id: string, recordId: string) => ({
  id,
  program: "claude",
  args: [],
  cwd: null,
  pid: 1,
  size: { cols: 80, rows: 24 },
  labels: { workspace: "ws", harness: "claude", record: recordId },
  state: { status: "running" as const },
  hasOutput: true,
  busy: false,
  idleMs: 0,
});

const tab = (id: string, extra: Partial<TerminalTab> = {}): TerminalTab => ({
  id,
  workspaceId: "ws",
  title: "claude",
  exit: null,
  recordId: null,
  busy: false,
  attention: false,
  toldAbout: null,
  toldAt: null,
  ...extra,
});

beforeEach(() => {
  vi.clearAllMocks();
  core.ptyClose.mockResolvedValue(undefined);
  core.sessionForget.mockResolvedValue(undefined);
  native.notify.mockResolvedValue(undefined);
  useTerminalStore.setState({ tabs: [], active: {}, error: null });
  useSessionsStore.setState({ byWorkspace: {}, error: null });
});

describe("SessionHistory", () => {
  it("lists past conversations, newest emphasised, and resumes one in a click", async () => {
    const user = userEvent.setup();
    useSessionsStore.setState({
      byWorkspace: {
        ws: [
          record("new", { title: "add dark mode", interrupted: true, exitCode: null }),
          record("old", { title: "fix the login bug", model: "opus" }),
        ],
      },
    });
    core.sessionResume.mockResolvedValue(live("pty-1", "new"));
    render(<SessionHistory workspaceId="ws" />);

    const items = within(screen.getByRole("region", { name: "Previous sessions" })).getAllByRole(
      "listitem",
    );
    expect(items[0]).toHaveTextContent("Claude Code — add dark mode");
    expect(items[0]).toHaveTextContent("interrupted when Yardsort closed");
    expect(items[1]).toHaveTextContent("opus");

    await user.click(within(items[0]!).getByRole("button", { name: "Resume" }));
    expect(core.sessionResume).toHaveBeenCalledWith("new", expect.anything());
    expect(useTerminalStore.getState().tabs[0]).toMatchObject({ id: "pty-1", recordId: "new" });
  });

  it("leaves out conversations that are running or already open in a tab", () => {
    useSessionsStore.setState({
      byWorkspace: { ws: [record("running", { running: true }), record("open"), record("past")] },
    });
    useTerminalStore.setState({ tabs: [tab("t", { recordId: "open" })] });
    render(<SessionHistory workspaceId="ws" />);
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("explains a conversation that cannot be continued instead of offering dead buttons", () => {
    useSessionsStore.setState({
      byWorkspace: {
        ws: [
          record("stale", {
            resumable: false,
            forkable: false,
            unavailableReason: "Codex can only continue its most recent conversation in a folder.",
          }),
        ],
      },
    });
    render(<SessionHistory workspaceId="ws" />);
    expect(screen.getByRole("button", { name: "Resume" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Fork" })).toBeDisabled();
    expect(screen.getByRole("listitem")).toHaveTextContent(/only continue its most recent/);
  });

  it("forgets a conversation and reloads the list", async () => {
    const user = userEvent.setup();
    useSessionsStore.setState({ byWorkspace: { ws: [record("r1")] } });
    core.sessionsList.mockResolvedValue([]);
    render(<SessionHistory workspaceId="ws" />);
    await user.click(screen.getByRole("button", { name: /Forget Claude Code session/ }));
    expect(core.sessionForget).toHaveBeenCalledWith("r1");
    await vi.waitFor(() => expect(screen.queryByRole("region")).not.toBeInTheDocument());
  });

  it("renders nothing for a workspace without a past", () => {
    const { container } = render(<SessionHistory workspaceId="ws" />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("EndedBar", () => {
  const ended = tab("old", { recordId: "r1", exit: { code: 0, success: true, signal: null } });

  it("offers to carry on where an ended agent left off, replacing its dead terminal", async () => {
    const user = userEvent.setup();
    useSessionsStore.setState({ byWorkspace: { ws: [record("r1")] } });
    useTerminalStore.setState({ tabs: [ended], active: { ws: "old" } });
    core.sessionResume.mockResolvedValue(live("new", "r1"));
    render(<EndedBar tab={ended} />);

    expect(screen.getByRole("status")).toHaveTextContent("Claude Code ended.");
    await user.click(screen.getByRole("button", { name: "Resume" }));
    expect(useTerminalStore.getState().tabs.map((t) => t.id)).toEqual(["new"]);
  });

  it("stays out of the way for shells and for agents still running", () => {
    useSessionsStore.setState({ byWorkspace: { ws: [record("r1")] } });
    const { container, rerender } = render(<EndedBar tab={tab("shell", { exit: ended.exit })} />);
    expect(container).toBeEmptyDOMElement();
    rerender(<EndedBar tab={tab("agent", { recordId: "r1" })} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("host events", () => {
  const focus = (focused: boolean) => vi.spyOn(document, "hasFocus").mockReturnValue(focused);

  beforeEach(() => {
    useProjectsStore.setState({
      projects: [project("app", { workspaces: [{ ...project("app").workspaces[0]!, id: "ws" }] })],
      selectedWorkspaceId: "ws",
      composingProjectId: null,
    });
    useTerminalStore.setState({
      tabs: [tab("agent", { recordId: "r1" })],
      active: { ws: "agent" },
    });
    useAppStore.setState({ notifyWhenQuiet: true });
  });

  it("notifies when an agent finishes long work while you are in another window", () => {
    focus(false);
    handleHostEvent({ type: "busy", id: "agent" });
    expect(useTerminalStore.getState().tabs[0]!.busy).toBe(true);
    handleHostEvent({ type: "quiet", id: "agent", busyMs: 45_000 });
    expect(native.notify).toHaveBeenCalledWith("claude is waiting", "app / local");
    expect(useTerminalStore.getState().tabs[0]).toMatchObject({ busy: false, attention: true });
  });

  it("stays silent when you are watching it, when it was brief, or when you turned it off", () => {
    focus(true);
    handleHostEvent({ type: "quiet", id: "agent", busyMs: 45_000 });
    focus(false);
    handleHostEvent({ type: "quiet", id: "agent", busyMs: 900 });
    useAppStore.setState({ notifyWhenQuiet: false });
    handleHostEvent({ type: "quiet", id: "agent", busyMs: 45_000 });
    expect(native.notify).not.toHaveBeenCalled();
    // Turning notifications off does not hide the mark in the app itself.
    expect(useTerminalStore.getState().tabs[0]!.attention).toBe(true);
  });

  it("marks — without a notification — work finished in a tab you are not looking at", () => {
    focus(true);
    useTerminalStore.setState({ active: { ws: "some-other-tab" } });
    handleHostEvent({ type: "quiet", id: "agent", busyMs: 45_000 });
    expect(useTerminalStore.getState().tabs[0]!.attention).toBe(true);
    expect(native.notify).not.toHaveBeenCalled();
  });

  it("refreshes the history when the exit is for a tab that was already closed", () => {
    core.sessionsList.mockResolvedValue([record("r1")]);
    useSessionsStore.setState({ byWorkspace: { ws: [record("r1", { running: true })] } });
    useTerminalStore.setState({ tabs: [], active: {} });
    handleHostEvent({
      type: "exited",
      id: "closed-already",
      exit: { code: 1, success: false, signal: null },
    });
    expect(core.sessionsList).toHaveBeenCalledWith("ws");
  });

  it("refreshes the history when an agent exits, so Resume is on offer at once", () => {
    core.sessionsList.mockResolvedValue([record("r1")]);
    handleHostEvent({
      type: "exited",
      id: "agent",
      exit: { code: 0, success: true, signal: null },
    });
    expect(core.sessionsList).toHaveBeenCalledWith("ws");
  });
});

describe("session wording", () => {
  it("tells time coarsely", () => {
    const now = 1_800_000_000_000;
    const ago = (ms: number) => timeAgo(now - ms, now);
    expect([
      ago(20_000),
      ago(5 * 60_000),
      ago(3 * 3_600_000),
      ago(26 * 3_600_000),
      ago(4 * 86_400_000),
    ]).toEqual(["just now", "5 min ago", "3 h ago", "yesterday", "4 days ago"]);
  });

  it("distinguishes a clean end, a failure and an interruption", () => {
    expect(describeEnd(record("a"))).toBe("ended");
    expect(describeEnd(record("b", { exitCode: 2 }))).toBe("exited with code 2");
    expect(describeEnd(record("d", { exitCode: 129 }))).toBe("stopped");
    expect(describeEnd(record("c", { interrupted: true, exitCode: null }))).toMatch(/interrupted/);
  });
});
