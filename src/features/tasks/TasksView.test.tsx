import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectTasks } from "@/lib/ipc";
import { project, task, taskDetail, tasksOf, worktree } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  uiStateSave: vi.fn(),
  projectTasks: vi.fn(),
  taskDetail: vi.fn(),
  taskPrompt: vi.fn(),
  sessionsList: vi.fn(),
  ptySpawn: vi.fn(),
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
const clipboard = vi.hoisted(() => ({ writeText: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
vi.mock("@tauri-apps/plugin-opener", () => opener);
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);

import { useProjectsStore } from "@/stores/projects";
import { useTasksStore } from "@/stores/tasks";
import { FILTERS_KEY } from "./filters";
import { TasksView } from "./TasksView";

const alpha = project("alpha");
const beta = project("beta");
const bug = { name: "bug", color: "d73a4a" };

const drive = task(12, {
  title: "Worktrees on a network drive",
  author: "grace",
  labels: [bug],
  assignees: ["ada"],
  comments: 3,
  needsAnswer: true,
  linkedPullRequests: [14],
  updatedAt: "2026-09-30T10:00:00Z",
});
const daemon = task(9, {
  title: "Document the daemon",
  author: "ada",
  labels: [{ name: "docs", color: "0075ca" }],
  updatedAt: "2026-09-29T10:00:00Z",
});
const crash = task(4, {
  title: "Crash on an empty repository",
  author: "ken",
  state: "closed",
  closedAs: "notPlanned",
  updatedAt: "2026-09-20T10:00:00Z",
});
const dark = task(2, {
  title: "Dark mode",
  author: "ken",
  updatedAt: "2026-09-28T10:00:00Z",
});

const answers = (): Record<string, ProjectTasks> => ({
  [alpha.id]: tasksOf([drive, daemon, crash], { closed: true }),
  [beta.id]: tasksOf([dark], {
    repo: { host: "github.com", owner: "demo", name: "site", kind: "github" },
  }),
});

function show(byProject = answers(), ui: Record<string, string> = {}) {
  useTasksStore.setState({ byProject });
  useProjectsStore.setState({ ui });
  render(<TasksView />);
  return userEvent.setup();
}

const row = (title: RegExp) => screen.getByRole("button", { name: title });
const titles = () =>
  within(screen.getByRole("list", { name: "Tasks" }))
    .getAllByRole("listitem")
    .map((item) => item.querySelector("[data-task-title]")?.textContent);
/** What the filters were last saved as, in the core's `ui_state`. */
const saved = () => {
  const calls = core.uiStateSave.mock.calls.filter((call) => call[0] === FILTERS_KEY);
  const last = calls[calls.length - 1];
  return last ? JSON.parse(last[1] as string) : null;
};

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  core.uiStateSave.mockResolvedValue(undefined);
  core.projectTasks.mockImplementation(async (id: string) => answers()[id]);
  core.taskDetail.mockImplementation(async (id: string, key: string) =>
    taskDetail(answers()[id]!.tasks.find((it) => it.key === key)!),
  );
  opener.openUrl.mockResolvedValue(undefined);
  clipboard.writeText.mockResolvedValue(undefined);
  useProjectsStore.setState({
    projects: [alpha, beta],
    loaded: true,
    ui: {},
    collapsed: [],
    selectedWorkspaceId: null,
    composingProjectId: null,
    workflowId: null,
    usageOpen: false,
    pullRequestsOpen: false,
    tasksOpen: true,
    error: null,
  });
  core.sessionsList.mockResolvedValue([]);
  core.ptySpawn.mockResolvedValue({ id: "s1", labels: {} });
  core.taskPrompt.mockImplementation(async (_id: string, key: string) => ({
    prompt: `Work on this GitHub issue: ${key}`,
    task: linkTo(Number(key.slice(1))),
  }));
  useTasksStore.setState({
    byProject: {},
    closedWanted: false,
    selected: null,
    details: {},
    busy: null,
    error: null,
  });
});

/** What a workspace remembers of the task it was started from. */
const linkTo = (number: number) => ({
  source: "github" as const,
  repo: "github.com/demo/app",
  key: `#${number}`,
  url: `https://github.com/demo/app/issues/${number}`,
  title: `Task ${number}`,
});

