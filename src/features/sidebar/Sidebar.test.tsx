import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { added, project, record, worktree } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  uiStateLoad: vi.fn(),
  uiStateSave: vi.fn(),
  projectsList: vi.fn(),
  projectOpen: vi.fn(),
  projectCreate: vi.fn(),
  projectRemove: vi.fn(),
  projectsReorder: vi.fn(),
  workspaceDelete: vi.fn(),
  workspaceArchive: vi.fn(),
  workspaceRestore: vi.fn(),
  workspaceRename: vi.fn(),
  sessionsList: vi.fn(),
  ptySpawn: vi.fn(),
  ptyClose: vi.fn(),
}));
const native = vi.hoisted(() => ({
  pickFolder: vi.fn(),
  confirm: vi.fn(),
  revealInFileManager: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
vi.mock("@/lib/native", () => ({ native }));

import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";
import { Sidebar } from "./Sidebar";

const shellIn = (workspace: string) => ({
  id: `s-${workspace}`,
  program: "/bin/bash",
  args: [],
  cwd: null,
  pid: 1,
  size: { cols: 80, rows: 24 },
  labels: { workspace },
  state: { status: "running" as const },
  hasOutput: true,
  busy: false,
  idleMs: 0,
});

async function renderSidebar(...names: string[]) {
  core.projectsList.mockResolvedValue(names.map((name) => project(name)));
  render(<Sidebar />);
  if (names[0]) await screen.findByRole("treeitem", { name: names[0] });
  else await screen.findByText("No projects yet.");
}

/** One project whose `local` sits beside a worktree workspace called `feature`. */
async function renderWithWorktree() {
  const alpha = project("alpha");
  core.projectsList.mockResolvedValue([
    { ...alpha, workspaces: [...alpha.workspaces, worktree("alpha", "feature")] },
  ]);
  render(<Sidebar />);
  await screen.findByRole("treeitem", { name: "feature" });
}

const rowButton = (name: string) =>
  within(screen.getByRole("treeitem", { name })).getAllByRole("button")[0]!;

describe("Sidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.uiStateLoad.mockResolvedValue({});
    for (const fn of [core.uiStateSave, core.projectRemove, core.projectsReorder, core.ptyClose]) {
      fn.mockResolvedValue(undefined);
    }
    core.ptySpawn.mockImplementation(async ({ workspaceId }) => shellIn(workspaceId));
    core.sessionsList.mockResolvedValue([]);
    useSessionsStore.setState({ byWorkspace: {}, error: null });
    useProjectsStore.setState({
      projects: [],
      loaded: false,
      selectedWorkspaceId: null,
      composingProjectId: null,
      collapsed: [],
      error: null,
      notice: null,
    });
    useTerminalStore.setState({ tabs: [], active: {}, error: null });
  });

  it("shows each project with its local workspace and branch", async () => {
    await renderSidebar("alpha", "beta");
    const alpha = screen.getByRole("treeitem", { name: "alpha" });
    const local = within(alpha).getByRole("treeitem", { name: "local" });
    expect(local).toHaveTextContent("main");
    expect(local).toHaveAttribute("aria-selected", "false");
  });

  it("says on the row whether an agent is waiting, working or done", async () => {
    await renderWithWorktree();
    const harness = {
      id: "s1",
      workspaceId: "w-alpha-feature",
      title: "claude",
      exit: null,
      recordId: "r1",
      busy: false,
      attention: false,
    };
    const row = () => within(screen.getByRole("treeitem", { name: "feature" }));
    const badge = (name: string) => row().queryByRole("img", { name });

    act(() => useTerminalStore.setState({ tabs: [harness] }));
    expect(badge("1 waiting for you")).toHaveTextContent("1");

    // Working is the one state that says nothing: the dot is already pulsing.
    act(() => useTerminalStore.setState({ tabs: [{ ...harness, busy: true }] }));
    expect(badge("1 waiting for you")).toBeNull();

    act(() =>
      useTerminalStore.setState({
        tabs: [{ ...harness, exit: { code: 0, success: true, signal: null } }],
      }),
    );
    expect(badge("finished")).toHaveTextContent("✓");

    act(() =>
      useTerminalStore.setState({
        tabs: [{ ...harness, exit: { code: 1, success: false, signal: null } }],
      }),
    );
    expect(badge("exited with an error")).toHaveTextContent("✗");
  });

  it("entering a workspace selects it and opens a shell there — once", async () => {
    const user = userEvent.setup();
    await renderWithWorktree();

    await user.click(rowButton("feature"));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha-feature");
    expect(core.ptySpawn).toHaveBeenCalledWith(
      expect.objectContaining({ workspaceId: "w-alpha-feature" }),
    );

    await user.click(rowButton("feature"));
    expect(core.ptySpawn).toHaveBeenCalledTimes(1);
  });

  // `local` is the project's own checkout, where a shell is only one of the things you might
  // want. It selects, and the panel offers the choice; a worktree gets its shell as before.
  it("selecting local opens nothing by itself", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha");

    await user.click(rowButton("local"));
    await vi.waitFor(() => expect(core.sessionsList).toHaveBeenCalledWith("w-alpha"));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  it("collapses and expands a project", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha");
    await user.click(screen.getByRole("button", { name: "alpha" }));
    expect(screen.queryByRole("treeitem", { name: "local" })).not.toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: "alpha" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("opening a plain folder asks before initialising git, and respects a no", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    native.pickFolder.mockResolvedValue("/tmp/plain");
    native.confirm.mockResolvedValue(false);
    core.projectOpen.mockRejectedValue({ code: "not_a_git_repo", message: "not a repo" });

    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Open a folder/ }));

    expect(native.confirm).toHaveBeenCalledWith(
      expect.stringContaining("/tmp/plain"),
      expect.objectContaining({ okLabel: "Initialise git" }),
    );
    expect(core.projectOpen).toHaveBeenCalledTimes(1);
    expect(core.projectOpen).not.toHaveBeenCalledWith("/tmp/plain", true);
  });

  it("creates a project from the dialog and lands in its local workspace", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    native.pickFolder.mockResolvedValue("/code");
    core.projectCreate.mockResolvedValue(added("fresh"));

    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Create a new project/ }));
    const create = screen.getByRole("button", { name: "Create project" });
    expect(create).toBeDisabled();

    await user.type(screen.getByLabelText("Name"), "fresh");
    await user.click(screen.getByRole("button", { name: "Browse…" }));
    expect(screen.getByText("/code/fresh")).toBeInTheDocument();
    await user.click(create);

    expect(core.projectCreate).toHaveBeenCalledWith("fresh", "/code");
    expect(await screen.findByRole("treeitem", { name: "fresh" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(core.ptySpawn).toHaveBeenCalledWith(expect.objectContaining({ workspaceId: "w-fresh" }));
  });

  it("keeps the dialog open and shows why when creation fails", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    core.projectCreate.mockRejectedValue({
      code: "invalid_name",
      message: 'Project name cannot contain "/".',
    });

    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Create a new project/ }));
    await user.type(screen.getByLabelText("Name"), "a/b");
    await user.type(screen.getByLabelText("Location"), "/code");
    await user.click(screen.getByRole("button", { name: "Create project" }));

    expect(await screen.findByRole("alert")).toHaveTextContent('cannot contain "/"');
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("removing a project confirms first, then closes its sessions and forgets it", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha", "beta");
    // A session has to exist for the confirmation to have something to count; a worktree opens
    // one by itself, which `local` deliberately no longer does.
    useTerminalStore.setState({
      tabs: [
        {
          id: "s-w-alpha",
          workspaceId: "w-alpha",
          title: "bash",
          exit: null,
          recordId: null,
          busy: false,
          attention: false,
        },
      ],
      active: { "w-alpha": "s-w-alpha" },
    });

    native.confirm.mockResolvedValue(false);
    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    await user.click(screen.getByRole("menuitem", { name: /Remove from Yardsort/ }));
    expect(native.confirm).toHaveBeenCalledWith(
      expect.stringMatching(/Nothing on disk is deleted[\s\S]*1 running terminal session /),
      expect.anything(),
    );
    expect(core.projectRemove).not.toHaveBeenCalled();

    native.confirm.mockResolvedValue(true);
    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    await user.click(screen.getByRole("menuitem", { name: /Remove from Yardsort/ }));
    expect(core.ptyClose).toHaveBeenCalledWith("s-w-alpha");
    expect(core.projectRemove).toHaveBeenCalledWith("p-alpha");
    expect(screen.queryByRole("treeitem", { name: "alpha" })).not.toBeInTheDocument();
  });

  it("reorders from the menu, with the ends disabled", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha", "beta");
    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    expect(screen.getByRole("menuitem", { name: "Move up" })).toBeDisabled();
    await user.click(screen.getByRole("menuitem", { name: "Move down" }));
    expect(core.projectsReorder).toHaveBeenCalledWith(["p-beta", "p-alpha"]);
  });

  it("the project's + opens the composer for it, and picking a workspace closes it again", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha", "beta");
    await user.click(screen.getByRole("button", { name: "New workspace in beta" }));
    expect(useProjectsStore.getState().composingProjectId).toBe("p-beta");
    expect(
      within(screen.getByRole("treeitem", { name: "beta" })).getByText("new workspace…"),
    ).toBeVisible();

    const alpha = screen.getByRole("treeitem", { name: "alpha" });
    await user.click(
      within(within(alpha).getByRole("treeitem", { name: "local" })).getByRole("button"),
    );
    expect(useProjectsStore.getState().composingProjectId).toBeNull();
  });

  describe("deleting a workspace", () => {
    async function openDeleteMenu() {
      const user = userEvent.setup();
      const app = project("app");
      app.workspaces.push(worktree("app", "fix-login"));
      core.projectsList.mockResolvedValue([app]);
      render(<Sidebar />);
      await screen.findByRole("treeitem", { name: "fix-login" });
      await user.click(screen.getByRole("button", { name: "More actions for fix-login" }));
      await user.click(screen.getByRole("menuitem", { name: /Delete workspace/ }));
      return user;
    }

    it("says the branch is kept, then removes it", async () => {
      native.confirm.mockResolvedValue(true);
      core.workspaceDelete.mockResolvedValue(undefined);
      await openDeleteMenu();

      expect(native.confirm).toHaveBeenCalledTimes(1);
      expect(native.confirm.mock.calls[0]![0]).toMatch(
        /branch "ys\/fix-login" and all its commits are kept/,
      );
      expect(core.workspaceDelete).toHaveBeenCalledWith("w-app-fix-login", false);
      expect(screen.queryByRole("treeitem", { name: "fix-login" })).not.toBeInTheDocument();
    });

    it("never destroys uncommitted work without a second, explicit yes", async () => {
      core.workspaceDelete.mockRejectedValueOnce({ code: "worktree_dirty", message: "dirty" });
      native.confirm.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
      await openDeleteMenu();

      expect(native.confirm).toHaveBeenCalledTimes(2);
      expect(native.confirm.mock.calls[1]![0]).toMatch(/destroys that work for good/);
      expect(core.workspaceDelete).toHaveBeenCalledTimes(1);
      expect(screen.getByRole("treeitem", { name: "fix-login" })).toBeInTheDocument();
    });

    it("forces only after that second yes", async () => {
      core.workspaceDelete.mockRejectedValueOnce({ code: "worktree_dirty", message: "dirty" });
      core.workspaceDelete.mockResolvedValueOnce(undefined);
      native.confirm.mockResolvedValue(true);
      await openDeleteMenu();

      expect(core.workspaceDelete).toHaveBeenLastCalledWith("w-app-fix-login", true);
      expect(screen.queryByRole("treeitem", { name: "fix-login" })).not.toBeInTheDocument();
    });

    it("shows an error instead of silently doing nothing when the dialog itself fails", async () => {
      native.confirm.mockRejectedValue(new Error("dialog.message not allowed"));
      await openDeleteMenu();
      expect(await screen.findByRole("alert")).toHaveTextContent("dialog.message not allowed");
      expect(core.workspaceDelete).not.toHaveBeenCalled();
    });

    it("local has no delete", async () => {
      await renderSidebar("alpha");
      expect(
        screen.queryByRole("button", { name: "More actions for local" }),
      ).not.toBeInTheDocument();
    });
  });

  it("entering a workspace with conversations to resume does not bury them under a new shell", async () => {
    const user = userEvent.setup();
    core.sessionsList.mockResolvedValue([record("r1", { workspaceId: "w-alpha" })]);
    await renderSidebar("alpha");
    await user.click(within(screen.getByRole("treeitem", { name: "local" })).getByRole("button"));
    await vi.waitFor(() => expect(core.sessionsList).toHaveBeenCalledWith("w-alpha"));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  describe("workspace housekeeping", () => {
    async function withWorktree(overrides = {}) {
      const user = userEvent.setup();
      const app = project("app");
      app.workspaces.push(worktree("app", "fix-login", overrides));
      core.projectsList.mockResolvedValue([app]);
      render(<Sidebar />);
      await screen.findByRole("treeitem", { name: "app" });
      return user;
    }
    const openMenu = async (user: ReturnType<typeof userEvent.setup>) =>
      user.click(screen.getByRole("button", { name: "More actions for fix-login" }));

    it("renames the label only, and says so", async () => {
      const user = await withWorktree();
      core.workspaceRename.mockResolvedValue(worktree("app", "fix-login", { name: "Login fix" }));
      await openMenu(user);
      await user.click(screen.getByRole("menuitem", { name: "Rename…" }));

      const dialog = screen.getByRole("dialog", { name: "Rename workspace" });
      expect(dialog).toHaveTextContent(/folder and the branch keep theirs/);
      expect(within(dialog).getByRole("button", { name: "Rename" })).toBeDisabled();

      const input = within(dialog).getByLabelText("Workspace name");
      await user.clear(input);
      await user.type(input, "Login fix{Enter}");

      expect(core.workspaceRename).toHaveBeenCalledWith("w-app-fix-login", "Login fix");
      expect(await screen.findByRole("treeitem", { name: "Login fix" })).toBeInTheDocument();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("keeps the rename dialog open with the reason when the core refuses", async () => {
      const user = await withWorktree();
      core.workspaceRename.mockRejectedValue({
        code: "invalid_name",
        message: "A workspace name needs 1 to 80 characters.",
      });
      await openMenu(user);
      await user.click(screen.getByRole("menuitem", { name: "Rename…" }));
      await user.type(screen.getByLabelText("Workspace name"), "x{Enter}");
      expect(await screen.findByRole("alert")).toHaveTextContent("1 to 80 characters");
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    it("archives after explaining what is kept, and tucks the workspace away", async () => {
      const user = await withWorktree();
      native.confirm.mockResolvedValue(true);
      core.workspaceArchive.mockResolvedValue(undefined);
      await openMenu(user);
      await user.click(screen.getByRole("menuitem", { name: "Archive…" }));

      expect(native.confirm).toHaveBeenCalledWith(
        expect.stringMatching(/branch, its commits and the session history are kept/),
        expect.objectContaining({ okLabel: "Archive" }),
      );
      expect(core.workspaceArchive).toHaveBeenCalledWith("w-app-fix-login", false);
      const group = await screen.findByRole("treeitem", { name: "Archived workspaces" });
      expect(group).toHaveTextContent("archived (1)");
      expect(screen.queryByRole("treeitem", { name: "fix-login" })).not.toBeInTheDocument();
    });

    it("never archives over uncommitted work without a second, explicit yes", async () => {
      const user = await withWorktree();
      native.confirm.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
      core.workspaceArchive.mockRejectedValue({ code: "worktree_dirty", message: "dirty" });
      await openMenu(user);
      await user.click(screen.getByRole("menuitem", { name: "Archive…" }));
      await vi.waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(2));
      expect(native.confirm).toHaveBeenLastCalledWith(
        expect.stringMatching(/cannot be recovered/),
        expect.objectContaining({ okLabel: "Archive anyway" }),
      );
      expect(core.workspaceArchive).toHaveBeenCalledTimes(1);
      expect(screen.getByRole("treeitem", { name: "fix-login" })).toBeInTheDocument();
    });

    it("restores an archived workspace from its group and goes to it", async () => {
      const user = await withWorktree({ archived: true, head: null });
      core.workspaceRestore.mockResolvedValue(worktree("app", "fix-login"));
      core.sessionsList.mockResolvedValue([record("r1", { workspaceId: "w-app-fix-login" })]);

      await user.click(screen.getByRole("button", { name: /archived \(1\)/ }));
      const archived = screen.getByRole("treeitem", { name: "fix-login" });
      const row = within(archived).getAllByRole("button")[0]!;
      expect(row).toBeDisabled();
      // It cannot be selected, so its own tooltip is the only place its path is written down.
      expect(row.getAttribute("title")).toContain("Archived — was at");

      await openMenu(user);
      expect(screen.queryByRole("menuitem", { name: "Archive…" })).not.toBeInTheDocument();
      await user.click(screen.getByRole("menuitem", { name: "Restore workspace" }));

      expect(core.workspaceRestore).toHaveBeenCalledWith("w-app-fix-login");
      await vi.waitFor(() =>
        expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-app-fix-login"),
      );
      expect(
        screen.queryByRole("treeitem", { name: "Archived workspaces" }),
      ).not.toBeInTheDocument();
    });

    it("offers to restore a workspace whose folder vanished", async () => {
      const user = await withWorktree({ missing: true, head: null });
      core.workspaceRestore.mockResolvedValue(worktree("app", "fix-login"));
      await openMenu(user);
      await user.click(screen.getByRole("menuitem", { name: "Restore from its branch" }));
      expect(core.workspaceRestore).toHaveBeenCalledWith("w-app-fix-login");
      await vi.waitFor(() => expect(screen.queryByText("missing")).not.toBeInTheDocument());
    });
  });

  describe("a worktree removed with git", () => {
    const vanished = (name = "fix-login") =>
      worktree("app", name, { missing: true, branchGone: true, head: null });

    async function withVanished(...extra: ReturnType<typeof worktree>[]) {
      const user = userEvent.setup();
      const app = project("app");
      app.workspaces.push(...extra);
      core.projectsList.mockResolvedValue([app]);
      core.workspaceDelete.mockResolvedValue(undefined);
      render(<Sidebar />);
      await screen.findByRole("treeitem", { name: "app" });
      return user;
    }

    it("offers to delete the workspace its worktree and branch left behind", async () => {
      native.confirm.mockResolvedValue(true);
      await withVanished(vanished());

      await vi.waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));
      expect(native.confirm).toHaveBeenCalledWith(
        expect.stringMatching(/"fix-login" is gone[\s\S]*no branch left to check out/),
        expect.objectContaining({ okLabel: "Delete workspace", cancelLabel: "Keep" }),
      );
      expect(core.workspaceDelete).toHaveBeenCalledWith("w-app-fix-login", false);
      await vi.waitFor(() =>
        expect(screen.queryByRole("treeitem", { name: "fix-login" })).not.toBeInTheDocument(),
      );
    });

    it("keeps the workspace on a no, and does not ask a second time", async () => {
      native.confirm.mockResolvedValue(false);
      await withVanished(vanished());
      await vi.waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));
      expect(core.workspaceDelete).not.toHaveBeenCalled();
      expect(core.uiStateSave).toHaveBeenCalledWith(
        "workspaces.keptVanished",
        JSON.stringify(["w-app-fix-login"]),
      );

      // Coming back to the window asks again about everything still in question — but not this.
      window.dispatchEvent(new Event("focus"));
      await vi.waitFor(() => expect(core.projectsList).toHaveBeenCalledTimes(2));
      expect(native.confirm).toHaveBeenCalledTimes(1);
      const row = screen.getByRole("treeitem", { name: "fix-login" });
      expect(row).toHaveTextContent("gone");
    });

    it("names every workspace it is about to forget", async () => {
      native.confirm.mockResolvedValue(true);
      await withVanished(vanished(), vanished("add-tests"));

      await vi.waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));
      expect(native.confirm.mock.calls[0]![0]).toMatch(/fix-login[\s\S]*add-tests/);
      await vi.waitFor(() => expect(core.workspaceDelete).toHaveBeenCalledTimes(2));
    });

    it("says nothing while the branch is still there to restore from", async () => {
      const user = await withVanished(worktree("app", "fix-login", { missing: true, head: null }));
      await vi.waitFor(() => expect(useProjectsStore.getState().loaded).toBe(true));
      expect(native.confirm).not.toHaveBeenCalled();

      await user.click(screen.getByRole("button", { name: "More actions for fix-login" }));
      expect(screen.getByRole("menuitem", { name: "Restore from its branch" })).toBeInTheDocument();
    });

    it("drops the restore option once there is nothing left to restore from", async () => {
      native.confirm.mockResolvedValue(false);
      const user = await withVanished(vanished());
      await vi.waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));

      await user.click(screen.getByRole("button", { name: "More actions for fix-login" }));
      expect(screen.queryByRole("menuitem", { name: /^Restore/ })).not.toBeInTheDocument();
      expect(screen.getByRole("menuitem", { name: "Delete workspace…" })).toBeInTheDocument();
    });
  });

  it("a project whose folder vanished is marked and cannot be entered", async () => {
    core.projectsList.mockResolvedValue([project("ghost", { missing: true })]);
    render(<Sidebar />);
    const ghost = await screen.findByRole("treeitem", { name: "ghost" });
    expect(ghost).toHaveTextContent("missing");
    expect(
      within(screen.getByRole("treeitem", { name: "local" })).getByRole("button"),
    ).toBeDisabled();
  });
});
