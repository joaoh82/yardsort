import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import type { OtherComputers, OtherHostView as View, RemoteTerminalSeen } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  otherComputers: vi.fn(),
  otherComputersSignIn: vi.fn(),
  otherComputersCancel: vi.fn(),
  otherComputersSignOut: vi.fn(),
  otherComputersPair: vi.fn(),
  otherComputersForget: vi.fn(),
  otherHost: vi.fn(),
  otherHostHarnesses: vi.fn(),
  otherHostStartWorkspace: vi.fn(),
  otherHostStartSession: vi.fn(),
  otherHostResume: vi.fn(),
  otherHostBranches: vi.fn(),
  otherHostClone: vi.fn(),
  otherHostStartShell: vi.fn(),
  otherHostRenameWorkspace: vi.fn(),
  otherHostRemoveWorkspace: vi.fn(),
  projectRepository: vi.fn(),
  remoteTerminalOpen: vi.fn(),
  remoteTerminalClose: vi.fn(),
  remoteTerminalTakeControl: vi.fn(),
  remoteTerminalRelease: vi.fn(),
  remoteTerminalWrite: vi.fn(),
  remoteTerminalResize: vi.fn(),
  uiStateSave: vi.fn(async () => {}),
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => opener);
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
// xterm needs a layout engine: the surface says what it was given, and attaches as the real one.
vi.mock("@/features/terminal/TerminalView", async () => {
  const { useEffect } = await import("react");
  return {
    TerminalSurface: ({
      source,
      fits,
      typing,
    }: {
      source: import("@/features/terminal/source").TerminalSource;
      fits: boolean;
      typing: boolean;
    }) => {
      useEffect(() => {
        const stopping = source.attach(
          { cols: 80, rows: 24 },
          { output() {}, reset() {}, size() {} },
        );
        // The real view says why it could not open; this one has nothing to show it on.
        stopping.catch(() => {});
        return () =>
          void stopping.then(
            (stop) => stop(),
            () => {},
          );
      }, [source]);
      return <div data-testid="surface" data-fits={String(fits)} data-typing={String(typing)} />;
    },
  };
});

import { OtherComputersSettings } from "@/features/settings/OtherComputersSettings";
import { OtherComputersTree } from "@/features/sidebar/OtherComputersTree";
import { thereKey, useOtherComputersStore } from "@/stores/otherComputers";
import { recall, useProjectsStore } from "@/stores/projects";
import { project } from "@/test/fixtures";
import { RemoteComposer } from "./RemoteComposer";
import { RemoteWorkspaceView } from "./RemoteWorkspaceView";
import { offeredThere } from "./there";

const signedOut: OtherComputers = {
  client: null,
  signingIn: false,
  hosts: [],
  problem: null,
  signedOut: false,
  suggestedServerUrl: "https://remote.example.com",
  suggestedName: "Workstation",
};
const paired: OtherComputers = {
  ...signedOut,
  client: { serverUrl: "https://remote.example.com", name: "Workstation", since: 1, email: null },
  hosts: [
    {
      grantId: "g-1",
      hostId: "h-1",
      name: "Build box",
      pairing: "paired",
      link: { kind: "online" },
    },
    {
      grantId: "g-2",
      hostId: "h-2",
      name: "Old laptop",
      pairing: "waiting",
      link: null,
    },
    {
      grantId: "g-3",
      hostId: "h-3",
      name: "Refused one",
      pairing: "refused",
      link: null,
    },
  ],
};
const shown: View = {
  hostId: "h-1",
  name: "Build box",
  link: { kind: "online" },
  mayType: true,
  mayStart: false,
  mayResume: false,
  mayManage: false,
  mayLook: false,
  mayPublish: false,
  readAt: 1_790_000_000_000,
  platform: "Linux",
  appVersion: "0.23.0",
  projects: [
    {
      id: "p-1",
      name: "service",
      repository: null,
      workspaces: [
        {
          id: "w-0",
          name: "local",
          kind: "local",
          branch: "main",
          archived: false,
          missing: false,
          tasks: [],
          sessions: [
            {
              id: "s-0",
              title: "Read the issue",
              harness: "codex",
              state: "ended",
              startedAt: 0,
              terminal: null,
              // That computer could resume it, but nothing offers it unless this one is let.
              resumable: true,
            },
          ],
          terminals: [],
        },
        {
          id: "w-1",
          name: "fix-login",
          kind: "worktree",
          branch: "ys/fix-login",
          archived: false,
          missing: false,
          tasks: [
            {
              key: "#91",
              title: "Login is flaky",
              url: "https://github.com/acme/service/issues/91",
            },
          ],
          sessions: [
            {
              id: "s-1",
              title: "Fix the flaky login test",
              harness: "claude",
              state: "busy",
              startedAt: 1,
              terminal: "t-1",
              resumable: false,
            },
          ],
          terminals: [{ id: "t-2", program: "bash", purpose: "shell", state: "quiet" }],
        },
      ],
    },
  ],
  activity: [],
};
const able: View = { ...shown, mayStart: true, mayResume: true };
/** A computer new enough to take a name, a model, an effort and a branch, and the rest. */
const full: View = { ...able, mayManage: true };
const away: View = {
  ...shown,
  link: { kind: "offline", since: 1_790_000_000_000, why: "It is not connected" },
};
const agents = [
  { id: "claude", name: "Claude Code", models: ["opus", "sonnet"], efforts: ["low", "high"] },
  { id: "codex", name: "Codex", models: [], efforts: [] },
];
/** A start that says only what every computer reads. */
const plainly = (projectId: string, prompt: string, harness: string) => ({
  projectId,
  prompt,
  harness,
  name: null,
  model: null,
  effort: null,
  baseBranch: null,
  existingBranch: null,
});

/** What that computer shows, as the window holds it once it has read it. */
const showing = (view: View = shown, status: OtherComputers = paired) => {
  core.otherComputers.mockResolvedValue(status);
  core.otherHost.mockResolvedValue(view);
  core.otherHostHarnesses.mockResolvedValue(agents);
  useOtherComputersStore.setState({ status, views: { "h-1": view } });
};

