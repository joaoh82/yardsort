import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_BINDINGS } from "@/lib/shortcuts";
import type { OtherComputers, OtherHostView } from "@/lib/ipc";
import { useLayoutStore } from "@/stores/layout";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { usePreferencesStore } from "@/stores/preferences";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";
import { project, worktree } from "@/test/fixtures";
import { CommandPalette } from "./CommandPalette";
import { commandEnabled, runCommand } from "./commands";
import { useAppShortcuts } from "./useAppShortcuts";
import { KeyboardSettings } from "@/features/settings/KeyboardSettings";

const core = vi.hoisted(() => ({
  uiStateSave: vi.fn().mockResolvedValue(undefined),
  uiStateLoad: vi.fn(),
  ptyClose: vi.fn().mockResolvedValue(undefined),
  otherHost: vi.fn(),
  otherHostStartShell: vi.fn(),
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
    usageOpen: false,
    pullRequestsOpen: false,
    tasksOpen: false,
    remote: null,
    composingThere: null,
  });
  useTerminalStore.setState({ tabs: [], active: {} });
  useOtherComputersStore.setState({ status: null, views: {}, active: {} });
});

/** Another computer this one is let into, with one workspace that has an agent and a shell. */
const others: OtherComputers = {
  client: { serverUrl: "https://remote.example.com", name: "Workstation", since: 1, email: null },
  signingIn: false,
  hosts: [
    {
      grantId: "g-1",
      hostId: "h-1",
      name: "Build box",
      pairing: "paired",
      link: { kind: "online" },
    },
    { grantId: "g-2", hostId: "h-2", name: "Not let in", pairing: "waiting", link: null },
  ],
  problem: null,
  signedOut: false,
  suggestedServerUrl: null,
  suggestedName: null,
};
const there: OtherHostView = {
  hostId: "h-1",
  name: "Build box",
  link: { kind: "online" },
  mayType: true,
  mayStart: true,
  mayResume: true,
  mayManage: false,
  mayLook: false,
  mayPublish: false,
  readAt: 1,
  platform: "Linux",
  appVersion: "0.34.1",
  projects: [
    {
      id: "p-there",
      name: "service",
      repository: null,
      workspaces: [
        {
          id: "w-there",
          name: "fix-login",
          kind: "worktree",
          branch: "ys/fix-login",
          archived: false,
          missing: false,
          tasks: [],
          sessions: [
            {
              id: "s-1",
              title: "Fix the login",
              harness: "claude",
              state: "busy",
              startedAt: 1,
              terminal: "t-1",
              resumable: false,
            },
          ],
          terminals: [{ id: "t-2", program: "bash", purpose: "shell", state: "quiet" }],
        },
        {
          id: "w-put-away",
          name: "old",
          kind: "worktree",
          branch: null,
          archived: true,
          missing: false,
          tasks: [],
          sessions: [],
          terminals: [],
        },
      ],
    },
  ],
  activity: [],
};

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
  it("opens Tasks from the palette, where a new shell tab is then not on offer", async () => {
    const user = userEvent.setup();
    render(<Host />);
    fireEvent.keyDown(window, { key: "K", ...mod });
    await user.type(screen.getByRole("combobox"), "tasks");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(useProjectsStore.getState().tasksOpen).toBe(true));

    fireEvent.keyDown(window, { key: "K", ...mod });
    await user.type(screen.getByRole("combobox"), "new shell");
    expect(screen.getByRole("option", { name: /New shell tab/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });
  it("opens Pull requests from the palette, where a new shell tab is then not on offer", async () => {
    const user = userEvent.setup();
    render(<Host />);
    fireEvent.keyDown(window, { key: "K", ...mod });
    await user.type(screen.getByRole("combobox"), "pull req");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(useProjectsStore.getState().pullRequestsOpen).toBe(true));

    // The panel is showing pull requests, not a workspace's terminals.
    fireEvent.keyDown(window, { key: "K", ...mod });
    await user.type(screen.getByRole("combobox"), "new shell");
    expect(screen.getByRole("option", { name: /New shell tab/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("opens Notifications with its key, where a new shell tab is then not on offer", async () => {
    const user = userEvent.setup();
    render(<Host />);
    fireEvent.keyDown(window, { key: "I", ...mod });
    await waitFor(() => expect(useProjectsStore.getState().notificationsOpen).toBe(true));

    // The panel is showing the list, not a workspace's terminals.
    fireEvent.keyDown(window, { key: "K", ...mod });
    await user.type(screen.getByRole("combobox"), "new shell");
    expect(screen.getByRole("option", { name: /New shell tab/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
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
  it("goes on from this computer's workspaces to another computer's, and wraps back", () => {
    useOtherComputersStore.setState({ status: others, views: { "h-1": there } });
    render(<Host />);
    fireEvent.keyDown(window, { key: "ArrowDown", ...mod });
    // The one put away there is passed over, as one here is.
    expect(useProjectsStore.getState().remote).toEqual({ hostId: "h-1", workspaceId: "w-there" });
    fireEvent.keyDown(window, { key: "ArrowDown", ...mod });
    expect(enter).toHaveBeenLastCalledWith("w-demo", false);
    useProjectsStore.setState({ remote: null });
    fireEvent.keyDown(window, { key: "ArrowUp", ...mod });
    expect(useProjectsStore.getState().remote).toEqual({ hostId: "h-1", workspaceId: "w-there" });
  });
  it("opens a shell on the other computer when that one takes it from here", async () => {
    core.otherHost.mockResolvedValue({ ...there, mayManage: true });
    core.otherHostStartShell.mockResolvedValue({
      workspaceId: "w-there",
      terminal: "t-9",
      harness: "",
    });
    useOtherComputersStore.setState({
      status: others,
      views: { "h-1": { ...there, mayManage: true } },
    });
    useProjectsStore.getState().selectRemote("h-1", "w-there");
    const opened = vi.spyOn(useTerminalStore.getState(), "open");
    render(<Host />);
    expect(commandEnabled("newTerminal")).toBe(true);
    runCommand("newTerminal");
    expect(core.otherHostStartShell).toHaveBeenCalledWith("h-1", "w-there");
    await waitFor(() =>
      expect(useOtherComputersStore.getState().active["h-1/w-there"]).toBe("t-9"),
    );
    // Nothing is opened in this computer's workspace, hidden beneath it.
    expect(opened).not.toHaveBeenCalled();
    // A computer that is away is not asked.
    useOtherComputersStore.setState({
      views: {
        "h-1": { ...there, mayManage: true, link: { kind: "offline", since: 1, why: "away" } },
      },
    });
    expect(commandEnabled("newTerminal")).toBe(false);
  });
  it("acts on the workspace in view on another computer, never on the one here beneath it", () => {
    useOtherComputersStore.setState({ status: others, views: { "h-1": there } });
    useProjectsStore.getState().selectRemote("h-1", "w-there");
    render(<Host />);
    // Its tabs are that computer's terminals.
    fireEvent.keyDown(window, { key: "ArrowRight", ...mod });
    expect(useOtherComputersStore.getState().active["h-1/w-there"]).toBe("t-2");
    // No shell is opened in this computer's workspace, and nothing of it is closed.
    expect(commandEnabled("newTerminal")).toBe(false);
    expect(commandEnabled("closeTerminal")).toBe(false);
    // A new workspace is made where the one in view is.
    runCommand("newWorkspace");
    expect(useProjectsStore.getState().composingThere).toEqual({
      hostId: "h-1",
      projectId: "p-there",
    });
  });
  it("offers no new workspace on a computer that lets this one only watch", () => {
    useOtherComputersStore.setState({
      status: others,
      views: { "h-1": { ...there, mayStart: false } },
    });
    useProjectsStore.getState().selectRemote("h-1", "w-there");
    expect(commandEnabled("newWorkspace")).toBe(false);
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
        toldAbout: null,
        toldAt: null,
        busySince: null,
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
      key: "U",
      ...mod,
    });
    await user.click(screen.getByRole("button", { name: "Save shortcuts" }));
    expect(core.uiStateSave).toHaveBeenCalledWith(
      "keyboard.bindings",
      expect.stringContaining('"toggleLeft":"u"'),
    );
    fireEvent.keyDown(window, { key: "B", ...mod });
    expect(useLayoutStore.getState().collapsed.left).toBe(false);
    fireEvent.keyDown(window, { key: "U", ...mod });
    expect(useLayoutStore.getState().collapsed.left).toBe(true);
    core.uiStateLoad.mockResolvedValue({
      "keyboard.bindings": JSON.stringify(usePreferencesStore.getState().bindings),
    });
    await act(async () => {
      usePreferencesStore.setState({ bindings: { ...DEFAULT_BINDINGS } });
      await usePreferencesStore.getState().load();
    });
    expect(usePreferencesStore.getState().bindings.toggleLeft).toBe("u");
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
