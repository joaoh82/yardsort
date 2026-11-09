import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { CoordinatorStatus, HostEvent, SessionInfo } from "@/lib/ipc";
import { harness } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  coordinatorStatus: vi.fn(),
  coordinatorStart: vi.fn(),
  coordinatorStop: vi.fn(),
  uiStateLoad: vi.fn(),
  uiStateSave: vi.fn(),
  harnessesList: vi.fn(),
  onHostEvent: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
// xterm needs a real layout engine; what matters here is which session is shown, and how.
vi.mock("@/features/terminal/TerminalView", () => ({
  TerminalView: (props: { sessionId: string; agent?: boolean }) => (
    <div data-testid="terminal" data-agent={String(!!props.agent)}>
      {props.sessionId}
    </div>
  ),
}));

import { runCommand } from "@/features/keyboard/commands";
import { StatusBar } from "@/features/shell/StatusBar";
import { HARNESS_KEY, useCoordinatorStore } from "@/stores/coordinator";
import { useHarnessStore } from "@/stores/harnesses";
import { useLayoutStore } from "@/stores/layout";
import { FloorManagerPanel } from "./FloorManager";

const session = (id: string, running: boolean): SessionInfo => ({
  id,
  program: "claude",
  args: [],
  cwd: "/profile/coordinator",
  pid: 7,
  size: { cols: 80, rows: 30 },
  labels: { coordinator: "claude", harness: "claude" },
  state: running
    ? { status: "running" }
    : { status: "exited", exit: { code: 0, success: true, signal: null } },
  hasOutput: true,
  busy: false,
  idleMs: 0,
  busyMs: 0,
});

const status = (current: SessionInfo | null, extra: Partial<CoordinatorStatus> = {}) => ({
  session: current,
  harness: current ? "claude" : null,
  folder: "/profile/coordinator",
  hasYs: true,
  ...extra,
});

let hostEvent: ((event: HostEvent) => void) | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  useCoordinatorStore.setState({
    open: false,
    status: null,
    chosen: null,
    busy: false,
    error: null,
  });
  useHarnessStore.setState({ harnesses: [], loaded: false });
  useLayoutStore.setState({ settingsOpen: false, settingsSection: "harnesses" });
  core.coordinatorStatus.mockResolvedValue(status(null));
  core.uiStateLoad.mockResolvedValue({});
  core.uiStateSave.mockResolvedValue(undefined);
  core.harnessesList.mockResolvedValue([
    harness("claude"),
    harness("codex"),
    harness("grok", { resolvedPath: null }),
    harness("pi", { enabled: false }),
  ]);
  core.onHostEvent.mockImplementation((handler: (event: HostEvent) => void) => {
    hostEvent = handler;
    return Promise.resolve(() => {});
  });
});

const openPanel = async () => {
  await act(async () => useCoordinatorStore.getState().setOpen(true));
  render(<FloorManagerPanel />);
  return screen.findByRole("complementary", { name: "Floor Manager" });
};

it("opens from the status bar and from its shortcut, and closing leaves it running", async () => {
  core.coordinatorStatus.mockResolvedValue(status(session("s1", true)));
  render(<StatusBar />);
  const button = screen.getByRole("button", { name: /Floor Manager/ });
  expect(button).toHaveAttribute("aria-pressed", "false");
  await userEvent.click(button);
  expect(useCoordinatorStore.getState().open).toBe(true);
  expect(button).toHaveAttribute("aria-pressed", "true");

  act(() => runCommand("floorManager"));
  expect(useCoordinatorStore.getState().open).toBe(false);
  act(() => runCommand("floorManager"));
  expect(useCoordinatorStore.getState().open).toBe(true);

  render(<FloorManagerPanel />);
  await userEvent.click(await screen.findByRole("button", { name: "Close the Floor Manager" }));
  expect(useCoordinatorStore.getState().open).toBe(false);
  expect(core.coordinatorStop).not.toHaveBeenCalled();
});