describe("the list", () => {
  it("shows every project's open tasks, most recently changed first", () => {
    show();
    expect(titles()).toEqual(["Worktrees on a network drive", "Document the daemon", "Dark mode"]);
    const first = row(/Worktrees on a network drive/);
    expect(first).toHaveTextContent("alpha");
    expect(first).toHaveTextContent("#12");
    expect(first).toHaveTextContent("Open");
    expect(first).toHaveTextContent("Needs an answer");
    expect(first).toHaveTextContent("grace");
    expect(first).toHaveTextContent("bug");
    expect(first).toHaveTextContent("→ ada");
    expect(first).toHaveTextContent("PR #14");
    expect(within(first).getByLabelText("3 comments")).toBeVisible();
    expect(row(/Document the daemon/)).not.toHaveTextContent("Needs an answer");
  });

  it("moves between rows with Up, Down, Home and End", async () => {
    const user = show();
    row(/Worktrees/).focus();
    await user.keyboard("{ArrowDown}");
    expect(row(/Document the daemon/)).toHaveFocus();
    await user.keyboard("{End}");
    expect(row(/Dark mode/)).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(row(/Dark mode/), "stops at the end").toHaveFocus();
    await user.keyboard("{Home}{ArrowUp}");
    expect(row(/Worktrees/)).toHaveFocus();
  });

  it("closes from its header", async () => {
    const user = show();
    await user.click(screen.getByRole("button", { name: "Close tasks" }));
    expect(useProjectsStore.getState().tasksOpen).toBe(false);
  });

  it("asks every project again on Refresh, closed tasks too when they are showing", async () => {
    const user = show(answers(), { [FILTERS_KEY]: JSON.stringify({ state: "all" }) });
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(core.projectTasks).toHaveBeenCalledTimes(2));
    expect(core.projectTasks).toHaveBeenCalledWith(alpha.id, true, true);
    expect(core.projectTasks).toHaveBeenCalledWith(beta.id, true, true);
  });
});

