import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ContextUsage } from "@/lib/ipc";
import type { TerminalTab } from "@/stores/terminals";

const core = vi.hoisted(() => ({
  sessionContext: vi.fn(),
  onActivityChanged: vi.fn(),
  ptyWrite: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { ContextBar } from "./ContextBar";
import { band, useDismissedContext } from "./contextHint";

const tab: TerminalTab = {
  id: "pty-1",
  workspaceId: "ws",
  title: "claude",
  exit: null,
  recordId: "rec-1",
  busy: false,
  attention: false,
};

const usage = (percent: number, compactCommand: string | null = "/compact"): ContextUsage => ({
  usedTokens: percent * 2000,
  windowTokens: 200_000,
  percent,
  suggest: percent >= 80 && compactCommand !== null,
  compactCommand,
});

let changed: (ids: string[]) => void = () => {};

beforeEach(() => {
  vi.clearAllMocks();
  useDismissedContext.setState({ byRecord: {} });
  core.ptyWrite.mockResolvedValue(undefined);
  core.onActivityChanged.mockImplementation(async (handler: (ids: string[]) => void) => {
    changed = handler;
    return () => {};
  });
});

describe("ContextBar", () => {
  it("stays hidden below the threshold and appears once a turn reports 80 %", async () => {
    core.sessionContext.mockResolvedValueOnce(usage(79)).mockResolvedValue(usage(84));
    render(<ContextBar tab={tab} />);
    await waitFor(() => expect(core.sessionContext).toHaveBeenCalledWith("rec-1"));
    expect(screen.queryByRole("button", { name: "Compact" })).toBeNull();

    // Another workspace's activity is not this conversation's.
    act(() => changed(["other"]));
    expect(core.sessionContext).toHaveBeenCalledTimes(1);
    act(() => changed(["ws"]));
    expect(await screen.findByText(/Context 84% full/)).toBeTruthy();
    expect(screen.getByText(/168,000 of 200,000 tokens/)).toBeTruthy();
  });

  it("types the harness's compact command, then Enter on its own", async () => {
    const user = userEvent.setup();
    core.sessionContext.mockResolvedValue(usage(86));
    render(<ContextBar tab={tab} />);
    await user.click(await screen.findByRole("button", { name: "Compact" }));
    await waitFor(() => expect(core.ptyWrite).toHaveBeenCalledTimes(2));
    expect(core.ptyWrite.mock.calls).toEqual([
      ["pty-1", "/compact"],
      ["pty-1", "\r"],
    ]);
    expect(screen.queryByRole("button", { name: "Compact" })).toBeNull();
  });

  it("once dismissed, comes back only at a higher band or after dropping below", async () => {
    const user = userEvent.setup();
    core.sessionContext.mockResolvedValue(usage(82));
    render(<ContextBar tab={tab} />);
    await user.click(await screen.findByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("status")).toBeNull();

    core.sessionContext.mockResolvedValue(usage(88));
    act(() => changed(["ws"]));
    await waitFor(() => expect(core.sessionContext).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("status")).toBeNull();

    core.sessionContext.mockResolvedValue(usage(91));
    act(() => changed(["ws"]));
    expect(await screen.findByText(/Context 91% full/)).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Not now" }));

    // Compacted: the reading is gone, and the next climb is news again.
    core.sessionContext.mockResolvedValue(null);
    act(() => changed(["ws"]));
    await waitFor(() => expect(useDismissedContext.getState().byRecord["rec-1"]).toBe(0));
    core.sessionContext.mockResolvedValue(usage(81));
    act(() => changed(["ws"]));
    expect(await screen.findByText(/Context 81% full/)).toBeTruthy();
  });

  it("keeps a dismissal across tab and workspace switches", async () => {
    const user = userEvent.setup();
    const other: TerminalTab = { ...tab, id: "pty-2", recordId: "rec-2" };
    // The other tab's reading arrives only when the test says so: until then, the bar holds
    // rec-1's reading while showing rec-2, and nothing about rec-2 is known.
    let release: (value: ContextUsage) => void = () => {};
    core.sessionContext.mockImplementation((id: string) =>
      id === "rec-2"
        ? new Promise<ContextUsage>((resolve) => (release = resolve))
        : Promise.resolve(usage(82)),
    );
    const { rerender, unmount } = render(<ContextBar tab={tab} />);
    await user.click(await screen.findByRole("button", { name: "Not now" }));
    expect(useDismissedContext.getState().byRecord["rec-1"]).toBe(80);

    rerender(<ContextBar tab={other} />);
    await waitFor(() => expect(core.sessionContext).toHaveBeenCalledWith("rec-2"));
    rerender(<ContextBar tab={tab} />);
    await act(async () => release(usage(10)));
    await waitFor(() => expect(core.sessionContext).toHaveBeenCalledTimes(3));
    expect(useDismissedContext.getState().byRecord["rec-1"]).toBe(80);
    expect(screen.queryByRole("status")).toBeNull();

    // Another workspace and back: the bar mounts afresh with no reading yet.
    unmount();
    render(<ContextBar tab={tab} />);
    await waitFor(() => expect(core.sessionContext).toHaveBeenCalledTimes(4));
    expect(useDismissedContext.getState().byRecord["rec-1"]).toBe(80);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("does not offer to compact while the agent is in the middle of a turn", async () => {
    core.sessionContext.mockResolvedValue(usage(86));
    const { rerender } = render(<ContextBar tab={{ ...tab, busy: true }} />);
    expect(await screen.findByRole("button", { name: "Compact" })).toBeDisabled();
    rerender(<ContextBar tab={tab} />);
    expect(screen.getByRole("button", { name: "Compact" })).toBeEnabled();
  });

  it("offers nothing to a shell, an ended agent, or a harness with no compact command", async () => {
    core.sessionContext.mockResolvedValue(usage(90, null));
    const { rerender } = render(<ContextBar tab={tab} />);
    await waitFor(() => expect(core.sessionContext).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();

    core.sessionContext.mockClear();
    core.sessionContext.mockResolvedValue(usage(90));
    rerender(<ContextBar tab={{ ...tab, recordId: null }} />);
    rerender(<ContextBar tab={{ ...tab, exit: { code: 0, signal: null, success: true } }} />);
    expect(core.sessionContext).not.toHaveBeenCalled();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("steps through bands", () => {
    expect([0, 79, 80, 89, 90, 94, 95, 100].map(band)).toEqual([0, 0, 80, 80, 90, 90, 95, 95]);
  });
});
