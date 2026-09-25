import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  settingsSaveActivity: vi.fn(),
  activityDiagnostics: vi.fn(),
  activityClear: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { useAppStore } from "@/stores/app";
import { ActivitySettings } from "./ActivitySettings";

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ showTimeline: false });
  core.activityDiagnostics.mockResolvedValue({
    events: 12,
    runs: 5,
    spoolDir: "/tmp/ys/activity/spool",
    spoolPending: 1,
    inboxDir: "/tmp/ys/activity/inbox",
    inboxPending: 3,
    claudeHooksFile: "/tmp/ys/activity/hooks/claude.json",
    counters: [{ name: "write_failed", count: 2, lastAt: 0, lastDetail: "disk full" }],
  });
  core.activityClear.mockResolvedValue(undefined);
});

describe("ActivitySettings", () => {
  it("saves the switches and lets the app know the timeline is on", async () => {
    const user = userEvent.setup();
    core.settingsSaveActivity.mockImplementation(async (activity) => ({
      activity,
    }));
    render(
      <ActivitySettings
        initial={{
          recordLifecycle: true,
          showTimeline: false,
          captureClaude: false,
          captureCodex: false,
        }}
      />,
    );

    const save = screen.getByRole("button", { name: "Save activity settings" });
    expect(save).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: /Show the activity timeline/ }));
    await user.click(screen.getByRole("checkbox", { name: /Capture what Claude Code reports/ }));
    await user.click(screen.getByRole("checkbox", { name: /Capture what Codex reports/ }));
    await user.click(save);
    expect(core.settingsSaveActivity).toHaveBeenCalledWith({
      recordLifecycle: true,
      showTimeline: true,
      captureClaude: true,
      captureCodex: true,
    });
    await waitFor(() => expect(screen.getByText("Saved.")).toBeInTheDocument());
    expect(useAppStore.getState().showTimeline).toBe(true);
  });

  it("cannot capture what Claude Code reports without recording at all", async () => {
    const user = userEvent.setup();
    render(
      <ActivitySettings
        initial={{
          recordLifecycle: true,
          showTimeline: false,
          captureClaude: true,
          captureCodex: true,
        }}
      />,
    );
    const capture = screen.getByRole("checkbox", { name: /Capture what Claude Code reports/ });
    const codex = screen.getByRole("checkbox", { name: /Capture what Codex reports/ });
    expect(capture).toBeEnabled();
    expect(codex).toBeEnabled();
    await user.click(screen.getByRole("checkbox", { name: /Record when agents start and exit/ }));
    expect(capture).toBeDisabled();
    expect(codex).toBeDisabled();
  });

  it("shows what is recorded and what went wrong, and can clear all of it", async () => {
    const user = userEvent.setup();
    render(
      <ActivitySettings
        initial={{
          recordLifecycle: true,
          showTimeline: false,
          captureClaude: false,
          captureCodex: false,
        }}
      />,
    );
    await waitFor(() => expect(screen.getByText(/12 events across 5 runs/)).toBeInTheDocument());
    expect(screen.getByText(/1 exit\(s\) waiting in the spool/)).toBeInTheDocument();
    expect(screen.getByText(/3 reported event\(s\) waiting in the inbox/)).toBeInTheDocument();
    expect(screen.getByText(/write_failed: 2 — disk full/)).toBeInTheDocument();
    expect(screen.getByTitle("/tmp/ys/activity/hooks/claude.json")).toBeInTheDocument();

    core.activityDiagnostics.mockResolvedValue({
      events: 0,
      runs: 0,
      spoolDir: "/tmp/ys/activity/spool",
      spoolPending: 0,
      inboxDir: "/tmp/ys/activity/inbox",
      inboxPending: 0,
      claudeHooksFile: "/tmp/ys/activity/hooks/claude.json",
      counters: [],
    });
    await user.click(screen.getByRole("button", { name: "Clear all recorded activity" }));
    expect(core.activityClear).toHaveBeenCalledWith(null);
    await waitFor(() => expect(screen.getByText(/0 events across 0 runs/)).toBeInTheDocument());
  });
});
