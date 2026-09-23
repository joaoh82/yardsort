import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DownloadProgress, UpdateStatus } from "@/lib/ipc";

const core = vi.hoisted(() => ({ updateCheck: vi.fn(), updateInstall: vi.fn() }));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
vi.mock("@tauri-apps/plugin-opener", () => opener);

import { useAppStore } from "@/stores/app";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { CHECK_EVERY_MS, FIRST_CHECK_MS, useUpdatesStore } from "@/stores/updates";
import { UpdateDialog } from "./UpdateDialog";
import { useUpdateChecks } from "./useUpdateChecks";

const status = (overrides: Partial<UpdateStatus> = {}): UpdateStatus => ({
  currentVersion: "0.3.0",
  installKind: "selfUpdating",
  available: {
    version: "0.3.1",
    notes: "- Fixes the thing\n- Adds the other thing",
    url: "https://github.com/joaoh82/yardsort/releases/tag/v0.3.1",
  },
  ...overrides,
});

const tab = (id: string, exited = false): TerminalTab => ({
  id,
  workspaceId: "ws",
  title: id,
  exit: exited ? { code: 0, success: true, signal: null } : null,
  recordId: null,
  busy: false,
  attention: false,
});

beforeEach(() => {
  vi.clearAllMocks();
  opener.openUrl.mockResolvedValue(undefined);
  useUpdatesStore.setState({
    status: null,
    checking: false,
    checkedAt: null,
    progress: null,
    installing: false,
    error: null,
    open: false,
  });
  useTerminalStore.setState({ tabs: [], active: {} });
});

describe("checking for updates", () => {
  afterEach(() => vi.useRealTimers());

  function Checks() {
    useUpdateChecks();
    return null;
  }
  const releaseBuild = { info: { debug: false } as never, checkForUpdates: true };

  it("looks shortly after start and once a day, quietly", async () => {
    vi.useFakeTimers();
    core.updateCheck.mockResolvedValue(status({ available: null }));
    useAppStore.setState(releaseBuild);
    render(<Checks />);

    expect(core.updateCheck).not.toHaveBeenCalled();
    await act(() => vi.advanceTimersByTimeAsync(FIRST_CHECK_MS));
    expect(core.updateCheck).toHaveBeenCalledTimes(1);
    await act(() => vi.advanceTimersByTimeAsync(CHECK_EVERY_MS));
    expect(core.updateCheck).toHaveBeenCalledTimes(2);
  });

  it("never looks when switched off, or in a development build", async () => {
    vi.useFakeTimers();
    useAppStore.setState({ ...releaseBuild, checkForUpdates: false });
    const { unmount } = render(<Checks />);
    await act(() => vi.advanceTimersByTimeAsync(CHECK_EVERY_MS));
    unmount();

    useAppStore.setState({ info: { debug: true } as never, checkForUpdates: true });
    render(<Checks />);
    await act(() => vi.advanceTimersByTimeAsync(CHECK_EVERY_MS));
    expect(core.updateCheck).not.toHaveBeenCalled();
  });

  it("keeps quiet when an automatic check fails, but tells you when you asked", async () => {
    core.updateCheck.mockRejectedValue({
      code: "update_failed",
      message: "Could not check for updates: offline",
    });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    await useUpdatesStore.getState().check();
    expect(useUpdatesStore.getState().error).toBeNull();

    await useUpdatesStore.getState().check({ manual: true });
    expect(useUpdatesStore.getState().error).toMatch(/offline/);
    warn.mockRestore();
  });

  it("looks again when the window comes back, but only once the last check is a day old", async () => {
    vi.useFakeTimers();
    core.updateCheck.mockResolvedValue(status({ available: null }));
    useAppStore.setState(releaseBuild);
    render(<Checks />);
    await act(() => vi.advanceTimersByTimeAsync(FIRST_CHECK_MS));
    expect(core.updateCheck).toHaveBeenCalledTimes(1);

    // Back at the window a minute later: nothing worth a request can have changed.
    await act(() => vi.advanceTimersByTimeAsync(60_000));
    await act(async () => void window.dispatchEvent(new Event("focus")));
    expect(core.updateCheck).toHaveBeenCalledTimes(1);

    // A day asleep leaves the interval pending, so coming back is when we find out.
    act(() => useUpdatesStore.setState({ checkedAt: Date.now() - CHECK_EVERY_MS }));
    await act(async () => void window.dispatchEvent(new Event("focus")));
    expect(core.updateCheck).toHaveBeenCalledTimes(2);
  });
});

describe("UpdateDialog", () => {
  const open = (s: UpdateStatus) => {
    useUpdatesStore.setState({ status: s, open: true });
    render(<UpdateDialog />);
    return userEvent.setup();
  };

  it("shows what is new and, before restarting, what that will interrupt", () => {
    useTerminalStore.setState({ tabs: [tab("a"), tab("b"), tab("done", true)] });
    open(status());

    const dialog = screen.getByRole("dialog", { name: "Yardsort 0.3.1 is available" });
    expect(dialog).toHaveTextContent("You have 0.3.0.");
    expect(dialog).toHaveTextContent("Fixes the thing");
    expect(dialog).toHaveTextContent("2 terminals are running.");
    expect(dialog).toHaveTextContent(/conversations are kept.*Resume/);
  });

  it("says nothing about terminals when none are running", () => {
    open(status());
    expect(screen.getByRole("dialog")).not.toHaveTextContent(/running/);
  });

  it("installs on request, showing progress, and cannot be dismissed half-way", async () => {
    let report: (progress: DownloadProgress) => void = () => {};
    core.updateInstall.mockImplementation((onProgress: typeof report) => {
      report = onProgress;
      return new Promise(() => {}); // the app restarts; this never resolves
    });
    const user = open(status());

    await user.click(screen.getByRole("button", { name: "Install and restart" }));
    act(() => report({ downloaded: 4_194_304, total: 8_388_608 }));

    expect(screen.getByRole("status")).toHaveTextContent("Downloading… 50% of 8.0 MB");
    expect(screen.getByRole("button", { name: "Installing…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Later" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(useUpdatesStore.getState().open).toBe(true);

    act(() => report({ downloaded: 8_388_608, total: 8_388_608 }));
    expect(screen.getByRole("status")).toHaveTextContent("Verifying and installing…");
  });

  it("reports a failed install and lets you try again or leave", async () => {
    core.updateInstall.mockRejectedValue({
      code: "update_failed",
      message: "Could not install the update: signature mismatch",
    });
    const user = open(status());
    await user.click(screen.getByRole("button", { name: "Install and restart" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("signature mismatch");
    expect(screen.getByRole("button", { name: "Install and restart" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Later" }));
    expect(useUpdatesStore.getState().open).toBe(false);
  });

  it("a copy owned by a package manager is told, not updated", async () => {
    const user = open(status({ installKind: "packageManager" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(/installed by a package manager/);
    expect(screen.queryByRole("button", { name: /Install/ })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Release page/ }));
    expect(opener.openUrl).toHaveBeenCalledWith(status().available!.url);
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(useUpdatesStore.getState().open).toBe(false);
  });
});