it("starts with an agent that can run, the one chosen last time first", async () => {
  core.uiStateLoad.mockResolvedValue({ [HARNESS_KEY]: "codex" });
  core.coordinatorStart.mockResolvedValue(status(session("s2", true), { harness: "codex" }));
  const panel = await openPanel();
  expect(within(panel).getByText("One agent that sees every project.")).toBeInTheDocument();

  const picker = await within(panel).findByRole("combobox", { name: "Agent" });
  // Neither one that is not installed nor one switched off is offered.
  expect(
    within(picker)
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual(["CLAUDE", "CODEX"]);
  await vi.waitFor(() => expect(picker).toHaveValue("codex"));

  await userEvent.click(within(panel).getByRole("button", { name: "Start" }));
  expect(core.coordinatorStart).toHaveBeenCalledWith("codex", { cols: 80, rows: 30 });
  const terminal = await within(panel).findByTestId("terminal");
  expect(terminal).toHaveTextContent("s2");
  expect(terminal).toHaveAttribute("data-agent", "true");
  expect(within(panel).getByText("codex · running")).toBeInTheDocument();
  expect(core.uiStateSave).toHaveBeenCalledWith(HARNESS_KEY, "codex");
});

it("stops only after a second, explicit yes, and a no keeps it running", async () => {
  core.coordinatorStatus.mockResolvedValue(status(session("s1", true)));
  core.coordinatorStop.mockResolvedValue(status(session("s1", false)));
  const panel = await openPanel();

  await userEvent.click(await within(panel).findByRole("button", { name: "Stop" }));
  expect(within(panel).getByText("End its conversation?")).toBeInTheDocument();
  await userEvent.click(within(panel).getByRole("button", { name: "Keep it" }));
  expect(core.coordinatorStop).not.toHaveBeenCalled();
  expect(within(panel).queryByText("End its conversation?")).toBeNull();

  await userEvent.click(within(panel).getByRole("button", { name: "Stop" }));
  await userEvent.click(within(panel).getByRole("button", { name: "End it" }));
  expect(core.coordinatorStop).toHaveBeenCalledTimes(1);
  expect(await within(panel).findByText("The Floor Manager has ended.")).toBeInTheDocument();
  // Its last screen is still there to read.
  expect(within(panel).getByTestId("terminal")).toHaveTextContent("s1");
});

it("says when it has ended by itself, and starts again from there", async () => {
  core.coordinatorStatus.mockResolvedValue(status(session("s1", true)));
  const panel = await openPanel();
  expect(await within(panel).findByText("claude · running")).toBeInTheDocument();

  core.coordinatorStatus.mockResolvedValue(status(session("s1", false)));
  await act(async () =>
    hostEvent!({ type: "exited", id: "s1", exit: { code: 0, success: true, signal: null } }),
  );
  expect(await within(panel).findByText("The Floor Manager has ended.")).toBeInTheDocument();
  expect(within(panel).queryByRole("button", { name: "Stop" })).toBeNull();

  core.coordinatorStart.mockResolvedValue(status(session("s3", true)));
  await userEvent.click(within(panel).getByRole("button", { name: "Start again" }));
  expect(core.coordinatorStart).toHaveBeenCalledWith("claude", { cols: 80, rows: 30 });
  expect(await within(panel).findByTestId("terminal")).toHaveTextContent("s3");
});

it("sends you to the harnesses when no agent can run it", async () => {
  core.harnessesList.mockResolvedValue([harness("claude", { resolvedPath: null })]);
  const panel = await openPanel();
  await userEvent.click(await within(panel).findByRole("button", { name: "Settings → Harnesses" }));
  expect(useLayoutStore.getState()).toMatchObject({
    settingsOpen: true,
    settingsSection: "harnesses",
  });
  expect(core.coordinatorStart).not.toHaveBeenCalled();
});

it("shows what the core said went wrong", async () => {
  core.coordinatorStart.mockRejectedValue({
    code: "program_not_found",
    message: "`claude` was not found on PATH.",
  });
  const panel = await openPanel();
  await userEvent.click(await within(panel).findByRole("button", { name: "Start" }));
  expect(await within(panel).findByRole("alert")).toHaveTextContent("was not found on PATH");
});