beforeEach(() => {
  vi.clearAllMocks();
  core.otherHostBranches.mockResolvedValue({
    branches: ["main", "develop", "ys/kept"],
    default: "main",
    checkedOut: ["main"],
    more: false,
  });
  core.projectRepository.mockResolvedValue({ key: null, clonable: false });
  useOtherComputersStore.setState({
    status: null,
    views: {},
    harnesses: {},
    active: {},
    startedHere: {},
    taking: [],
    error: null,
    startProblem: null,
  });
  useProjectsStore.setState({
    projects: [],
    remote: null,
    composingThere: null,
    ui: {},
    error: null,
  });
});

describe("the sidebar's other computers", () => {
  it("lists each computer with its projects and workspaces under it, as local ones are, and opens one", async () => {
    showing();
    render(<OtherComputersTree />);
    const tree = screen.getByRole("tree", { name: "Other computers" });
    const computer = within(tree).getByRole("treeitem", { name: "Build box" });
    expect(computer).toHaveAttribute("aria-expanded", "true");
    expect(within(computer).getByRole("button", { name: /Build box/ })).toHaveTextContent("online");
    // One still waiting to be let in is listed, and has nothing to open.
    expect(within(tree).getByRole("button", { name: /Old laptop/ })).toBeDisabled();
    expect(within(tree).getByRole("button", { name: /Old laptop/ })).toHaveTextContent("waiting");
    // Refused: in Settings, not here.
    expect(within(tree).queryByText("Refused one")).toBeNull();

    const project = within(computer).getByRole("treeitem", { name: "service" });
    // `local` says its branch; a worktree's is its name.
    expect(within(project).getByRole("treeitem", { name: "local" })).toHaveTextContent("main");
    const workspace = within(project).getByRole("treeitem", { name: "fix-login" });
    expect(workspace).not.toHaveTextContent("ys/fix-login");
    expect(within(workspace).getByRole("img", { name: "busy" })).toBeInTheDocument();
    expect(within(workspace).getByLabelText("1 open harness")).toBeInTheDocument();
    await userEvent.click(
      within(workspace).getByRole("button", { name: "Open task #91 — Login is flaky" }),
    );
    expect(opener.openUrl).toHaveBeenCalledWith("https://github.com/acme/service/issues/91");

    await userEvent.click(within(workspace).getByRole("button", { name: /fix-login/ }));
    expect(useProjectsStore.getState().remote).toEqual({ hostId: "h-1", workspaceId: "w-1" });
    expect(within(tree).getByRole("treeitem", { name: "fix-login" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("folds a computer and a project, and remembers both", async () => {
    showing();
    render(<OtherComputersTree />);
    const tree = screen.getByRole("tree", { name: "Other computers" });
    await userEvent.click(within(tree).getByRole("button", { name: "service" }));
    expect(within(tree).queryByRole("treeitem", { name: "fix-login" })).toBeNull();
    expect(recall(useProjectsStore.getState().ui, "sidebar.computerProjects", [])).toEqual([
      "h-1/p-1",
    ]);
    await userEvent.click(within(tree).getByRole("button", { name: "service" }));
    expect(within(tree).getByRole("treeitem", { name: "fix-login" })).toBeInTheDocument();

    await userEvent.click(within(tree).getByRole("button", { name: /Build box/ }));
    expect(within(tree).getByRole("treeitem", { name: "Build box" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(within(tree).queryByRole("treeitem", { name: "service" })).toBeNull();
    expect(recall(useProjectsStore.getState().ui, "sidebar.computers", {})).toEqual({
      "h-1": false,
    });
  });

  it("folds an offline computer by itself, and shows what it last showed once opened", async () => {
    const status = {
      ...paired,
      hosts: [{ ...paired.hosts[0]!, link: away.link }],
    };
    showing(away, status);
    render(<OtherComputersTree />);
    const tree = screen.getByRole("tree", { name: "Other computers" });
    expect(within(tree).getByRole("treeitem", { name: "Build box" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(within(tree).getByRole("button", { name: /Build box/ })).toHaveTextContent(/offline · /);
    await userEvent.click(within(tree).getByRole("button", { name: /Build box/ }));
    expect(within(tree).getByRole("treeitem", { name: "fix-login" })).toBeInTheDocument();
    // Nothing can be started on a computer that is away.
    expect(within(tree).queryByRole("button", { name: /^New workspace/ })).toBeNull();
  });

  it("says when a computer lets this one only watch, and offers nothing to start there", () => {
    showing({ ...shown, mayType: false });
    render(<OtherComputersTree />);
    const tree = screen.getByRole("tree", { name: "Other computers" });
    expect(within(tree).getByRole("button", { name: /Build box/ })).toHaveTextContent("watch only");
    expect(within(tree).queryByRole("button", { name: /^New workspace/ })).toBeNull();
    expect(core.otherHostHarnesses).not.toHaveBeenCalled();
  });

  it("composes a new workspace in a project there, where this computer may start work", async () => {
    showing(able);
    render(<OtherComputersTree />);
    const tree = screen.getByRole("tree", { name: "Other computers" });
    await userEvent.click(
      within(tree).getByRole("button", { name: "New workspace in service on Build box" }),
    );
    expect(useProjectsStore.getState().composingThere).toEqual({
      hostId: "h-1",
      projectId: "p-1",
    });
    // Where it will appear is shown, as under a local project.
    expect(within(tree).getByText("new workspace…")).toBeInTheDocument();
  });

  it("filters its projects with the sidebar's search", () => {
    showing();
    const { rerender } = render(<OtherComputersTree query="serv" />);
    expect(screen.getByRole("treeitem", { name: "service" })).toBeInTheDocument();
    rerender(<OtherComputersTree query="nothing-like-it" />);
    expect(screen.queryByRole("treeitem", { name: "Build box" })).toBeNull();
    // By the computer's own name, all of it shows.
    rerender(<OtherComputersTree query="build" />);
    expect(screen.getByRole("treeitem", { name: "service" })).toBeInTheDocument();
  });

  it("shows nothing until this computer is signed in and paired", () => {
    useOtherComputersStore.setState({ status: signedOut });
    const { container } = render(<OtherComputersTree />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("a workspace on another computer", () => {
  let said: (seen: RemoteTerminalSeen) => void = () => {};
  /** How opening a terminal goes: at once, as number 7, unless a test says otherwise. */
  let opened: () => Promise<number> = async () => 7;
  beforeEach(() => {
    opened = async () => 7;
    core.remoteTerminalOpen.mockImplementation(async (_host, _terminal, _output, onSeen) => {
      said = (seen) => act(() => onSeen(seen));
      return opened();
    });
    core.remoteTerminalClose.mockResolvedValue(undefined);
    core.remoteTerminalTakeControl.mockResolvedValue(undefined);
    core.remoteTerminalRelease.mockResolvedValue(undefined);
  });
  const watching = (terminal: string) =>
    vi.waitFor(() =>
      expect(core.remoteTerminalOpen).toHaveBeenLastCalledWith(
        "h-1",
        terminal,
        expect.any(Function),
        expect.any(Function),
      ),
    );
  const open = async (view: View = shown, workspaceId = "w-1") => {
    showing(view);
    render(<RemoteWorkspaceView hostId="h-1" workspaceId={workspaceId} />);
    if (workspaceId === "w-1") await watching("t-1");
  };
  const surface = () => screen.getByTestId("surface");
  const control = () => screen.getByRole("status", { name: "Control" });

  it("is the panel a local workspace gets, with the computer named in the bar and the bottom line", async () => {
    await open();
    expect(screen.getByRole("group", { name: "Build box, another computer" })).toHaveTextContent(
      "online",
    );
    // Its terminals are tabs: the agent's first and in front, then its shell.
    const tabs = within(screen.getByRole("tablist", { name: "Terminals" })).getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["claude", "bash"]);
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("through the relay").parentElement).toHaveTextContent(
      "Build box/service/fix-login· ys/fix-loginthrough the relay",
    );
    // Nothing of the page it replaces.
    expect(screen.queryByRole("button", { name: /Watch|Back|Close/ })).toBeNull();

    // Another tab watches another terminal, and lets the first go.
    await userEvent.click(tabs[1]!);
    await watching("t-2");
    expect(core.remoteTerminalClose).toHaveBeenCalledWith(7);
  });

  it("says which terminal there is on screen, for the Notifications list to mark seen", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const view = render(<span />);
    view.unmount();
    await open();
    const { useNotificationsStore } = await import("@/stores/notifications");
    // The agent's terminal is in front when the workspace opens.
    expect(useNotificationsStore.getState().thereInFront).toEqual({
      hostId: "h-1",
      terminalId: "t-1",
    });
    const tabs = within(screen.getByRole("tablist", { name: "Terminals" })).getAllByRole("tab");
    await userEvent.click(tabs[1]!);
    expect(useNotificationsStore.getState().thereInFront).toEqual({
      hostId: "h-1",
      terminalId: "t-2",
    });
  });

  it("watches at that computer's size, and types only once this computer takes control", async () => {
    await open();
    said({ kind: "controller", controller: { kind: "local" } });
    expect(control()).toHaveTextContent("The person at that computer has control.");
    expect(surface()).toHaveAttribute("data-typing", "false");
    expect(surface()).toHaveAttribute("data-fits", "false");
    // A terminal that was already running there is never taken by itself.
    expect(core.remoteTerminalTakeControl).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Take control" }));
    expect(core.remoteTerminalTakeControl).toHaveBeenCalledWith(7);
    expect(screen.getByRole("button", { name: "Taking control…" })).toBeDisabled();
    said({ kind: "held" });
    expect(control()).toHaveTextContent("This computer has control.");
    expect(surface()).toHaveAttribute("data-typing", "true");
    expect(surface()).toHaveAttribute("data-fits", "true");

    // Taken there: back to watching, and why.
    said({ kind: "controller", controller: { kind: "device", device: "d-2", name: "Phone" } });
    said({ kind: "notHeld", why: "Phone took control." });
    expect(screen.getByRole("alert")).toHaveTextContent("Phone took control.");
    expect(control()).toHaveTextContent("Phone has control.");
    expect(surface()).toHaveAttribute("data-typing", "false");
  });

  it("offers control once it is reached, and asks for it once it is open", async () => {
    let finish!: (id: number) => void;
    opened = () => new Promise((resolve) => (finish = resolve));
    await open();
    const take = screen.getByRole("button", { name: "Take control" });
    // Nobody has said who has control: it has not been reached.
    expect(take).toBeDisabled();

    // Its channel speaks before the open is answered, as it can.
    said({ kind: "controller", controller: { kind: "nobody" } });
    await userEvent.click(take);
    expect(screen.getByRole("button", { name: "Taking control…" })).toBeDisabled();
    expect(core.remoteTerminalTakeControl).not.toHaveBeenCalled();
    await act(async () => finish(7));
    await vi.waitFor(() => expect(core.remoteTerminalTakeControl).toHaveBeenCalledWith(7));
  });

  it("says why control was not asked for, and can be asked again", async () => {
    await open();
    said({ kind: "controller", controller: { kind: "nobody" } });
    core.remoteTerminalTakeControl.mockRejectedValueOnce({
      code: "remote_terminal",
      message: "That terminal is not open here.",
    });
    await userEvent.click(screen.getByRole("button", { name: "Take control" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That terminal is not open here.");
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
  });

  it("offers no control of a terminal that could not be opened", async () => {
    opened = async () => {
      throw { code: "remote_host", message: "That computer is not reached." };
    };
    await open();
    await act(async () => {});
    expect(screen.getByRole("button", { name: "Take control" })).toBeDisabled();
  });

  it("hands control back", async () => {
    await open();
    said({ kind: "held" });
    await userEvent.click(screen.getByRole("button", { name: "Release" }));
    expect(core.remoteTerminalRelease).toHaveBeenCalledWith(7);
  });

  it("offers no control where that computer lets this one only watch, and says when it is away", async () => {
    await open({ ...shown, mayType: false });
    expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
    expect(control()).toHaveTextContent("Build box lets this computer watch, not type.");
    said({ kind: "away", why: "The connection dropped." });
    expect(screen.getByRole("alert")).toHaveTextContent("The connection dropped.");
  });

  it("says when what it shows is from before the computer went away", async () => {
    await open(away);
    expect(screen.getByRole("group", { name: "Build box, another computer" })).toHaveTextContent(
      /offline since/,
    );
    expect(
      screen.getByText(/It is not connected\. Shown as Build box last showed it, at /),
    ).toBeInTheDocument();
    // Nothing is offered to start on a computer that is away.
    expect(screen.queryByRole("button", { name: /^Start / })).toBeNull();
  });

  it("offers nothing to start or resume where that computer does not let this one", async () => {
    await open(shown, "w-0");
    expect(screen.getByText("Nothing running in this workspace.")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Previous sessions" })).toHaveTextContent(
      "Read the issue",
    );
    expect(screen.queryByRole("button", { name: /^Start |^Resume / })).toBeNull();
    expect(core.otherHostHarnesses).not.toHaveBeenCalled();
  });

  it("starts one of that computer's agents from the tab bar, in front and this computer's to type into at once", async () => {
    const started = { workspaceId: "w-1", terminal: "t-9", harness: "codex" };
    core.otherHostStartSession.mockResolvedValue(started);
    await open(able);
    // What that computer shows once the agent runs there.
    const workspace = able.projects[0]!.workspaces[1]!;
    core.otherHost.mockResolvedValue({
      ...able,
      projects: [
        {
          ...able.projects[0]!,
          workspaces: [
            able.projects[0]!.workspaces[0]!,
            {
              ...workspace,
              sessions: [
                ...workspace.sessions,
                {
                  id: "s-9",
                  title: "",
                  harness: "codex",
                  state: "starting",
                  startedAt: 2,
                  terminal: "t-9",
                  resumable: false,
                },
              ],
            },
          ],
        },
      ],
    });
    const listsIt = core.otherHost.getMockImplementation()!;
    // That computer's next reading comes a moment after the start is answered.
    let read!: () => void;
    core.otherHost.mockImplementation(
      () => new Promise((resolve) => (read = () => resolve(listsIt("h-1")))),
    );
    await userEvent.click(await screen.findByRole("button", { name: "Start Codex" }));
    expect(core.otherHostStartSession).toHaveBeenCalledWith("h-1", "w-1", "codex", null);
    // Its terminal is in front at once, before it is listed.
    await watching("t-9");
    expect(screen.getByRole("tab", { name: /starting…/ })).toHaveAttribute("aria-selected", "true");
    await act(async () => read());
    expect(screen.getByRole("tab", { name: /codex/ })).toHaveAttribute("aria-selected", "true");

    // Its control is asked for as soon as it says who has it, without a press.
    said({ kind: "controller", controller: { kind: "nobody" } });
    await vi.waitFor(() => expect(core.remoteTerminalTakeControl).toHaveBeenCalledTimes(1));
    said({ kind: "held" });
    expect(control()).toHaveTextContent("This computer has control.");
    // Taken back there, it is not taken again by itself.
    said({ kind: "controller", controller: { kind: "local" } });
    said({ kind: "notHeld", why: "The person at that computer took control." });
    await act(async () => {});
    expect(core.remoteTerminalTakeControl).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
  });

  it("resumes an ended conversation there, and says why when that is refused", async () => {
    core.otherHostResume.mockRejectedValueOnce({
      code: "remote_lost",
      message: "That computer's answer did not arrive, so it may have started anyway.",
    });
    core.otherHostResume.mockResolvedValueOnce({
      workspaceId: "w-0",
      terminal: "t-8",
      harness: "codex",
    });
    await open(able, "w-0");
    await userEvent.click(screen.getByRole("button", { name: "Resume Read the issue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/may have started anyway/);
    expect(core.remoteTerminalOpen).not.toHaveBeenCalled();
    // Pressed again on purpose, it is asked again.
    await userEvent.click(screen.getByRole("button", { name: "Resume Read the issue" }));
    expect(core.otherHostResume).toHaveBeenLastCalledWith("h-1", "s-0", null);
    await vi.waitFor(() =>
      expect(useOtherComputersStore.getState().active[thereKey("h-1", "w-0")]).toBe("t-8"),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /** What that computer shows with `local`'s one conversation as `session` says. */
  const withLocal = (
    session: Partial<View["projects"][0]["workspaces"][0]["sessions"][0]>,
  ): View => {
    const local = able.projects[0]!.workspaces[0]!;
    return {
      ...able,
      projects: [
        {
          ...able.projects[0]!,
          workspaces: [
            { ...local, sessions: [{ ...local.sessions[0]!, ...session }] },
            able.projects[0]!.workspaces[1]!,
          ],
        },
      ],
    };
  };
  const reads = async (view: View) => {
    core.otherHost.mockResolvedValue(view);
    await act(() => useOtherComputersStore.getState().loadHost("h-1"));
  };

  it("offers to resume again once what it resumed has ended there, and keeps no tab for it", async () => {
    core.otherHostResume.mockResolvedValue({
      workspaceId: "w-0",
      terminal: "t-8",
      harness: "codex",
    });
    await open(able, "w-0");
    await userEvent.click(screen.getByRole("button", { name: "Resume Read the issue" }));
    await watching("t-8");
    expect(screen.getByRole("tab", { name: /starting…/ })).toBeInTheDocument();
    // That computer lists it while it runs, and no longer once it has ended.
    await reads(withLocal({ state: "busy", terminal: "t-8" }));
    expect(screen.getByRole("tab", { name: /codex/ })).toHaveAttribute("aria-selected", "true");
    await reads(withLocal({ state: "ended", terminal: null }));
    expect(screen.queryByRole("tab")).toBeNull();
    expect(screen.getByText("Nothing running in this workspace.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Resume Read the issue" })).toBeEnabled();
  });

  it("keeps no tab for what it started when a later reading there never listed it", async () => {
    core.otherHostResume.mockResolvedValue({
      workspaceId: "w-0",
      terminal: "t-8",
      harness: "codex",
    });
    await open(able, "w-0");
    await userEvent.click(screen.getByRole("button", { name: "Resume Read the issue" }));
    await watching("t-8");
    // A reading from about when it started may have been taken before it: it is still to come.
    await reads({ ...able, readAt: Date.now() });
    expect(screen.getByRole("tab", { name: /starting…/ })).toBeInTheDocument();
    // It ended before any reading listed it, and one well after its start does not list it.
    await reads({ ...able, readAt: Date.now() + 60_000 });
    expect(screen.queryByRole("tab")).toBeNull();
    expect(screen.getByRole("button", { name: "Resume Read the issue" })).toBeEnabled();
    expect(useOtherComputersStore.getState().taking).toEqual([]);
  });

  it("says so when that computer no longer shows the workspace", async () => {
    await open(shown, "w-gone");
    expect(
      screen.getByText("Build box no longer shows this workspace to this computer."),
    ).toBeInTheDocument();
  });
});

describe("looking after a workspace on another computer", () => {
  const row = (name: string) =>
    within(screen.getByRole("treeitem", { name })).getAllByRole("button")[0]!;
  const menu = async (name: string) => {
    fireEvent.contextMenu(row(name));
    return screen.findByRole("menu");
  };
  beforeEach(() => {
    core.otherHostStartShell.mockResolvedValue({
      workspaceId: "w-1",
      terminal: "t-7",
      harness: "",
    });
    core.otherHostRenameWorkspace.mockResolvedValue(undefined);
    core.otherHostRemoveWorkspace.mockResolvedValue(undefined);
    core.remoteTerminalOpen.mockResolvedValue(7);
    core.remoteTerminalClose.mockResolvedValue(undefined);
  });

  it("starts a shell there from its row and from its tab bar, and offers neither on an older computer", async () => {
    showing(full);
    render(<OtherComputersTree />);
    await userEvent.click(
      within(await menu("fix-login")).getByRole("menuitem", { name: "New shell" }),
    );
    expect(core.otherHostStartShell).toHaveBeenCalledWith("h-1", "w-1");
    await vi.waitFor(() =>
      expect(useProjectsStore.getState().remote).toEqual({ hostId: "h-1", workspaceId: "w-1" }),
    );
    expect(useOtherComputersStore.getState().active[thereKey("h-1", "w-1")]).toBe("t-7");
    render(<RemoteWorkspaceView hostId="h-1" workspaceId="w-1" />);
    await userEvent.click(await screen.findByRole("button", { name: "New shell" }));
    expect(core.otherHostStartShell).toHaveBeenCalledTimes(2);
  });

  it("offers no shell, rename, archive or delete where that computer's Yardsort is older, nor for its own checkout", async () => {
    showing(able);
    const { unmount } = render(<OtherComputersTree />);
    // Its agents are what its menu still offers, once it has said which it has.
    await vi.waitFor(() =>
      expect(useOtherComputersStore.getState().harnesses["h-1"]).toBeDefined(),
    );
    const older = await menu("fix-login");
    expect(within(older).getByRole("menuitem", { name: "Start Codex" })).toBeEnabled();
    for (const name of ["New shell", "Rename…", "Archive…", "Delete workspace…"]) {
      expect(within(older).queryByRole("menuitem", { name })).toBeNull();
    }
    unmount();
    showing(full);
    render(<OtherComputersTree />);
    const local = await menu("local");
    expect(within(local).getByRole("menuitem", { name: "New shell" })).toBeEnabled();
    for (const name of ["Rename…", "Archive…", "Delete workspace…"]) {
      expect(within(local).queryByRole("menuitem", { name })).toBeNull();
    }
  });

  it("renames a workspace there, and says why when that computer refuses", async () => {
    core.otherHostRenameWorkspace.mockRejectedValueOnce({
      code: "remote_refused",
      message: "A workspace name needs 1 to 80 characters.",
    });
    showing(full);
    render(<OtherComputersTree />);
    await userEvent.click(
      within(await menu("fix-login")).getByRole("menuitem", { name: "Rename…" }),
    );
    const dialog = screen.getByRole("dialog", { name: "Rename workspace" });
    const name = within(dialog).getByRole("textbox", { name: "Workspace name" });
    await userEvent.clear(name);
    await userEvent.type(name, "Login spacing");
    await userEvent.click(within(dialog).getByRole("button", { name: "Rename" }));
    expect(core.otherHostRenameWorkspace).toHaveBeenCalledWith("h-1", "w-1", "Login spacing");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("1 to 80 characters");
    await userEvent.click(within(dialog).getByRole("button", { name: "Rename" }));
    await vi.waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("deletes a workspace there only after it is asked, and uncommitted work only after a second yes", async () => {
    const dirty = { code: "remote_uncommitted", message: "It has uncommitted changes." };
    showing(full);
    useProjectsStore.setState({ remote: { hostId: "h-1", workspaceId: "w-1" } });
    render(<OtherComputersTree />);
    const press = async () =>
      userEvent.click(
        within(await menu("fix-login")).getByRole("menuitem", { name: "Delete workspace…" }),
      );
    // No to the first question: that computer is not asked at all.
    native.confirm.mockResolvedValueOnce(false);
    await press();
    await act(async () => {});
    expect(native.confirm.mock.calls[0]![0]).toContain(
      'Delete workspace "fix-login" on Build box?',
    );
    expect(native.confirm.mock.calls[0]![0]).toContain('The branch "ys/fix-login"');
    expect(core.otherHostRemoveWorkspace).not.toHaveBeenCalled();

    // Yes, but it has uncommitted work there, and no to losing it: nothing is forced.
    core.otherHostRemoveWorkspace.mockRejectedValueOnce(dirty);
    native.confirm.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    await press();
    await vi.waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(3));
    expect(native.confirm.mock.calls[2]![0]).toContain("destroys that work for good");
    await act(async () => {});
    expect(core.otherHostRemoveWorkspace.mock.calls).toEqual([["h-1", "w-1", true, false]]);
    expect(useProjectsStore.getState().remote).not.toBeNull();

    // Yes to both: asked again with force, and it leaves the view.
    core.otherHostRemoveWorkspace.mockRejectedValueOnce(dirty);
    native.confirm.mockResolvedValue(true);
    await press();
    await vi.waitFor(() =>
      expect(core.otherHostRemoveWorkspace).toHaveBeenLastCalledWith("h-1", "w-1", true, true),
    );
    await vi.waitFor(() => expect(useProjectsStore.getState().remote).toBeNull());
  });

  it("archives one the same way, and says why when that computer refuses for another reason", async () => {
    showing(full);
    render(<OtherComputersTree />);
    native.confirm.mockResolvedValue(true);
    core.otherHostRemoveWorkspace.mockRejectedValueOnce({
      code: "remote_offline",
      message: "That computer cannot be reached just now.",
    });
    const press = async () =>
      userEvent.click(within(await menu("fix-login")).getByRole("menuitem", { name: "Archive…" }));
    await press();
    await vi.waitFor(() =>
      expect(useProjectsStore.getState().error).toBe("That computer cannot be reached just now."),
    );
    // One question only: a refusal that is not about uncommitted work is not asked over.
    expect(native.confirm).toHaveBeenCalledTimes(1);
    await press();
    await vi.waitFor(() =>
      expect(core.otherHostRemoveWorkspace).toHaveBeenLastCalledWith("h-1", "w-1", false, false),
    );
  });
});

describe("a new workspace on another computer", () => {
  const composing = (view: View = able) => {
    showing(view);
    useProjectsStore.setState({ composingThere: { hostId: "h-1", projectId: "p-1" } });
    render(<RemoteComposer hostId="h-1" projectId="p-1" />);
    return screen.getByRole("form", { name: "New workspace on Build box" });
  };

  it("starts it there with the agent and the message, and opens it as this computer's to type into", async () => {
    core.otherHostStartWorkspace.mockResolvedValue({
      workspaceId: "w-9",
      terminal: "t-9",
      harness: "codex",
    });
    const form = composing();
    expect(form).toHaveTextContent("New workspace in service on Build box");
    const go = within(form).getByRole("button", { name: "Start" });
    await vi.waitFor(() =>
      expect(within(form).getByRole("combobox", { name: "Harness" })).toHaveValue("claude"),
    );
    // A new workspace needs something to do.
    expect(go).toBeDisabled();
    await userEvent.selectOptions(within(form).getByRole("combobox", { name: "Harness" }), "codex");
    await userEvent.type(
      within(form).getByRole("textbox", { name: "What should the agent work on?" }),
      "Fix the spacing",
    );
    await userEvent.click(go);
    expect(core.otherHostStartWorkspace).toHaveBeenCalledWith(
      "h-1",
      plainly("p-1", "Fix the spacing", "codex"),
    );
    await vi.waitFor(() =>
      expect(useProjectsStore.getState().remote).toEqual({ hostId: "h-1", workspaceId: "w-9" }),
    );
    expect(useProjectsStore.getState().composingThere).toBeNull();
    const others = useOtherComputersStore.getState();
    expect(others.active[thereKey("h-1", "w-9")]).toBe("t-9");
    expect(Object.keys(others.startedHere)).toEqual([thereKey("h-1", "t-9")]);
  });

  it("keeps the message and says why when the start fails, and does not try again by itself", async () => {
    core.otherHostStartWorkspace.mockRejectedValue({
      code: "remote_lost",
      message: "That computer's answer did not arrive, so it may have started anyway.",
    });
    const form = composing();
    const message = within(form).getByRole("textbox", { name: "What should the agent work on?" });
    await userEvent.type(message, "Fix the spacing{Enter}");
    expect(await within(form).findByRole("alert")).toHaveTextContent(/may have started anyway/);
    expect(message).toHaveValue("Fix the spacing");
    expect(core.otherHostStartWorkspace).toHaveBeenCalledTimes(1);
    expect(useProjectsStore.getState().composingThere).not.toBeNull();
  });

  it("starts nothing on a computer that is away, and says nothing is queued", async () => {
    const form = composing({ ...away, mayStart: true });
    await userEvent.type(
      within(form).getByRole("textbox", { name: "What should the agent work on?" }),
      "Fix the spacing{Enter}",
    );
    expect(within(form).getByRole("button", { name: "Start" })).toBeDisabled();
    expect(within(form).getByRole("alert")).toHaveTextContent(/nothing is queued for it/);
    expect(core.otherHostStartWorkspace).not.toHaveBeenCalled();
  });

  /** The composer as the center panel shows it: for wherever the store says composing is. */
  function Composing() {
    const there = useProjectsStore((s) => s.composingThere);
    return there ? (
      <RemoteComposer hostId={there.hostId} projectId={there.projectId} from={there.from ?? null} />
    ) : null;
  }
  /** Build box with a second project, and this computer with `service` and `notes` of its own. */
  const twoThere: View = {
    ...able,
    projects: [
      ...able.projects,
      { id: "p-2", name: "service-fork", repository: "github.com/acme/api", workspaces: [] },
    ],
  };
  const fromHere = (
    from: string,
    projectId: string | null,
    message = "",
    view: View = twoThere,
  ) => {
    showing(view);
    useProjectsStore.setState({
      projects: [project("service"), project("notes")],
      composingThere: { hostId: "h-1", projectId, from, message },
    });
    render(<Composing />);
    return screen.getByRole("form", { name: "New workspace on Build box" });
  };
  const field = (form: HTMLElement) =>
    within(form).getByRole("combobox", { name: "Project on Build box" });
  const kept = () =>
    recall<Record<string, string>>(useProjectsStore.getState().ui, "composer.projectThere", {});

  it("shows which of that computer's projects it will be in, for the project here it was opened in, and brings what was typed", async () => {
    const form = fromHere("p-service", "p-1", "Fix the spacing");
    expect(form).toHaveTextContent("New workspace in service on Build box");
    expect(field(form)).toHaveValue("p-1");
    expect(form).toHaveTextContent("same name as here · remembered after you start");
    expect(
      within(form).getByRole("textbox", { name: "What should the agent work on?" }),
    ).toHaveValue("Fix the spacing");
  });

  it("starts in another of that computer's projects when one is chosen, and offers that one next time", async () => {
    core.otherHostStartWorkspace.mockResolvedValue({
      workspaceId: "w-9",
      terminal: "t-9",
      harness: "claude",
    });
    const form = fromHere("p-service", "p-1", "Fix the spacing");
    await userEvent.selectOptions(field(form), "service-fork");
    expect(useProjectsStore.getState().composingThere).toMatchObject({
      projectId: "p-2",
      from: "p-service",
    });
    expect(form).toHaveTextContent("remembered after you start");
    expect(form).not.toHaveTextContent("same name as here");
    // Nothing is remembered for a choice that started nothing.
    expect(kept()).toEqual({});
    await vi.waitFor(() =>
      expect(within(form).getByRole("button", { name: "Start" })).toBeEnabled(),
    );
    await userEvent.click(within(form).getByRole("button", { name: "Start" }));
    expect(core.otherHostStartWorkspace).toHaveBeenCalledWith(
      "h-1",
      plainly("p-2", "Fix the spacing", "claude"),
    );
    await vi.waitFor(() => expect(kept()).toEqual({ [thereKey("h-1", "p-service")]: "p-2" }));
    // Next time, from the same project here, it is the one offered, and said to be.
    const again = offeredThere(twoThere, useProjectsStore.getState().ui, "h-1", {
      id: "p-service",
      name: "service",
    });
    expect(again).toEqual({ projectId: "p-2", how: "remembered" });
    useProjectsStore.setState({
      remote: null,
      composingThere: { hostId: "h-1", projectId: again.projectId, from: "p-service" },
    });
    expect(await screen.findByText("as you chose last time")).toBeInTheDocument();
  });

  it("says so when that computer has no project of that name, and starts nothing until one is chosen", async () => {
    const form = fromHere("p-notes", null, "Tidy the notes");
    expect(form).toHaveTextContent("New workspace in notes on Build box");
    expect(within(form).getByRole("alert")).toHaveTextContent(
      "Build box has no project called notes. Add it there",
    );
    expect(within(form).getByRole("alert")).toHaveTextContent(
      "Or choose another of its projects above.",
    );
    expect(field(form)).toHaveValue("");
    expect(within(form).getByRole("button", { name: "Start" })).toBeDisabled();
    await userEvent.type(
      within(form).getByRole("textbox", { name: "What should the agent work on?" }),
      "{Enter}",
    );
    expect(core.otherHostStartWorkspace).not.toHaveBeenCalled();
    await userEvent.selectOptions(field(form), "service");
    await vi.waitFor(() =>
      expect(within(form).getByRole("button", { name: "Start" })).toBeEnabled(),
    );
    expect(within(form).queryByRole("alert")).toBeNull();
  });

  it("goes back to this computer's composer with what was typed", async () => {
    const form = fromHere("p-service", "p-1");
    await userEvent.type(
      within(form).getByRole("textbox", { name: "What should the agent work on?" }),
      "Fix the spacing",
    );
    await userEvent.click(within(form).getByRole("button", { name: "Run on: Build box" }));
    const menu = screen.getByRole("menu", { name: "Run on" });
    expect(within(menu).getByRole("menuitemradio", { name: "Build box, online" })).toBeChecked();
    await userEvent.click(within(menu).getByRole("menuitemradio", { name: "This computer" }));
    const projects = useProjectsStore.getState();
    expect(projects.composingThere).toBeNull();
    expect(projects.composingProjectId).toBe("p-service");
    expect(projects.composingPrompt).toBe("Fix the spacing");
  });

  it("does not offer this computer for a project it has none of that name for", async () => {
    const form = composing();
    await userEvent.click(within(form).getByRole("button", { name: "Run on: Build box" }));
    expect(
      screen.getByRole("menuitemradio", {
        name: "This computer, no project called service here",
      }),
    ).toBeDisabled();
  });

  const typed = (form: HTMLElement) =>
    within(form).getByRole("textbox", { name: "What should the agent work on?" });

  it("says what the local composer says, on a computer that reads it: a name, a model, an effort and a branch", async () => {
    core.otherHostStartWorkspace.mockResolvedValue({
      workspaceId: "w-9",
      terminal: "t-9",
      harness: "claude",
    });
    const form = fromHere("p-service", "p-1", "Fix the spacing", { ...twoThere, mayManage: true });
    const branch = await within(form).findByRole("combobox", { name: "Branch" });
    expect(core.otherHostBranches).toHaveBeenCalledWith("h-1", "p-1");
    // Its default branch to start from, and only branches nobody has out there to open.
    expect(branch).toHaveValue("new:main");
    expect(within(branch).getByRole("group", { name: "Open existing branch" })).toHaveTextContent(
      /^developys\/kept$/,
    );
    await userEvent.type(within(form).getByRole("textbox", { name: "Name" }), "tidy spacing");
    await userEvent.type(within(form).getByRole("combobox", { name: "Model" }), "opus");
    await userEvent.selectOptions(within(form).getByRole("combobox", { name: "Effort" }), "high");
    await userEvent.selectOptions(branch, "new:develop");
    await userEvent.click(within(form).getByRole("button", { name: "Start" }));
    expect(core.otherHostStartWorkspace).toHaveBeenCalledWith("h-1", {
      projectId: "p-1",
      prompt: "Fix the spacing",
      harness: "claude",
      name: "tidy spacing",
      model: "opus",
      effort: "high",
      baseBranch: "develop",
      existingBranch: null,
    });
  });

  it("opens an existing branch there as it is, with no name of its own", async () => {
    core.otherHostStartWorkspace.mockResolvedValue({
      workspaceId: "w-9",
      terminal: "t-9",
      harness: "claude",
    });
    const form = fromHere("p-service", "p-1", "Carry on", { ...twoThere, mayManage: true });
    const branch = await within(form).findByRole("combobox", { name: "Branch" });
    await userEvent.type(within(form).getByRole("textbox", { name: "Name" }), "ignored");
    await userEvent.selectOptions(branch, "open:ys/kept");
    expect(within(form).queryByRole("textbox", { name: "Name" })).toBeNull();
    expect(form).toHaveTextContent('Opens the existing branch "ys/kept" in a new git worktree');
    await userEvent.click(within(form).getByRole("button", { name: "Start" }));
    expect(core.otherHostStartWorkspace).toHaveBeenCalledWith("h-1", {
      ...plainly("p-1", "Carry on", "claude"),
      existingBranch: "ys/kept",
    });
  });

  it("leaves those out for a computer whose Yardsort is older, and says so", async () => {
    const form = fromHere("p-service", "p-1", "Fix the spacing");
    await vi.waitFor(() =>
      expect(within(form).getByRole("button", { name: "Start" })).toBeEnabled(),
    );
    for (const name of ["Model", "Effort", "Branch"]) {
      expect(within(form).queryByRole("combobox", { name })).toBeNull();
    }
    expect(within(form).queryByRole("textbox", { name: "Name" })).toBeNull();
    expect(form).toHaveTextContent("Its Yardsort is older than this one");
    expect(core.otherHostBranches).not.toHaveBeenCalled();
  });

  it("offers the project there that is the same repository over one of the same name", async () => {
    core.projectRepository.mockResolvedValue({ key: "github.com/acme/api", clonable: true });
    const form = fromHere("p-service", "p-1");
    expect(core.projectRepository).toHaveBeenCalledWith("p-service");
    await vi.waitFor(() => expect(field(form)).toHaveValue("p-2"));
    expect(form).toHaveTextContent("same repository as here · remembered after you start");
    // Chosen by the person, it is theirs: nothing offered replaces it.
    await userEvent.selectOptions(field(form), "service");
    await act(async () => {});
    expect(field(form)).toHaveValue("p-1");
  });

  it("has that computer clone a project it lacks, then starts in the clone and remembers it", async () => {
    core.projectRepository.mockResolvedValue({ key: "github.com/acme/notes", clonable: true });
    core.otherHostClone.mockRejectedValueOnce({
      code: "remote_refused",
      message: "The computer could not clone that repository.",
    });
    core.otherHostClone.mockResolvedValueOnce({ projectId: "p-new", name: "notes" });
    core.otherHostStartWorkspace.mockResolvedValue({
      workspaceId: "w-9",
      terminal: "t-9",
      harness: "claude",
    });
    const form = fromHere("p-notes", null, "Tidy the notes", { ...twoThere, mayManage: true });
    // Never by itself: it is offered, and Start waits until it is chosen.
    const offer = await within(form).findByRole("button", { name: "Clone it there" });
    expect(within(form).getByRole("button", { name: "Start" })).toBeDisabled();
    await userEvent.click(offer);
    expect(field(form)).toHaveDisplayValue("clone notes there");
    expect(form).toHaveTextContent("cloned into Build box's projects folder when you start");
    // A clone that fails says why, and nothing is started.
    await userEvent.click(within(form).getByRole("button", { name: "Start" }));
    expect(await within(form).findByRole("alert")).toHaveTextContent("could not clone");
    expect(core.otherHostStartWorkspace).not.toHaveBeenCalled();
    expect(typed(form)).toHaveValue("Tidy the notes");
    // Asked again on purpose, it is cloned once, by this computer's project and nothing else.
    await userEvent.click(within(form).getByRole("button", { name: "Start" }));
    await vi.waitFor(() => expect(core.otherHostStartWorkspace).toHaveBeenCalledTimes(1));
    expect(core.otherHostClone).toHaveBeenLastCalledWith("h-1", "p-notes");
    expect(core.otherHostStartWorkspace).toHaveBeenCalledWith(
      "h-1",
      plainly("p-new", "Tidy the notes", "claude"),
    );
    await vi.waitFor(() => expect(kept()).toEqual({ [thereKey("h-1", "p-notes")]: "p-new" }));
  });

  it("offers no clone where that computer cannot, or this project has nowhere to clone from", async () => {
    core.projectRepository.mockResolvedValue({ key: "github.com/acme/notes", clonable: true });
    const older = fromHere("p-notes", null, "Tidy the notes");
    await act(async () => {});
    expect(within(older).queryByRole("button", { name: "Clone it there" })).toBeNull();
    expect(within(older).getByRole("alert")).toHaveTextContent("Add it there");
  });

  it("gives up on Escape", async () => {
    const form = composing();
    await userEvent.type(
      within(form).getByRole("textbox", { name: "What should the agent work on?" }),
      "{Escape}",
    );
    expect(useProjectsStore.getState().composingThere).toBeNull();
  });
});

describe("Settings → Remote → Other computers", () => {
  it("offers the server this computer is a host on, and signs in as a client there", async () => {
    core.otherComputers.mockResolvedValue(signedOut);
    core.otherComputersSignIn.mockResolvedValue(paired);
    useOtherComputersStore.setState({ status: signedOut });
    render(<OtherComputersSettings />);
    expect(screen.getByRole("textbox", { name: /Server/ })).toHaveValue(
      "https://remote.example.com",
    );
    await userEvent.click(screen.getByRole("button", { name: "Sign in to reach other computers" }));
    expect(core.otherComputersSignIn).toHaveBeenCalledWith(
      "https://remote.example.com",
      "Workstation",
    );
    expect(await screen.findByText(/Signed in as/)).toHaveTextContent(
      "Signed in as Workstation at https://remote.example.com.",
    );
  });

  it("says which account it signed in to, which need not be the host's", async () => {
    const signedIn = {
      ...paired,
      client: { ...paired.client!, email: "ana+work@example.com" },
    };
    core.otherComputers.mockResolvedValue(signedIn);
    useOtherComputersStore.setState({ status: signedIn });
    render(<OtherComputersSettings />);
    expect(await screen.findByText(/Account:/)).toHaveTextContent("Account: ana+work@example.com");
  });

  it("pairs by the other computer's code, and says what the server said when it is wrong", async () => {
    core.otherComputersPair.mockRejectedValueOnce({
      code: "remote_code",
      message: "No computer is showing that code. Check it, or show a new one there.",
    });
    core.otherComputers.mockResolvedValue(paired);
    useOtherComputersStore.setState({ status: paired });
    render(<OtherComputersSettings />);
    await userEvent.type(screen.getByRole("textbox", { name: /Pairing code/ }), "abcd-efgh");
    await userEvent.click(screen.getByRole("button", { name: "Pair" }));
    expect(core.otherComputersPair).toHaveBeenCalledWith("abcd-efgh");
    expect(await screen.findByRole("alert")).toHaveTextContent("No computer is showing that code.");

    core.otherComputersPair.mockResolvedValueOnce(paired);
    await userEvent.click(screen.getByRole("button", { name: "Pair" }));
    expect(screen.getByRole("textbox", { name: /Pairing code/ })).toHaveValue("");
  });

  it("lists every pairing, refused ones too, each of which can be forgotten", async () => {
    core.otherComputers.mockResolvedValue(paired);
    core.otherComputersForget.mockResolvedValue({ ...paired, hosts: paired.hosts.slice(1) });
    useOtherComputersStore.setState({ status: paired });
    render(<OtherComputersSettings />);
    const list = screen.getByRole("list");
    expect(within(list).getByText("waiting for its answer")).toBeInTheDocument();
    expect(within(list).getByText("refused")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Forget Build box" }));
    expect(core.otherComputersForget).toHaveBeenCalledWith("g-1");
    expect(screen.queryByText("Build box")).toBeNull();
  });
});
