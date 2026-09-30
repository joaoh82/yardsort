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
    bindingNotice: null,
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
  it.each(["textarea", "input", "select", "contenteditable"])(
    "keeps modified arrows in %s controls without leaving the composer",
    (kind) => {
      useProjectsStore.setState({
        composingProjectId: "p-demo",
        projects: [project("demo"), project("other")],
      });
      render(
        <>
          <Host />
          {kind === "textarea" ? (
            <textarea aria-label="Editor" defaultValue="Keep this draft" />
          ) : kind === "input" ? (
            <input aria-label="Editor" defaultValue="Keep this draft" />
          ) : kind === "select" ? (
            <select aria-label="Editor">
              <option>Keep this draft</option>
            </select>
          ) : (
            <div contentEditable suppressContentEditableWarning>
              <span aria-label="Editor">Keep this draft</span>
            </div>
          )}
        </>,
      );
      const editor = screen.getByLabelText("Editor");
      for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]) {
        const event = new KeyboardEvent("keydown", {
          key,
          ...mod,
          bubbles: true,
          cancelable: true,
        });
        fireEvent(editor, event);
        expect(event.defaultPrevented).toBe(false);
      }
      expect(enter).not.toHaveBeenCalled();
      expect(useProjectsStore.getState().composingProjectId).toBe("p-demo");
      if (kind === "textarea" || kind === "input") expect(editor).toHaveValue("Keep this draft");
    },
  );
  it("still dispatches modified arrows from xterm's helper textarea", () => {
    render(
      <>
        <Host />
        <textarea className="xterm-helper-textarea" aria-label="Terminal input" />
      </>,
    );
    const event = new KeyboardEvent("keydown", {
      key: "ArrowDown",
      ...mod,
      bubbles: true,
      cancelable: true,
    });
    fireEvent(screen.getByLabelText("Terminal input"), event);
    expect(enter).toHaveBeenCalledWith("w-demo", false);
    expect(event.defaultPrevented).toBe(true);
  });
  it("leaves disabled command keys unconsumed", () => {
    useProjectsStore.setState({ composingProjectId: "p-demo" });
    render(<Host />);
    for (const key of ["ArrowLeft", "ArrowRight", "T", "W"]) {
      const event = new KeyboardEvent("keydown", { key, ...mod, bubbles: true, cancelable: true });
      fireEvent(screen.getByRole("button", { name: "Background" }), event);
      expect(event.defaultPrevented).toBe(false);
    }
    expect(core.ptyClose).not.toHaveBeenCalled();
  });
  it("keeps the command palette available from text fields", () => {
    render(
      <>
        <Host />
        <input aria-label="Search" />
      </>,
    );
    fireEvent.keyDown(screen.getByLabelText("Search"), { key: "K", ...mod });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
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
  it.each(["bad json", JSON.stringify({ toggleLeft: "k", toggleRight: "j" })])(
    "shows recovered binding notices and only clears them after saving (%s)",
    async (raw) => {
      core.uiStateLoad.mockResolvedValue({ "keyboard.bindings": raw });
      await usePreferencesStore.getState().load();
      const user = userEvent.setup();
      render(<KeyboardSettings />);
      expect(screen.getByRole("status")).toHaveTextContent(/Saved shortcuts|left unassigned/);
      if (raw !== "bad json") {
        expect(usePreferencesStore.getState().bindings).toMatchObject({
          palette: null,
          toggleLeft: "k",
          toggleRight: "j",
        });
      }
      core.uiStateSave.mockRejectedValueOnce(new Error("Disk full"));
      await user.click(screen.getByRole("button", { name: "Save shortcuts" }));
      expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
      expect(usePreferencesStore.getState().bindingNotice).not.toBeNull();
      await user.click(screen.getByRole("button", { name: "Save shortcuts" }));
      expect(usePreferencesStore.getState().bindingNotice).toBeNull();
      const stored = core.uiStateSave.mock.lastCall![1];
      core.uiStateLoad.mockResolvedValue({ "keyboard.bindings": stored });
      await act(() => usePreferencesStore.getState().load());
      expect(usePreferencesStore.getState().bindingNotice).toBeNull();
    },
  );
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
