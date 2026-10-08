import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  added,
  project,
  pullRequest,
  pullRequestsOf,
  record,
  repositoriesOf,
  repository,
  task,
  tasksOf,
  worktree,
} from "@/test/fixtures";

const core = vi.hoisted(() => ({
  uiStateLoad: vi.fn(),
  uiStateSave: vi.fn(),
  projectsList: vi.fn(),
  projectOpen: vi.fn(),
  projectCreate: vi.fn(),
  projectClone: vi.fn(),
  forgeRepositories: vi.fn(),
  forgeSearchRepositories: vi.fn(),
  projectRemove: vi.fn(),
  projectsReorder: vi.fn(),
  workspaceDelete: vi.fn(),
  workspaceArchive: vi.fn(),
  workspaceRestore: vi.fn(),
  workspaceRename: vi.fn(),
  workspaceForget: vi.fn(),
  projectUntrackedWorktrees: vi.fn(),
  workspacesImport: vi.fn(),
  sessionsList: vi.fn(),
  projectPullRequests: vi.fn(),
  projectTasks: vi.fn(),
  ptySpawn: vi.fn(),
  ptyClose: vi.fn(),
  memoryWaiting: vi.fn(),
  memoryGet: vi.fn(),
  outcomeLabel: vi.fn(),
  onActivityChanged: vi.fn(),
  workflowList: vi.fn(),
  workflowPreview: vi.fn(),
  harnessesList: vi.fn(),
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
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

import { useAppStore } from "@/stores/app";
import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { useTasksStore } from "@/stores/tasks";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";
import { useUpdatesStore } from "@/stores/updates";
import { useMemoryStore } from "@/stores/memory";
import { useOutcomesStore } from "@/stores/outcomes";
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
    core.workflowList.mockResolvedValue([]);
    core.harnessesList.mockResolvedValue([]);
    for (const fn of [core.uiStateSave, core.projectRemove, core.projectsReorder, core.ptyClose]) {
      fn.mockResolvedValue(undefined);
    }
    core.ptySpawn.mockImplementation(async ({ workspaceId }) => shellIn(workspaceId));
    core.sessionsList.mockResolvedValue([]);
    core.memoryWaiting.mockResolvedValue([]);
    core.onActivityChanged.mockResolvedValue(() => {});
    useMemoryStore.setState({ waiting: {} });
    useOutcomesStore.setState({ asking: null, error: null });
    opener.openUrl.mockResolvedValue(undefined);
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      pullRequests: [],
      problem: null,
      loggedOut: false,
      workspaces: {},
    });
    core.projectTasks.mockResolvedValue(tasksOf([]));
    core.forgeRepositories.mockResolvedValue(repositoriesOf([]));
    useTasksStore.setState({ byProject: {}, closedWanted: false, selected: null, details: {} });
    useSessionsStore.setState({ byWorkspace: {}, error: null });
    usePublishStore.setState({ byProject: {}, workspaceId: null, state: null, busy: null });
    useProjectsStore.setState({
      projects: [],
      pendingClones: [],
      lastParentDir: null,
      repositories: {
        status: "idle",
        list: [],
        total: null,
        problem: null,
        notInstalled: false,
        loggedOut: false,
      },
      loaded: false,
      selectedWorkspaceId: null,
      composingProjectId: null,
      pullRequestsOpen: false,
      tasksOpen: false,
      usageOpen: false,
      workflowId: null,
      collapsed: [],
      error: null,
      notice: null,
    });
    useTerminalStore.setState({ tabs: [], active: {}, error: null });
    useUpdatesStore.setState({ status: null, open: false });
  });

  it("counts every project's open pull requests at the top, and opens and closes the view", async () => {
    core.projectPullRequests.mockImplementation(async (id: string) =>
      id === "p-alpha"
        ? pullRequestsOf([pullRequest(1), pullRequest(2, { state: "merged" })])
        : // The forge has more open than the list holds: the count says so.
          pullRequestsOf([pullRequest(1), pullRequest(3, { draft: true })], { openTotal: 1394 }),
    );
    const user = userEvent.setup();
    await renderSidebar("alpha", "beta");
    const row = await screen.findByRole("button", { name: "Pull requests, 3+ open" });
    expect(row).not.toHaveAttribute("aria-current");

    act(() => useProjectsStore.setState({ usageOpen: true }));
    await user.click(row);
    expect(useProjectsStore.getState().pullRequestsOpen).toBe(true);
    expect(useProjectsStore.getState().usageOpen, "one view at a time").toBe(false);
    expect(row).toHaveAttribute("aria-current", "page");

    await user.click(row);
    expect(useProjectsStore.getState().pullRequestsOpen).toBe(false);
  });

  it("counts the tasks that need an answer under Pull requests, and opens and closes the view", async () => {
    core.projectTasks.mockImplementation(async (id: string) =>
      id === "p-alpha"
        ? tasksOf([task(1, { needsAnswer: true }), task(2)])
        : tasksOf([task(1, { needsAnswer: true }), task(3, { needsAnswer: true })]),
    );
    const user = userEvent.setup();
    await renderSidebar("alpha", "beta");
    const row = await screen.findByRole("button", { name: "Tasks, 3 need an answer" });
    expect(row).not.toHaveAttribute("aria-current");

    act(() => useProjectsStore.setState({ pullRequestsOpen: true }));
    await user.click(row);
    expect(useProjectsStore.getState().tasksOpen).toBe(true);
    expect(useProjectsStore.getState().pullRequestsOpen, "one view at a time").toBe(false);
    expect(row).toHaveAttribute("aria-current", "page");

    await user.click(screen.getByRole("button", { name: /^Pull requests/ }));
    expect(useProjectsStore.getState().tasksOpen, "and the other way round").toBe(false);
    await user.click(row);
    await user.click(row);
    expect(useProjectsStore.getState().tasksOpen).toBe(false);
  });

  it("shows no number on Tasks when nothing is waiting, and says one in the singular", async () => {
    core.projectTasks.mockResolvedValue(tasksOf([task(1), task(2)]));
    await renderSidebar("alpha");
    expect(await screen.findByRole("button", { name: "Tasks" })).toBeVisible();

    act(() =>
      useTasksStore.setState({
        byProject: { "p-alpha": tasksOf([task(1, { needsAnswer: true }), task(2)]) },
      }),
    );
    expect(screen.getByRole("button", { name: "Tasks, 1 needs an answer" })).toBeVisible();
  });

  it("reads tasks once at the start, then only while the Tasks view is open", async () => {
    await renderSidebar("alpha");
    await waitFor(() => expect(core.projectTasks).toHaveBeenCalledTimes(1));
    expect(core.projectTasks).toHaveBeenLastCalledWith("p-alpha", false, false);

    // Coming back to the window with the view closed asks nothing.
    act(() => void window.dispatchEvent(new Event("focus")));
    expect(core.projectTasks).toHaveBeenCalledTimes(1);

    // Opening the view asks, from the core's copy when that is fresh…
    act(() => useProjectsStore.getState().openTasks(true));
    await waitFor(() => expect(core.projectTasks).toHaveBeenCalledTimes(2));
    expect(core.projectTasks).toHaveBeenLastCalledWith("p-alpha", false, false);
    // …and coming back while it is open asks the source again.
    act(() => void window.dispatchEvent(new Event("focus")));
    await waitFor(() => expect(core.projectTasks).toHaveBeenCalledTimes(3));
    expect(core.projectTasks).toHaveBeenLastCalledWith("p-alpha", true, false);

    // Closed tasks are a second question, asked when the view turns to them.
    act(() => useTasksStore.getState().wantClosed(true));
    await waitFor(() => expect(core.projectTasks).toHaveBeenCalledTimes(4));
    expect(core.projectTasks).toHaveBeenLastCalledWith("p-alpha", false, true);

    act(() => useProjectsStore.getState().openTasks(false));
    act(() => void window.dispatchEvent(new Event("focus")));
    expect(core.projectTasks).toHaveBeenCalledTimes(4);
  });

  describe("a workspace started from a task", () => {
    const from = {
      source: "github" as const,
      repo: "github.com/demo/app",
      key: "#91",
      url: "https://github.com/demo/app/issues/91",
      title: "Worktrees on a network drive",
    };
    async function renderStartedFromTask() {
      const alpha = project("alpha");
      const feature = { ...worktree("alpha", "feature"), tasks: [from] };
      core.projectsList.mockResolvedValue([
        { ...alpha, workspaces: [...alpha.workspaces, feature] },
      ]);
      render(<Sidebar />);
      await screen.findByRole("treeitem", { name: "feature" });
    }
    const badge = () =>
      screen.getByRole("button", { name: "Open task #91 — Worktrees on a network drive" });

    it("shows the task's key on its row, which opens the task in the Tasks view", async () => {
      core.projectTasks.mockResolvedValue(tasksOf([task(91, { title: from.title })]));
      const user = userEvent.setup();
      await renderStartedFromTask();
      await waitFor(() => expect(useTasksStore.getState().byProject["p-alpha"]).toBeDefined());
      expect(badge()).toHaveTextContent("#91");

      await user.click(badge());
      expect(useProjectsStore.getState().tasksOpen).toBe(true);
      expect(useTasksStore.getState().selected).toBe("p-alpha#91");
      expect(opener.openUrl).not.toHaveBeenCalled();
    });

    it("opens the task on GitHub when the list has no row for it", async () => {
      // Closed since, or past what the list reads.
      core.projectTasks.mockResolvedValue(tasksOf([task(5)]));
      const user = userEvent.setup();
      await renderStartedFromTask();
      await waitFor(() => expect(useTasksStore.getState().byProject["p-alpha"]).toBeDefined());

      await user.click(badge());
      expect(opener.openUrl).toHaveBeenCalledWith(from.url);
      expect(useProjectsStore.getState().tasksOpen, "not a view with nothing open").toBe(false);
    });

    it("gives the row's one number to the pull request once there is one", async () => {
      core.projectPullRequests.mockResolvedValue(
        pullRequestsOf([pullRequest(95, { branch: "ys/feature" })]),
      );
      await renderStartedFromTask();
      expect(await screen.findByRole("button", { name: /^Open Pull request #95/ })).toBeVisible();
      expect(
        screen.queryByRole("button", { name: /^Open task #91/ }),
        "the task is in the hover, not on the row",
      ).not.toBeInTheDocument();
    });
  });

  it("gives the panel back to a workspace when one is chosen", async () => {
    const user = userEvent.setup();
    await renderWithWorktree();
    await user.click(rowButton("feature"));
    expect(screen.getByRole("treeitem", { name: "feature" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.click(screen.getByRole("button", { name: "Pull requests" }));
    // The workspace is still the one to go back to, but it is not what the panel shows.
    expect(screen.getByRole("treeitem", { name: "feature" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    await user.click(rowButton("feature"));
    expect(useProjectsStore.getState().pullRequestsOpen).toBe(false);
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha-feature");
  });

  it("reads every open pull request once at the start, then only while the view is open", async () => {
    await renderSidebar("alpha");
    await waitFor(() =>
      expect(core.projectPullRequests).toHaveBeenCalledWith("p-alpha", false, true),
    );

    // Coming back to the window with the view closed: the newest fifty, as before.
    core.projectPullRequests.mockClear();
    act(() => void window.dispatchEvent(new Event("focus")));
    await waitFor(() =>
      expect(core.projectPullRequests).toHaveBeenCalledWith("p-alpha", true, false),
    );
    expect(core.projectPullRequests).not.toHaveBeenCalledWith("p-alpha", true, true);

    // Opening the view asks for all of them; so does coming back while it is open.
    core.projectPullRequests.mockClear();
    act(() => useProjectsStore.getState().openPullRequests(true));
    await waitFor(() =>
      expect(core.projectPullRequests).toHaveBeenCalledWith("p-alpha", false, true),
    );
    core.projectPullRequests.mockClear();
    act(() => void window.dispatchEvent(new Event("focus")));
    await waitFor(() =>
      expect(core.projectPullRequests).toHaveBeenCalledWith("p-alpha", true, true),
    );
  });

  it("offers Usage beside Settings, opens and closes it, and hides it when asked", async () => {
    const user = userEvent.setup();
    core.projectsList.mockResolvedValue([]);
    useAppStore.setState({ showUsageInSidebar: true });
    useProjectsStore.setState({ usageOpen: false, workflowId: "wf" });
    const { unmount } = render(<Sidebar />);

    await user.click(screen.getByRole("button", { name: "Usage" }));
    expect(useProjectsStore.getState().usageOpen).toBe(true);
    // It takes the center panel over, like a workflow.
    expect(useProjectsStore.getState().workflowId).toBeNull();
    expect(screen.getByRole("button", { name: "Usage" })).toHaveAttribute("aria-current", "page");
    await user.click(screen.getByRole("button", { name: "Usage" }));
    expect(useProjectsStore.getState().usageOpen).toBe(false);
    unmount();

    useAppStore.setState({ showUsageInSidebar: false });
    render(<Sidebar />);
    expect(screen.queryByRole("button", { name: "Usage" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Settings/ })).toBeInTheDocument();
  });

  it("filters project names, clears the query, and closes search with Escape", async () => {
    await renderSidebar("alpha", "beta");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Search projects" }));
    const input = screen.getByRole("textbox", { name: "Filter projects" });
    expect(input).toHaveFocus();
    await user.type(input, " ALP ");
    expect(screen.getByRole("treeitem", { name: "alpha" })).toBeInTheDocument();
    expect(screen.queryByRole("treeitem", { name: "beta" })).not.toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: "local" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add project" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear project filter" }));
    expect(input).toHaveFocus();
    expect(input).toHaveValue("");
    expect(screen.getByRole("treeitem", { name: "beta" })).toBeInTheDocument();
    await user.type(input, "nonexistent");
    expect(screen.getByRole("status")).toHaveTextContent("No projects match your search.");
    expect(screen.queryByText("No projects yet.")).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("textbox", { name: "Filter projects" })).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Search projects" })).toHaveFocus(),
    );
    expect(screen.getByRole("treeitem", { name: "beta" })).toBeInTheDocument();
  });

  it("lets a mouse user close an empty search field, including after clearing it", async () => {
    await renderSidebar("alpha");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Search projects" }));
    await user.click(screen.getByRole("button", { name: "Close project search" }));
    expect(screen.queryByRole("textbox", { name: "Filter projects" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Search projects" }));
    await user.type(screen.getByRole("textbox", { name: "Filter projects" }), "alpha");
    await user.click(screen.getByRole("button", { name: "Clear project filter" }));
    await user.click(screen.getByRole("button", { name: "Close project search" }));
    expect(screen.queryByRole("textbox", { name: "Filter projects" })).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Search projects" })).toHaveFocus(),
    );
  });

  it("keeps a project added through the dialog visible with a non-matching filter", async () => {
    await renderSidebar("alpha");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Search projects" }));
    await user.type(screen.getByRole("textbox", { name: "Filter projects" }), "missing");
    native.pickFolder.mockResolvedValue("/code/fresh");
    core.projectOpen.mockResolvedValue(added("fresh"));
    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Open a folder/ }));
    expect(await screen.findByRole("treeitem", { name: "fresh" })).toBeInTheDocument();
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-fresh");
    expect(screen.queryByText("No projects match your search.")).not.toBeInTheDocument();
    expect(screen.queryByRole("treeitem", { name: "alpha" })).not.toBeInTheDocument();
  });

  it("updates an already mounted live region when no projects match", async () => {
    await renderSidebar("alpha");
    const user = userEvent.setup();
    const status = screen.getByRole("status", { name: "Project search results" });
    expect(status).toBeEmptyDOMElement();
    await user.click(screen.getByRole("button", { name: "Search projects" }));
    await user.type(screen.getByRole("textbox", { name: "Filter projects" }), "missing");
    expect(screen.getByRole("status", { name: "Project search results" })).toBe(status);
    expect(status).toHaveTextContent("No projects match your search.");
    await user.click(screen.getByRole("button", { name: "Clear project filter" }));
    expect(status).toBeEmptyDOMElement();
  });

  it("preserves selection, collapsed projects and global reorder boundaries while filtering", async () => {
    await renderSidebar("alpha", "beta", "gamma");
    const user = userEvent.setup();
    act(() => useProjectsStore.setState({ selectedWorkspaceId: "w-alpha", collapsed: ["p-beta"] }));
    await user.click(screen.getByRole("button", { name: "Search projects" }));
    await user.type(screen.getByRole("textbox", { name: "Filter projects" }), "beta");
    expect(screen.getByRole("treeitem", { name: "beta" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
    expect(screen.getByRole("treeitem", { name: "alpha" })).toBeInTheDocument();
    expect(screen.queryByRole("treeitem", { name: "gamma" })).not.toBeInTheDocument();
    await user.pointer({ target: rowButton("beta"), keys: "[MouseRight]" });
    expect(screen.getByRole("menuitem", { name: "Move up" })).not.toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Move down" })).not.toBeDisabled();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Clear project filter" }));
    expect(screen.getByRole("treeitem", { name: "alpha" })).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: "beta" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
  });

  it("shows a waiting update beside Settings, and opens the dialog when pressed", async () => {
    await renderSidebar("alpha");
    expect(screen.queryByRole("button", { name: /^Update to/ })).not.toBeInTheDocument();

    act(() =>
      useUpdatesStore.setState({
        status: {
          currentVersion: "0.9.1",
          installKind: "selfUpdating",
          available: { version: "0.9.2", notes: null, url: "https://example.invalid/v0.9.2" },
        },
      }),
    );
    await userEvent.setup().click(screen.getByRole("button", { name: "Update to 0.9.2" }));
    expect(useUpdatesStore.getState().open).toBe(true);
  });

  it("puts a workspace's pull request on its row, coloured by its checks", async () => {
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      pullRequests: [
        {
          number: 42,
          url: "https://github.com/o/alpha/pull/42",
          title: "Fix the login redirect",
          branch: "ys/feature",
          state: "open",
          draft: false,
          checks: "failing",
        },
      ],
      problem: null,
      loggedOut: false,
      workspaces: {},
    });
    await renderWithWorktree();

    const row = await screen.findByRole("treeitem", { name: "feature" });
    const badge = await within(row).findByRole("button", { name: /Pull request #42/ });
    expect(badge).toHaveTextContent("#42");
    expect(badge).toHaveAccessibleName(
      "Open Pull request #42 — checks failing — Fix the login redirect",
    );
    // `local` is on `main`, which no pull request has as its head.
    expect(
      within(screen.getByRole("treeitem", { name: "local" })).queryByRole("button", {
        name: /Pull request/,
      }),
    ).not.toBeInTheDocument();
  });

  it("counts a workspace's other pull requests on its row and lists them on hover", async () => {
    const opened = (number: number, branch: string, title: string, state = "open") => ({
      number,
      url: `https://github.com/o/alpha/pull/${number}`,
      title,
      branch,
      state,
      draft: false,
      checks: "passing",
      details: null,
    });
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      pullRequests: [
        opened(43, "ys/feature-part-two", "Split out the migration"),
        opened(42, "ys/feature", "Fix the login redirect"),
        opened(30, "ys/feature", "First go at it", "merged"),
        opened(41, "ys/someone-else", "Not this workspace's"),
      ],
      problem: null,
      loggedOut: false,
      workspaces: { "w-alpha-feature": [43, 42, 30] },
    });
    await renderWithWorktree();

    const row = await screen.findByRole("treeitem", { name: "feature" });
    // The checked-out branch's own pull request is the badge; the others are counted.
    expect(await within(row).findByRole("button", { name: /Pull request #42/ })).toBeVisible();
    expect(
      within(row).getByLabelText("2 more pull requests from this workspace"),
    ).toHaveTextContent("+2");
    await userEvent.setup().hover(rowButton("feature"));
    const preview = await screen.findByRole("region", { name: "feature details" });
    expect(within(preview).getByText("Also opened from this workspace")).toBeVisible();
    expect(within(preview).getByRole("button", { name: /Pull request #43/ })).toBeVisible();
    expect(within(preview).getByRole("button", { name: /#30 — merged/ })).toBeVisible();
    expect(within(preview).queryByText("Not this workspace's")).not.toBeInTheDocument();
  });

  it("opens the pull request in a browser, without opening the workspace", async () => {
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      pullRequests: [
        {
          number: 42,
          url: "https://github.com/o/alpha/pull/42",
          title: "Fix the login redirect",
          branch: "ys/feature",
          state: "open",
          draft: false,
          checks: "passing",
        },
      ],
      problem: null,
      loggedOut: false,
      workspaces: {},
    });
    await renderWithWorktree();

    const row = await screen.findByRole("treeitem", { name: "feature" });
    const badge = await within(row).findByRole("button", { name: /Pull request #42/ });
    await userEvent.setup().click(badge);

    expect(opener.openUrl).toHaveBeenCalledWith("https://github.com/o/alpha/pull/42");
    // The row's own button opens the workspace; this one must not.
    expect(useProjectsStore.getState().selectedWorkspaceId).not.toBe("w-alpha-feature");
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  it("previews the workspace PR on hover without selecting it", async () => {
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      loggedOut: false,
      workspaces: {},
      problem: null,
      pullRequests: [
        {
          number: 42,
          url: "https://github.com/demo/app/pull/42",
          title: "Fix the login redirect",
          branch: "ys/feature",
          state: "open",
          draft: false,
          checks: "passing",
          details: null,
        },
      ],
    });
    await renderWithWorktree();
    const user = userEvent.setup();
    await user.hover(rowButton("feature"));
    const preview = await screen.findByRole("region", { name: "feature details" });
    expect(within(preview).getByText("Fix the login redirect")).toBeVisible();
    expect(useProjectsStore.getState().selectedWorkspaceId).not.toBe("w-alpha-feature");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: "feature details" })).not.toBeInTheDocument();
  });

  it("keeps previews closed after clicking a row, but opens them when tabbing to it", async () => {
    await renderWithWorktree();
    const user = userEvent.setup();
    await user.click(rowButton("feature"));
    expect(screen.queryByRole("region", { name: "feature details" })).not.toBeInTheDocument();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });
    expect(screen.queryByRole("region", { name: "feature details" })).not.toBeInTheDocument();
    await user.click(rowButton("local"));
    const feature = rowButton("feature");
    // jsdom retains :focus-visible history from removed search inputs across tests. Supply
    // the browser's keyboard-focus match while still exercising the real Tab/focus events.
    const matches = feature.matches.bind(feature);
    const focusVisible = vi
      .spyOn(feature, "matches")
      .mockImplementation((selector) => (selector === ":focus-visible" ? true : matches(selector)));
    try {
      await user.tab();
      expect(feature).toHaveFocus();
      expect(screen.getByRole("region", { name: "feature details" })).toBeVisible();
    } finally {
      focusVisible.mockRestore();
    }
  });

  it("offers a code review from a workspace's menu, in the Run dialog for that workspace", async () => {
    core.workflowList.mockResolvedValue([]);
    core.harnessesList.mockResolvedValue([]);
    core.workflowPreview.mockResolvedValue({
      needsPullRequest: true,
      pullRequest: null,
      pullRequestProblem: "`feature` has no open pull request.",
      needsOrigin: true,
    });
    await renderWithWorktree();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "More actions for feature" }));
    expect(screen.getByRole("menuitem", { name: "Run workflow…" })).toBeInTheDocument();
    await user.click(screen.getByRole("menuitem", { name: "Request code review…" }));
    const dialog = await screen.findByRole("dialog", { name: /Run/ });
    expect(within(dialog).queryByRole("combobox", { name: "Workspace" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("does not reopen the workspace preview over its action menu", async () => {
    await renderWithWorktree();
    const user = userEvent.setup();
    await user.hover(rowButton("feature"));
    await screen.findByRole("region", { name: "feature details" });
    await user.click(screen.getByRole("button", { name: "More actions for feature" }));
    expect(screen.getByRole("menu")).toBeVisible();
    expect(screen.queryByRole("region", { name: "feature details" })).not.toBeInTheDocument();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });
    expect(screen.queryByRole("region", { name: "feature details" })).not.toBeInTheDocument();
  });

  it.each(["loading", "not installed", "logged out", "forge error", "IPC error"])(
    "does not claim no PR was found when the forge is %s",
    async (status) => {
      if (status === "loading")
        core.projectPullRequests.mockImplementationOnce(() => new Promise(() => {}));
      else if (status === "IPC error")
        core.projectPullRequests.mockRejectedValueOnce(new Error("Could not fetch PRs"));
      else
        core.projectPullRequests.mockResolvedValueOnce({
          gh: status !== "not installed",
          pullRequests: [],
          loggedOut: status === "logged out",
          problem: status === "forge error" ? "Network unavailable" : null,
          workspaces: {},
        });
      await renderWithWorktree();
      await userEvent.setup().hover(rowButton("feature"));
      const preview = await screen.findByRole("region", { name: "feature details" });
      expect(within(preview).getByText("ys/feature")).toBeVisible();
      expect(within(preview).queryByText("No pull request found")).not.toBeInTheDocument();
    },
  );

  it("says no PR was found after a successful empty response", async () => {
    await renderWithWorktree();
    await userEvent.setup().hover(rowButton("feature"));
    const preview = await screen.findByRole("region", { name: "feature details" });
    expect(within(preview).getByText("No pull request found")).toBeVisible();
  });

  it("counts open harness tabs, shows names and activity, and switches to the chosen one", async () => {
    await renderWithWorktree();
    act(() => {
      useSessionsStore.setState({
        byWorkspace: {
          "w-alpha-feature": [
            record("r1", { harnessId: "claude", harnessLabel: "Claude Code" }),
            record("r2", { harnessId: "codex", harnessLabel: "Codex" }),
          ],
        },
      });
      useTerminalStore.setState({
        tabs: [
          {
            id: "t1",
            workspaceId: "w-alpha-feature",
            title: "claude",
            recordId: "r1",
            exit: null,
            busy: true,
            attention: false,
          },
          {
            id: "t2",
            workspaceId: "w-alpha-feature",
            title: "codex",
            recordId: "r2",
            exit: null,
            busy: false,
            attention: true,
          },
          {
            id: "t3",
            workspaceId: "w-alpha-feature",
            title: "bash",
            recordId: null,
            exit: null,
            busy: false,
            attention: false,
          },
        ],
      });
    });
    const user = userEvent.setup();
    await user.hover(screen.getByRole("button", { name: "2 open harnesses" }));
    const preview = await screen.findByRole("region", { name: "Workspace harnesses" });
    expect(within(preview).getByText("Claude Code")).toBeVisible();
    expect(within(preview).getByText("Working")).toBeVisible();
    expect(within(preview).getByText("Waiting")).toBeVisible();
    expect(screen.queryByRole("region", { name: "feature details" })).not.toBeInTheDocument();
    await user.click(within(preview).getByRole("button", { name: /Codex/ }));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha-feature");
    expect(useTerminalStore.getState().active["w-alpha-feature"]).toBe("t2");
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  it("says a pull request was merged instead of showing its number", async () => {
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      pullRequests: [
        {
          number: 42,
          url: "https://github.com/o/alpha/pull/42",
          title: "Fix the login redirect",
          branch: "ys/feature",
          state: "merged",
          draft: false,
          checks: "passing",
        },
      ],
      problem: null,
      loggedOut: false,
      workspaces: {},
    });
    await renderWithWorktree();
    const row = await screen.findByRole("treeitem", { name: "feature" });
    expect(await within(row).findByRole("button", { name: /merged/ })).toHaveTextContent("merged");
  });

  it("shows no pull requests at all without gh, and does not complain about it", async () => {
    core.projectPullRequests.mockResolvedValue({
      gh: false,
      pullRequests: [],
      problem: null,
      loggedOut: false,
      workspaces: {},
    });
    await renderWithWorktree();
    // No badge on any row. The Pull requests row at the top is still there, without a count.
    expect(screen.queryByRole("button", { name: /pull request #/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pull requests" })).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
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

  it("clones a GitHub repository and selects its local workspace", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    core.projectClone.mockResolvedValue(added("cloned"));
    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Clone a repository/ }));
    expect(screen.getByRole("button", { name: "Clone project" })).toBeDisabled();
    await user.type(screen.getByLabelText("Repository"), "owner/repo");
    await user.clear(screen.getByLabelText("Name"));
    await user.type(screen.getByLabelText("Name"), "cloned");
    await user.type(screen.getByLabelText("Location"), "/code");
    await user.click(screen.getByRole("button", { name: "Clone project" }));
    expect(core.projectClone).toHaveBeenCalledWith("owner/repo", "cloned", "/code", null);
    expect(await screen.findByRole("treeitem", { name: "cloned" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-cloned");
  });

  it("shows a foreground clone failure and lets the user retry", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    let fail!: (reason: unknown) => void;
    core.projectClone.mockReturnValue(
      new Promise((_, reject) => {
        fail = reject;
      }),
    );
    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Clone a repository/ }));
    await user.type(screen.getByLabelText("Repository"), "owner/private");
    await user.type(screen.getByLabelText("Location"), "/code");
    await user.click(screen.getByRole("button", { name: "Clone project" }));
    expect(screen.getByRole("button", { name: "Cloning…" })).toBeDisabled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await act(async () => fail({ code: "clone_failed", message: "Check your git credentials." }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Check your git credentials.");
    expect(screen.getByRole("button", { name: "Clone project" })).toBeEnabled();
    expect(screen.queryByText("Cloning private…")).not.toBeInTheDocument();
    core.projectClone.mockResolvedValue(added("private"));
    await user.click(screen.getByRole("button", { name: "Clone project" }));
    expect(await screen.findByRole("treeitem", { name: "private" })).toBeInTheDocument();
  });

  it.each(["Escape", "button", "backdrop"])(
    "keeps cloning after closing via %s without stealing selection or closing a new dialog",
    async (close) => {
      const user = userEvent.setup();
      await renderSidebar("alpha");
      let finish!: (value: ReturnType<typeof added>) => void;
      core.projectClone.mockReturnValue(
        new Promise((resolve) => {
          finish = resolve;
        }),
      );
      await user.click(screen.getByRole("button", { name: "Add project" }));
      await user.click(screen.getByRole("button", { name: /Clone a repository/ }));
      await user.type(screen.getByLabelText("Repository"), "owner/repo");
      await user.type(screen.getByLabelText("Location"), "/code");
      await user.click(screen.getByRole("button", { name: "Clone project" }));
      if (close === "Escape") await user.keyboard("{Escape}");
      else if (close === "button")
        await user.click(screen.getByRole("button", { name: "Run in background" }));
      else await user.click(screen.getByRole("dialog").parentElement!);
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(screen.getByText("Cloning repo…")).toBeInTheDocument();
      await user.click(rowButton("local"));
      expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
      await user.click(screen.getByRole("button", { name: "Add project" }));
      await act(async () => finish(added("repo")));
      expect(await screen.findByRole("treeitem", { name: "repo" })).toBeInTheDocument();
      expect(screen.queryByText("Cloning repo…")).not.toBeInTheDocument();
      expect(screen.getByRole("dialog", { name: "Add a project" })).toBeInTheDocument();
      expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
      expect(core.ptySpawn).not.toHaveBeenCalledWith(
        expect.objectContaining({ workspaceId: "w-repo" }),
      );
      await user.keyboard("{Escape}");
      expect(screen.getByText("repo cloned.")).toBeInTheDocument();
    },
  );

  it("reports a background clone failure in the sidebar", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    let fail!: (reason: unknown) => void;
    core.projectClone.mockReturnValue(
      new Promise((_, reject) => {
        fail = reject;
      }),
    );
    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Clone a repository/ }));
    await user.type(screen.getByLabelText("Repository"), "owner/private");
    await user.type(screen.getByLabelText("Location"), "/code");
    await user.click(screen.getByRole("button", { name: "Clone project" }));
    await user.click(screen.getByRole("button", { name: "Run in background" }));
    await act(async () => fail({ code: "clone_failed", message: "Check your git credentials." }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "private: Check your git credentials.",
    );
    expect(screen.queryByText("Cloning private…")).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it.each([
    "https://github.com/owner/repo.git/",
    "git@github.com:owner/repo.git",
    "ssh://git@github.com/owner/repo",
    "owner/repo",
    "www.github.com/owner/repo",
  ])("suggests a folder name from %s until the user edits it", async (repository) => {
    const user = userEvent.setup();
    await renderSidebar();
    native.pickFolder.mockResolvedValue("/code");
    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Clone a repository/ }));
    const input = screen.getByLabelText("Repository");
    const name = screen.getByLabelText("Name");
    await user.type(input, repository);
    expect(name).toHaveValue("repo");
    await user.clear(input);
    expect(name).toHaveValue("");
    await user.type(input, "owner/renamed.git");
    expect(name).toHaveValue("renamed");
    await user.clear(name);
    await user.type(name, "custom-folder");
    await user.clear(input);
    await user.type(input, "owner/another");
    expect(name).toHaveValue("custom-folder");
    await user.clear(name);
    await user.type(input, "-changed");
    expect(name).toHaveValue("");
    await user.click(screen.getByRole("button", { name: "Browse…" }));
    expect(native.pickFolder).toHaveBeenCalledWith("Clone the repository into…", undefined);
  });

  async function openCloneStep(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: "Add project" }));
    await user.click(screen.getByRole("button", { name: /Clone a repository/ }));
  }

  it("lists the account's repositories, filters them as you type, and clones the chosen one", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    core.forgeRepositories.mockResolvedValue(
      repositoriesOf([
        repository("o/weather-cli", { language: "Rust", description: "A forecast" }),
        repository("o/weather-api"),
        repository("o/notes", { isPrivate: true, isArchived: true }),
      ]),
    );
    core.projectClone.mockResolvedValue(added("weather-api"));
    useProjectsStore.setState({ lastParentDir: "/code" });
    await openCloneStep(user);
    const list = await screen.findByRole("listbox", { name: "Your repositories" });
    expect(within(list).getAllByRole("option")).toHaveLength(3);
    expect(within(list).getByRole("option", { name: "o/notes" })).toHaveTextContent(
      "private · archived",
    );
    // The remembered location is a line, not a question.
    expect(screen.queryByLabelText("Location")).not.toBeInTheDocument();
    expect(screen.getByText("/code")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Repository"), "api");
    expect(
      within(list)
        .getAllByRole("option")
        .map((o) => o.getAttribute("aria-label") ?? o.textContent),
    ).toEqual(["o/weather-api", "Search GitHub for “api”…"]);
    await user.click(within(list).getByRole("option", { name: "o/weather-api" }));
    expect(screen.getByLabelText("Repository")).toHaveValue("o/weather-api");
    expect(screen.getByLabelText("Name")).toHaveValue("weather-api");
    expect(screen.getByText("/code/weather-api")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clone project" }));
    expect(core.projectClone).toHaveBeenCalledWith(
      "https://github.com/o/weather-api.git",
      "weather-api",
      "/code",
      null,
    );
    expect(await screen.findByRole("treeitem", { name: "weather-api" })).toBeInTheDocument();
  });

  it("walks the list with the keyboard, says which are already projects, and goes to one instead of cloning it", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha");
    core.forgeRepositories.mockResolvedValue(
      repositoriesOf([
        repository("o/alpha", { projectId: "p-alpha" }),
        repository("o/beta", { isFork: true, parent: "upstream/beta" }),
      ]),
    );
    core.projectClone.mockResolvedValue(added("beta"));
    useProjectsStore.setState({ lastParentDir: "/code" });
    await openCloneStep(user);
    const list = await screen.findByRole("listbox", { name: "Your repositories" });
    expect(within(list).getByRole("option", { name: "o/alpha, already added" })).toHaveTextContent(
      "Already added",
    );
    await user.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByLabelText("Repository")).toHaveValue("o/beta");
    expect(screen.getByText(/upstream\/beta becomes its upstream remote/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clone project" }));
    expect(core.projectClone).toHaveBeenCalledWith(
      "https://github.com/o/beta.git",
      "beta",
      "/code",
      "https://github.com/upstream/beta.git",
    );
    await screen.findByRole("treeitem", { name: "beta" });

    await openCloneStep(user);
    await user.keyboard("{Enter}");
    expect(screen.getByLabelText("Repository")).toHaveValue("o/alpha");
    const go = screen.getByRole("button", { name: "Go to project" });
    await user.click(go);
    expect(core.projectClone).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha");
  });

  it("searches the forge only when asked, and comes back to the list", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    core.forgeRepositories.mockResolvedValue(
      repositoriesOf([repository("o/mine")], { total: 230 }),
    );
    core.forgeSearchRepositories.mockResolvedValue([repository("acme/tools")]);
    await openCloneStep(user);
    const list = await screen.findByRole("listbox", { name: "Your repositories" });
    expect(within(list).getByText(/Showing the 1 most recently pushed of 230/)).toBeInTheDocument();
    await user.type(screen.getByLabelText("Repository"), "tools");
    expect(within(list).getByText("Nothing of yours is named like “tools”.")).toBeInTheDocument();
    expect(core.forgeSearchRepositories).not.toHaveBeenCalled();
    await user.click(screen.getByRole("option", { name: "Search GitHub for “tools”…" }));
    expect(core.forgeSearchRepositories).toHaveBeenCalledWith("tools");
    const found = await screen.findByRole("listbox", { name: "Repositories on GitHub" });
    expect(within(found).getByRole("option", { name: "acme/tools" })).toBeInTheDocument();
    expect(within(found).getByText(/On GitHub, the 1 the forge ranks first/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Your repositories" }));
    expect(screen.getByRole("listbox", { name: "Your repositories" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Search GitHub for “tools”…" })).toBeInTheDocument();
  });

  it("still takes a URL without gh, and says why there is no list", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    core.forgeRepositories.mockRejectedValue({
      code: "gh_not_installed",
      message: "gh is not installed, or not on PATH",
    });
    core.projectClone.mockResolvedValue(added("repo"));
    await openCloneStep(user);
    expect(await screen.findByText(/needs the GitHub CLI/)).toBeInTheDocument();
    await user.type(screen.getByLabelText("Repository"), "https://github.com/owner/repo.git");
    expect(screen.getByRole("option", { name: /^Clone https:/ })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /Search GitHub/ })).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Location"), "/code");
    await user.keyboard("{Enter}");
    expect(core.projectClone).toHaveBeenCalledWith(
      "https://github.com/owner/repo.git",
      "repo",
      "/code",
      null,
    );
  });

  it("keeps what was read when a page fails, and offers to retry when logged out", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    core.forgeRepositories.mockResolvedValueOnce(
      repositoriesOf([repository("o/first")], { problem: "`gh api graphql` failed: 502" }),
    );
    await openCloneStep(user);
    const list = await screen.findByRole("listbox", { name: "Your repositories" });
    expect(within(list).getByRole("option", { name: "o/first" })).toBeInTheDocument();
    expect(
      within(list).getByText(/Stopped short: `gh api graphql` failed: 502/),
    ).toBeInTheDocument();
    core.forgeRepositories.mockResolvedValueOnce(repositoriesOf([], { loggedOut: true }));
    await user.click(within(list).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText(/Nobody is logged in to the GitHub CLI/)).toBeInTheDocument();
    expect(core.forgeRepositories).toHaveBeenCalledTimes(2);
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

  it("removing a project asks first, keeps its history by default, and closes its sessions", async () => {
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

    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    await user.click(screen.getByRole("menuitem", { name: /Remove from Yardsort/ }));
    let dialog = screen.getByRole("dialog", { name: /Remove “alpha”/ });
    expect(dialog).toHaveTextContent("Nothing on disk is deleted");
    expect(dialog).toHaveTextContent("1 running terminal session in this project will be closed");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(core.projectRemove).not.toHaveBeenCalled();
    expect(screen.getByRole("treeitem", { name: "alpha" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    await user.click(screen.getByRole("menuitem", { name: /Remove from Yardsort/ }));
    dialog = screen.getByRole("dialog", { name: /Remove “alpha”/ });
    expect(within(dialog).getByRole("checkbox", { name: /Also delete/ })).not.toBeChecked();
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    expect(core.ptyClose).toHaveBeenCalledWith("s-w-alpha");
    expect(core.projectRemove).toHaveBeenCalledWith("p-alpha", true);
    expect(screen.queryByRole("treeitem", { name: "alpha" })).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("removing a project drops its history only when the box is ticked", async () => {
    const user = userEvent.setup();
    await renderSidebar("alpha");
    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    await user.click(screen.getByRole("menuitem", { name: /Remove from Yardsort/ }));
    const dialog = screen.getByRole("dialog", { name: /Remove “alpha”/ });
    await user.click(within(dialog).getByRole("checkbox", { name: /Also delete/ }));
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    expect(core.projectRemove).toHaveBeenCalledWith("p-alpha", false);
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

    it("asks once how the attempt went, and records the answer", async () => {
      native.confirm.mockResolvedValue(true);
      core.workspaceDelete.mockResolvedValue(undefined);
      core.outcomeLabel.mockResolvedValue({
        projectId: "p-app",
        attempts: [],
        agents: [],
        minSample: 5,
      });
      const user = await openDeleteMenu();

      const prompt = await screen.findByRole("status", { name: "How did it go?" });
      expect(prompt).toHaveTextContent("How did fix-login go?");
      await user.click(within(prompt).getByRole("button", { name: "Partly" }));
      expect(core.outcomeLabel).toHaveBeenCalledWith("w-app-fix-login", "partly");
      await waitFor(() =>
        expect(screen.queryByRole("status", { name: "How did it go?" })).not.toBeInTheDocument(),
      );
    });

    it("does not ask when the delete did not happen, and can be dismissed", async () => {
      core.workspaceDelete.mockRejectedValueOnce({ code: "worktree_dirty", message: "dirty" });
      native.confirm.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
      const user = await openDeleteMenu();
      expect(screen.queryByRole("status", { name: "How did it go?" })).not.toBeInTheDocument();

      useOutcomesStore.getState().ask("w-app-fix-login", "fix-login");
      await user.click(await screen.findByRole("button", { name: "Not now" }));
      expect(screen.queryByRole("status", { name: "How did it go?" })).not.toBeInTheDocument();
      expect(core.outcomeLabel).not.toHaveBeenCalled();
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

  describe("forgetting a workspace", () => {
    async function openForget() {
      const user = userEvent.setup();
      const app = project("app");
      app.workspaces.push(
        worktree("app", "theirs", {
          head: { label: "experiment", detached: false, unborn: false },
        }),
      );
      core.projectsList.mockResolvedValue([app]);
      core.workspaceForget.mockResolvedValue(undefined);
      render(<Sidebar />);
      await screen.findByRole("treeitem", { name: "theirs" });
      await user.click(screen.getByRole("button", { name: "More actions for theirs" }));
      await user.click(screen.getByRole("menuitem", { name: "Forget…" }));
      return { user, dialog: await screen.findByRole("dialog", { name: /Forget workspace/ }) };
    }

    it("takes the workspace out of Yardsort and keeps its folder, branch and history", async () => {
      core.sessionsList.mockResolvedValue([record("r1", { workspaceId: "w-app-theirs" })]);
      const { user, dialog } = await openForget();

      expect(dialog).toHaveTextContent("The folder stays where it is, and so does the branch");
      expect(dialog).toHaveTextContent("experiment");
      const box = await within(dialog).findByRole("checkbox", {
        name: /Also delete its 1 saved conversation/,
      });
      expect(box).not.toBeChecked();
      await user.click(within(dialog).getByRole("button", { name: "Forget" }));

      expect(core.workspaceForget).toHaveBeenCalledWith("w-app-theirs", true);
      expect(core.workspaceDelete).not.toHaveBeenCalled();
      expect(screen.queryByRole("treeitem", { name: "theirs" })).not.toBeInTheDocument();
    });

    it("drops the history too only when the box is ticked", async () => {
      core.sessionsList.mockResolvedValue([record("r1"), record("r2")]);
      const { user, dialog } = await openForget();
      await user.click(
        await within(dialog).findByRole("checkbox", {
          name: /Also delete its 2 saved conversations/,
        }),
      );
      await user.click(within(dialog).getByRole("button", { name: "Forget" }));
      expect(core.workspaceForget).toHaveBeenCalledWith("w-app-theirs", false);
    });

    it("leaves everything alone on Cancel", async () => {
      const { user, dialog } = await openForget();
      await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(core.workspaceForget).not.toHaveBeenCalled();
      expect(screen.getByRole("treeitem", { name: "theirs" })).toBeInTheDocument();
    });
  });

  describe("importing worktrees", () => {
    const found = [
      { path: "/elsewhere/app/one", branch: "one" },
      { path: "/elsewhere/app/two", branch: null },
    ];

    async function openImport() {
      const user = userEvent.setup();
      await renderSidebar("app");
      await user.click(screen.getByRole("button", { name: "More actions for app" }));
      await user.click(screen.getByRole("menuitem", { name: "Import worktrees…" }));
      return { user, dialog: await screen.findByRole("dialog", { name: /Import worktrees/ }) };
    }

    it("lists what git has that Yardsort does not, all chosen, and imports the chosen ones", async () => {
      core.projectUntrackedWorktrees.mockResolvedValue(found);
      core.workspacesImport.mockResolvedValue([
        worktree("app", "one", { path: "/elsewhere/app/one" }),
      ]);
      const { user, dialog } = await openImport();

      const one = await within(dialog).findByRole("checkbox", { name: "one" });
      const two = within(dialog).getByRole("checkbox", { name: "two" });
      expect(one).toBeChecked();
      expect(two).toBeChecked();
      expect(dialog).toHaveTextContent("/elsewhere/app/one");
      expect(dialog).toHaveTextContent("detached");
      expect(core.projectUntrackedWorktrees).toHaveBeenCalledWith("p-app");

      await user.click(two);
      await user.click(within(dialog).getByRole("button", { name: "Import 1 worktree" }));

      expect(core.workspacesImport).toHaveBeenCalledWith("p-app", ["/elsewhere/app/one"]);
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(await screen.findByRole("treeitem", { name: "one" })).toBeInTheDocument();
    });

    it("says so when there is nothing to import", async () => {
      core.projectUntrackedWorktrees.mockResolvedValue([]);
      const { dialog } = await openImport();
      expect(await within(dialog).findByText(/Nothing to import/)).toBeInTheDocument();
      expect(within(dialog).queryByRole("button", { name: /^Import/ })).not.toBeInTheDocument();
    });

    it("shows why an import failed and keeps the dialog open", async () => {
      core.projectUntrackedWorktrees.mockResolvedValue([found[0]]);
      core.workspacesImport.mockRejectedValue({
        code: "not_importable",
        message: "not a worktree",
      });
      const { user, dialog } = await openImport();
      await within(dialog).findByRole("checkbox", { name: "one" });
      await user.click(within(dialog).getByRole("button", { name: "Import 1 worktree" }));
      expect(await within(dialog).findByRole("alert")).toHaveTextContent("not a worktree");
      expect(screen.queryByRole("treeitem", { name: "one" })).not.toBeInTheDocument();
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

  it("counts memory proposals on the project row, and opens the Memory view from it", async () => {
    const user = userEvent.setup();
    core.memoryWaiting.mockResolvedValue([{ projectId: "p-alpha", count: 2 }]);
    core.memoryGet.mockResolvedValue({
      projectId: "p-alpha",
      shared: false,
      preview: null,
      entries: [],
    });
    await renderSidebar("alpha", "beta");
    const count = await screen.findByRole("button", {
      name: "2 memory proposals waiting in alpha",
    });
    expect(
      screen.queryByRole("button", { name: /memory proposals? waiting in beta/ }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "More actions for alpha" }));
    expect(screen.getByRole("menuitem", { name: "Memory… (2 waiting)" })).toBeInTheDocument();
    await user.keyboard("{Escape}");

    await user.click(count);
    expect(await screen.findByRole("dialog", { name: "Memory — alpha" })).toBeInTheDocument();
    expect(core.memoryGet).toHaveBeenCalledWith("p-alpha");
  });
});