describe("filters", () => {
  it("switches state with the tabs, remembers it, and says closed tasks are wanted", async () => {
    const user = show();
    expect(screen.getByRole("tab", { name: "Open" })).toHaveAttribute("aria-selected", "true");
    expect(useTasksStore.getState().closedWanted).toBe(false);

    await user.click(screen.getByRole("tab", { name: "Closed" }));
    expect(titles()).toEqual(["Crash on an empty repository"]);
    expect(row(/Crash/)).toHaveTextContent("Not planned");
    expect(saved()).toMatchObject({ state: "closed" });
    expect(useTasksStore.getState().closedWanted, "the polling asks for them").toBe(true);
    expect(screen.getByRole("note")).toHaveTextContent("the 50 most recently updated");

    await user.click(screen.getByRole("tab", { name: "All" }));
    expect(titles()).toHaveLength(4);
    await user.click(screen.getByRole("tab", { name: "Open" }));
    expect(useTasksStore.getState().closedWanted).toBe(false);
  });

  it("stops wanting closed tasks when the view goes away", () => {
    useTasksStore.setState({ byProject: answers() });
    useProjectsStore.setState({ ui: { [FILTERS_KEY]: JSON.stringify({ state: "closed" }) } });
    const { unmount } = render(<TasksView />);
    expect(useTasksStore.getState().closedWanted).toBe(true);
    unmount();
    expect(useTasksStore.getState().closedWanted).toBe(false);
  });

  it("narrows by project, label, assignee, author and answer, and remembers each", async () => {
    const user = show();
    await user.click(screen.getByRole("button", { name: "Projects: All projects" }));
    await user.click(screen.getByRole("menuitemradio", { name: "alpha" }));
    expect(titles()).toEqual(["Worktrees on a network drive", "Document the daemon"]);
    expect(saved()).toMatchObject({ projects: [alpha.id] });

    await user.selectOptions(screen.getByRole("combobox", { name: "Label" }), "docs");
    expect(titles()).toEqual(["Document the daemon"]);
    expect(saved()).toMatchObject({ projects: [alpha.id], label: "docs" });

    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(titles()).toHaveLength(3);
    expect(saved()).toMatchObject({ projects: [], label: null });

    await user.selectOptions(screen.getByRole("combobox", { name: "Assignee" }), "Me (ada)");
    expect(titles()).toEqual(["Worktrees on a network drive"]);
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Assignee" }),
      "Assigned to no one",
    );
    expect(titles()).toEqual(["Document the daemon", "Dark mode"]);
    await user.selectOptions(screen.getByRole("combobox", { name: "Assignee" }), "Any assignee");

    await user.selectOptions(screen.getByRole("combobox", { name: "Author" }), "ken");
    expect(titles()).toEqual(["Dark mode"]);
    expect(saved()).toMatchObject({ author: "ken" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Author" }), "Any author");

    await user.click(screen.getByRole("checkbox", { name: "Needs an answer" }));
    expect(titles()).toEqual(["Worktrees on a network drive"]);
    expect(saved()).toMatchObject({ needsAnswer: true });
  });

  it("searches titles and numbers, without remembering the search", async () => {
    const user = show();
    const search = screen.getByRole("searchbox", { name: "Search tasks" });
    await user.type(search, "daemon");
    expect(titles()).toEqual(["Document the daemon"]);
    expect(saved(), "a search is of the moment").toBeNull();
    await user.keyboard("{Escape}");
    expect(search).toHaveValue("");
    await user.type(search, "#12");
    expect(titles()).toEqual(["Worktrees on a network drive"]);
  });

  it("brings remembered filters back, without a project that is gone", () => {
    show(answers(), {
      [FILTERS_KEY]: JSON.stringify({ state: "open", projects: ["gone"], author: "ken" }),
    });
    expect(titles()).toEqual(["Dark mode"]);
    expect(screen.getByRole("button", { name: "Projects: All projects" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Author" })).toHaveValue("ken");
  });

  it("still shows a remembered choice nothing in the list matches, so it can be undone", () => {
    show(answers(), { [FILTERS_KEY]: JSON.stringify({ label: "wontfix" }) });
    expect(screen.getByRole("combobox", { name: "Label" })).toHaveValue("wontfix");
    expect(screen.getByRole("status")).toHaveTextContent("No tasks match.");
    // Offered beside the filters and again where the rows would be.
    expect(screen.getAllByRole("button", { name: "Clear filters" })).toHaveLength(2);
  });
});

describe("when there is nothing to show", () => {
  it("says to add a project when there is none", () => {
    useProjectsStore.setState({ projects: [] });
    show({});
    expect(screen.getByRole("status")).toHaveTextContent("Add a project to see its tasks.");
  });

  it("says it is loading until a project has answered", () => {
    show({});
    expect(screen.getByRole("status")).toHaveTextContent("Loading tasks…");
  });

  it("says it is loading while the closed tasks are on their way", () => {
    show(
      { [alpha.id]: tasksOf([]), [beta.id]: tasksOf([]) },
      { [FILTERS_KEY]: JSON.stringify({ state: "closed" }) },
    );
    expect(screen.getByRole("status")).toHaveTextContent("Loading tasks…");
  });

  it("says there are none, in the words of the tab", async () => {
    const user = show({ [alpha.id]: tasksOf([crash], { closed: true }), [beta.id]: tasksOf([]) });
    expect(screen.getByRole("status")).toHaveTextContent("No open tasks.");
    expect(screen.queryByRole("button", { name: "Clear filters" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Closed" }));
    expect(titles()).toEqual(["Crash on an empty repository"]);
  });
});

describe("why a project's tasks are missing", () => {
  const without = (overrides: Partial<ProjectTasks>) => ({
    [alpha.id]: tasksOf([drive]),
    [beta.id]: tasksOf([], overrides),
  });

  it("says once that gh is not installed", () => {
    show({
      [alpha.id]: tasksOf([], { gh: false }),
      [beta.id]: tasksOf([], { gh: false }),
    });
    expect(screen.getAllByRole("note")).toHaveLength(1);
    expect(screen.getByRole("note")).toHaveTextContent("gh is not installed");
  });

  it("says once that nobody is logged in", () => {
    show(without({ loggedOut: true, problem: "gh auth login" }));
    expect(screen.getByRole("note")).toHaveTextContent("Nobody is logged in to the GitHub CLI");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("names a project with no remote, one on another forge, and one with issues off", () => {
    show({
      [alpha.id]: tasksOf([], { repo: null }),
      [beta.id]: tasksOf([], {
        repo: { host: "gitlab.com", owner: "demo", name: "site", kind: "gitlab" },
      }),
    });
    const notes = screen.getAllByRole("note").map((note) => note.textContent);
    expect(notes).toEqual([
      "alpha has no remote on a forge, so there is nowhere to ask about tasks.",
      "beta is on GitLab. Tasks are read for GitHub only.",
    ]);
  });

  it("names a project whose repository has issues switched off, beside the others' rows", () => {
    show(without({ disabled: true }));
    expect(screen.getByRole("note")).toHaveTextContent("beta’s repository has issues switched off");
    expect(titles()).toEqual(["Worktrees on a network drive"]);
  });

  it("shows what gh said and asks that one project again on Retry", async () => {
    const user = show(without({ problem: "HTTP 502: Bad Gateway" }));
    expect(screen.getByRole("alert")).toHaveTextContent("beta: HTTP 502: Bad Gateway");
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(core.projectTasks).toHaveBeenCalledWith(beta.id, true, false));
    expect(core.projectTasks).toHaveBeenCalledTimes(1);
  });

  it("says when what arrived is not all of it", () => {
    show({
      [alpha.id]: tasksOf([drive], { problem: "HTTP 504" }),
      [beta.id]: tasksOf([dark]),
    });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "alpha: not every task could be read. HTTP 504",
    );
    expect(titles()).toHaveLength(2);
  });

  it("says when the source has more open tasks than the list holds, with a way to them", async () => {
    const user = show({ [alpha.id]: tasksOf([drive, daemon], { openTotal: 1039 }) });
    expect(screen.getByRole("note")).toHaveTextContent(
      "Showing the 2 most recently updated of 1039 open in alpha.",
    );
    await user.click(screen.getByRole("button", { name: /See all on GitHub/ }));
    expect(opener.openUrl).toHaveBeenCalledWith("https://github.com/demo/app/issues");
  });
});

describe("one task in full", () => {
  const pane = () => screen.getByRole("region", { name: "Task #12" });

  it("opens beside the list with its description and conversation", async () => {
    core.taskDetail.mockResolvedValue(
      taskDetail(drive, {
        body: "It **fails** on SMB.",
        comments: [
          {
            author: "triage-bot",
            createdAt: "2026-09-29T08:00:00Z",
            body: "Thanks for the report.",
            url: "https://github.com/demo/app/issues/12#issuecomment-1",
            hidden: null,
            bot: true,
            maintainer: false,
          },
          {
            author: "ada",
            createdAt: "2026-09-29T09:00:00Z",
            body: "Which version?",
            url: "https://github.com/demo/app/issues/12#issuecomment-2",
            hidden: null,
            bot: false,
            maintainer: true,
          },
          {
            author: "mallory",
            createdAt: "2026-09-29T10:00:00Z",
            body: "",
            url: "https://github.com/demo/app/issues/12#issuecomment-3",
            hidden: "spam",
            bot: false,
            maintainer: false,
          },
        ],
      }),
    );
    const user = show();
    await user.click(row(/Worktrees on a network drive/));
    expect(row(/Worktrees/)).toHaveAttribute("aria-current", "true");
    expect(core.taskDetail).toHaveBeenCalledWith(alpha.id, "#12", false);

    const region = pane();
    expect(region).toHaveTextContent("alpha · grace opened it");
    expect(within(region).getByRole("heading", { level: 2 })).toHaveTextContent(
      "Worktrees on a network drive #12",
    );
    expect(region).toHaveTextContent("Needs an answer");
    expect(region).toHaveTextContent("Assigned toada");
    expect(region).toHaveTextContent("Closed by#14, when it merges");

    const description = within(region).getByRole("region", { name: "Description" });
    await waitFor(() =>
      expect(within(description).getByText("fails").tagName, "Markdown, rendered").toBe("STRONG"),
    );
    const comments = within(within(region).getByRole("region", { name: "Conversation" }))
      .getAllByRole("listitem")
      .map((item) => item.textContent);
    expect(comments[0]).toContain("triage-botbot");
    expect(comments[0]).toContain("Thanks for the report.");
    expect(comments[1]).toContain("adamaintainer");
    expect(comments[2]).toContain("Hidden on GitHub as spam.");
  });

  it("says when the conversation is longer than what was read", async () => {
    const long = { ...drive, comments: 62 };
    core.taskDetail.mockResolvedValue(
      taskDetail(long, {
        comments: [
          {
            author: "ken",
            createdAt: "2026-09-29T10:00:00Z",
            body: "Same here.",
            url: null,
            hidden: null,
            bot: false,
            maintainer: false,
          },
        ],
      }),
    );
    const user = show({ [alpha.id]: tasksOf([long]) });
    await user.click(row(/Worktrees/));
    expect(await screen.findByText(/The latest 1 of 62 comments\./)).toBeVisible();
    await user.click(screen.getByRole("button", { name: /Read them all on GitHub/ }));
    expect(opener.openUrl).toHaveBeenCalledWith(drive.url);
  });

  it("says so when there is no description and nobody has commented", async () => {
    core.taskDetail.mockResolvedValue(taskDetail(drive, { body: "  " }));
    const user = show();
    await user.click(row(/Worktrees/));
    expect(await screen.findByText("No description.")).toBeVisible();
    expect(screen.getByText("Nobody has commented yet.")).toBeVisible();
  });

  it("says why it could not be read, and reads it again on Retry", async () => {
    core.taskDetail.mockRejectedValueOnce({ code: "gh_failed", message: "HTTP 502" });
    const user = show();
    await user.click(row(/Worktrees/));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not read this task: HTTP 502",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("What Worktrees on a network drive is about.")).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(core.taskDetail).toHaveBeenLastCalledWith(alpha.id, "#12", true);
  });

  it("is read again when the list says the task changed, and not when it did not", async () => {
    const user = show();
    await user.click(row(/Worktrees/));
    await waitFor(() => expect(core.taskDetail).toHaveBeenCalledTimes(1));

    // A poll that brings the same task back as a new object.
    act(() => useTasksStore.setState({ byProject: { ...answers() } }));
    expect(core.taskDetail).toHaveBeenCalledTimes(1);

    const commented = { ...drive, comments: 4, updatedAt: "2026-10-01T10:00:00Z" };
    act(() =>
      useTasksStore.setState({
        byProject: { ...answers(), [alpha.id]: tasksOf([commented, daemon]) },
      }),
    );
    await waitFor(() => expect(core.taskDetail).toHaveBeenCalledTimes(2));
    expect(core.taskDetail).toHaveBeenLastCalledWith(alpha.id, "#12", true);
  });

  it("opens on GitHub and copies its link", async () => {
    const user = show();
    await user.click(row(/Worktrees/));
    await user.click(within(pane()).getByRole("button", { name: /Open on GitHub/ }));
    expect(opener.openUrl).toHaveBeenCalledWith(drive.url);
    await user.click(within(pane()).getByRole("button", { name: "Copy link" }));
    expect(clipboard.writeText).toHaveBeenCalledWith(drive.url);
    expect(await within(pane()).findByRole("button", { name: "Copied" })).toBeVisible();
  });

  it("closes with Escape and with its button, back to the row it came from", async () => {
    const user = show();
    await user.click(row(/Worktrees/));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: "Task #12" })).not.toBeInTheDocument();
    await waitFor(() => expect(row(/Worktrees/)).toHaveFocus());

    await user.click(row(/Document the daemon/));
    await user.click(screen.getByRole("button", { name: "Close details" }));
    expect(useTasksStore.getState().selected).toBeNull();
  });

  it("goes away with a task that is no longer in the list", async () => {
    const user = show();
    await user.click(row(/Worktrees/));
    act(() => useTasksStore.setState({ byProject: { [alpha.id]: tasksOf([daemon]) } }));
    expect(screen.queryByRole("region", { name: "Task #12" })).not.toBeInTheDocument();
  });
});

describe("handing a task to an agent", () => {
  const pane = () => screen.getByRole("region", { name: "Task #12" });

  it("opens the composer with the message the core wrote, and starts nothing", async () => {
    const user = show();
    await user.click(row(/Worktrees/));
    await user.click(within(pane()).getByRole("button", { name: "Delegate" }));

    await waitFor(() => expect(useProjectsStore.getState().composingProjectId).toBe(alpha.id));
    expect(core.taskPrompt).toHaveBeenCalledWith(alpha.id, "#12");
    const projects = useProjectsStore.getState();
    expect(projects.composingPrompt).toBe("Work on this GitHub issue: #12");
    expect(projects.composingTask).toEqual(linkTo(12));
    expect(projects.composingBranch).toBeNull();
    expect(projects.tasksOpen, "the composer takes the panel").toBe(false);
    expect(core.ptySpawn, "the composer is where it starts").not.toHaveBeenCalled();
  });

  it("says why when the task could not be got ready, and stays where it is", async () => {
    core.taskPrompt.mockRejectedValue({ code: "gh_failed", message: "HTTP 502" });
    const user = show();
    await user.click(row(/Worktrees/));
    await user.click(within(pane()).getByRole("button", { name: "Delegate" }));
    expect(await within(pane()).findByRole("alert")).toHaveTextContent("HTTP 502");
    expect(useProjectsStore.getState().composingProjectId).toBeNull();
    expect(useProjectsStore.getState().tasksOpen).toBe(true);
    await user.click(within(pane()).getByRole("button", { name: "Dismiss" }));
    expect(within(pane()).queryByRole("alert")).not.toBeInTheDocument();
  });

  it("is not offered for a closed task", async () => {
    const user = show(answers(), { [FILTERS_KEY]: JSON.stringify({ state: "closed" }) });
    await user.click(row(/Crash on an empty repository/));
    const closed = screen.getByRole("region", { name: "Task #4" });
    expect(within(closed).queryByRole("button", { name: /Delegate/ })).not.toBeInTheDocument();
    expect(within(closed).getByRole("button", { name: /Open on GitHub/ })).toBeVisible();
  });

  describe("when a workspace was started from it", () => {
    const one = { ...worktree("alpha", "12-network-drive"), tasks: [linkTo(12)] };
    const two = { ...worktree("alpha", "12-network-drive-2"), tasks: [linkTo(12)] };
    const withWorkspaces = (...made: (typeof one)[]) =>
      useProjectsStore.setState({
        projects: [{ ...alpha, workspaces: [...alpha.workspaces, ...made] }, beta],
      });

    it("names the workspace under the row and goes there", async () => {
      withWorkspaces(one);
      const user = show();
      await user.click(screen.getByRole("button", { name: "12-network-drive" }));
      expect(useProjectsStore.getState().selectedWorkspaceId).toBe(one.id);
      expect(useProjectsStore.getState().tasksOpen, "the view gives way").toBe(false);
    });

    it("offers Go to workspace, and Delegate again for another attempt", async () => {
      withWorkspaces(one);
      const user = show();
      await user.click(row(/Worktrees/));
      expect(within(pane()).queryByRole("button", { name: "Delegate" })).not.toBeInTheDocument();
      await user.click(within(pane()).getByRole("button", { name: "Delegate again" }));
      await waitFor(() => expect(core.taskPrompt).toHaveBeenCalledWith(alpha.id, "#12"));
      await waitFor(() => expect(useProjectsStore.getState().composingProjectId).toBe(alpha.id));

      useProjectsStore.setState({ composingProjectId: null, tasksOpen: true });
      await user.click(within(pane()).getByRole("button", { name: "Go to workspace" }));
      expect(useProjectsStore.getState().selectedWorkspaceId).toBe(one.id);
    });

    it("asks which, when there is more than one", async () => {
      withWorkspaces(one, two);
      const user = show();
      await user.click(row(/Worktrees/));
      await user.click(within(pane()).getByRole("button", { name: /Go to workspace/ }));
      await user.click(screen.getByRole("menuitem", { name: "12-network-drive-2" }));
      expect(useProjectsStore.getState().selectedWorkspaceId).toBe(two.id);
    });
  });
});
