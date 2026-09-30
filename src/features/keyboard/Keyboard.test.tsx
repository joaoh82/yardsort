import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_BINDINGS } from "@/lib/shortcuts";
import { useLayoutStore } from "@/stores/layout";
import { usePreferencesStore } from "@/stores/preferences";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";
import { project, worktree } from "@/test/fixtures";
import { CommandPalette } from "./CommandPalette";
import { useAppShortcuts } from "./useAppShortcuts";
import { KeyboardSettings } from "@/features/settings/KeyboardSettings";

const core = vi.hoisted(() => ({
  uiStateSave: vi.fn().mockResolvedValue(undefined),
  uiStateLoad: vi.fn(),
  ptyClose: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
const enter = vi.hoisted(() => vi.fn());
vi.mock("@/features/sidebar/actions", () => ({
  enterWorkspace: enter,
  composeInCurrentProject: vi.fn(),
  openProjectFromDisk: vi.fn(),
}));
function Host() {
  useAppShortcuts();
  const open = useLayoutStore((s) => s.paletteOpen);
  return (
    <>
      <button>Background</button>
      {open && <CommandPalette />}
    </>
  );
}
const mod = { ctrlKey: true, shiftKey: true };
beforeEach(() => {
  vi.clearAllMocks();
  usePreferencesStore.setState({
    loaded: true,
    bindings: { ...DEFAULT_BINDINGS },
    error: null,
    saving: false,
  });
  useLayoutStore.setState({
    paletteOpen: false,
    settingsOpen: false,
    settingsSection: "harnesses",
    collapsed: { left: false, right: false },
  });
  useProjectsStore.setState({
    projects: [project("demo")],
    selectedWorkspaceId: "w-demo",
    composingProjectId: null,
    composingWorkspaceId: null,
    workflowId: null,
  });
  useTerminalStore.setState({ tabs: [], active: {} });
});

describe("keyboard navigation", () => {
  it("opens and searches the palette from the keyboard and returns focus on Escape", async () => {
    const user = userEvent.setup();
    render(<Host />);
    screen.getByRole("button", { name: "Background" }).focus();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.keyDown(window, { key: "K", ...mod });
    const input = screen.getByRole("combobox");
    expect(input).toHaveFocus();
    await user.type(input, "demo");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    await user.keyboard("{Enter}");
    await waitFor(() => expect(enter).toHaveBeenCalledWith("w-demo", false));
    fireEvent.keyDown(window, { key: "K", ...mod });
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Background" })).toHaveFocus();
  });
  it("does not run app commands behind a dialog or on repeated keys", () => {
    render(<Host />);
    fireEvent.keyDown(window, { key: "B", ...mod, repeat: true });
    expect(useLayoutStore.getState().collapsed.left).toBe(false);
    fireEvent.keyDown(window, { key: "K", ...mod });
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "B", ...mod });
    expect(useLayoutStore.getState().collapsed.left).toBe(false);
  });
  it("cycles only usable workspaces and wraps", () => {
    useProjectsStore.setState({
      projects: [
        project("demo", {
          workspaces: [
            worktree("demo", "one"),
            worktree("demo", "archived", { archived: true }),
            worktree("demo", "missing", { missing: true }),
            worktree("demo", "two"),
          ],
        }),
      ],
      selectedWorkspaceId: "w-demo-two",
    });
    render(<Host />);
    fireEvent.keyDown(window, { key: "ArrowDown", ...mod });
    expect(enter).toHaveBeenLastCalledWith("w-demo-one", true);
    fireEvent.keyDown(window, { key: "ArrowUp", ...mod });
    expect(enter).toHaveBeenLastCalledWith("w-demo-one", true);
  });
  it("switches terminal tabs without closing them", () => {
    useTerminalStore.setState({
      tabs: ["one", "two"].map((id) => ({
        id,
        workspaceId: "w-demo",
        title: id,
        exit: null,
        recordId: null,
        busy: false,
        attention: false,
      })),
      active: { "w-demo": "one" },
    });
    render(<Host />);
    fireEvent.keyDown(window, { key: "ArrowLeft", ...mod });
    expect(useTerminalStore.getState().active["w-demo"]).toBe("two");
    expect(core.ptyClose).not.toHaveBeenCalled();
  });
  it("focuses and reveals a collapsed panel", async () => {
    useLayoutStore.setState({ collapsed: { left: true, right: false } });
    render(
      <>
        <Host />
        <div data-navigation="Projects">
          <button>Project control</button>
        </div>
      </>,
    );
    fireEvent.keyDown(window, { key: "L", ...mod });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Project control" })).toHaveFocus(),
    );
    expect(useLayoutStore.getState().collapsed.left).toBe(false);
  });
});

describe("shortcut configuration", () => {
  it("records, persists and reloads a custom binding and uses it immediately", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Host />
        <KeyboardSettings />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "Binding for Toggle projects panel" }));
    fireEvent.keyDown(screen.getByRole("button", { name: "Binding for Toggle projects panel" }), {
      key: "J",
      ...mod,
    });
    await user.click(screen.getByRole("button", { name: "Save shortcuts" }));
    expect(core.uiStateSave).toHaveBeenCalledWith(
      "keyboard.bindings",
      expect.stringContaining('"toggleLeft":"j"'),
    );
    fireEvent.keyDown(window, { key: "B", ...mod });
    expect(useLayoutStore.getState().collapsed.left).toBe(false);
    fireEvent.keyDown(window, { key: "J", ...mod });
    expect(useLayoutStore.getState().collapsed.left).toBe(true);
    core.uiStateLoad.mockResolvedValue({
      "keyboard.bindings": JSON.stringify(usePreferencesStore.getState().bindings),
    });
    await act(async () => {
      usePreferencesStore.setState({ bindings: { ...DEFAULT_BINDINGS } });
      await usePreferencesStore.getState().load();
    });
    expect(usePreferencesStore.getState().bindings.toggleLeft).toBe("j");
  });
  it("blocks duplicate and clipboard shortcuts, and Escape cancels recording", async () => {
    const user = userEvent.setup();
    render(<KeyboardSettings />);
    const button = screen.getByRole("button", { name: "Binding for Toggle projects panel" });
    await user.click(button);
    fireEvent.keyDown(button, { key: "C", ...mod });
    expect(screen.getByRole("status")).toHaveTextContent("reserved");
    fireEvent.keyDown(button, { key: "Escape" });
    expect(button).toHaveTextContent("Ctrl+Shift+B");
    await user.click(button);
    fireEvent.keyDown(button, { key: "K", ...mod });
    expect(screen.getByRole("alert")).toHaveTextContent("conflicts");
    expect(screen.getByRole("button", { name: "Save shortcuts" })).toBeDisabled();
    expect(core.uiStateSave).not.toHaveBeenCalled();
  });
  it("leaves active bindings unchanged when the core refuses a save", async () => {
    core.uiStateSave.mockRejectedValueOnce(new Error("Disk full"));
    const user = userEvent.setup();
    render(<KeyboardSettings />);
    await user.click(screen.getByRole("button", { name: "Clear Toggle projects panel" }));
    await user.click(screen.getByRole("button", { name: "Save shortcuts" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
    expect(usePreferencesStore.getState().bindings.toggleLeft).toBe("b");
  });
});
