import { beforeEach, describe, expect, it, vi } from "vitest";
import { added, project } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  uiStateLoad: vi.fn(),
  uiStateSave: vi.fn(),
  projectsList: vi.fn(),
  projectOpen: vi.fn(),
  projectCreate: vi.fn(),
  projectRemove: vi.fn(),
  projectsReorder: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { useProjectsStore } from "./projects";

const store = () => useProjectsStore.getState();
const saved = (key: string) =>
  core.uiStateSave.mock.calls.filter(([k]) => k === key).map(([, value]) => JSON.parse(value));

describe("projects store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    for (const fn of [core.uiStateSave, core.projectRemove, core.projectsReorder]) {
      fn.mockResolvedValue(undefined);
    }
    useProjectsStore.setState({
      projects: [],
      loaded: false,
      selectedWorkspaceId: null,
      collapsed: [],
      lastParentDir: null,
      error: null,
      notice: null,
    });
  });

  it("restores selection, collapsed projects and the last parent folder", async () => {
    core.projectsList.mockResolvedValue([project("a"), project("b")]);
    core.uiStateLoad.mockResolvedValue({
      "sidebar.selectedWorkspace": '"w-b"',
      "sidebar.collapsedProjects": '["p-a"]',
      "projects.lastParentDir": '"/code"',
    });
    await store().load();
    expect(store()).toMatchObject({
      loaded: true,
      selectedWorkspaceId: "w-b",
      collapsed: ["p-a"],
      lastParentDir: "/code",
    });
  });

  it("drops a saved selection that no longer exists, and survives corrupt state", async () => {
    core.projectsList.mockResolvedValue([project("a")]);
    core.uiStateLoad.mockResolvedValue({
      "sidebar.selectedWorkspace": '"w-gone"',
      "sidebar.collapsedProjects": "not json",
    });
    await store().load();
    expect(store().selectedWorkspaceId).toBeNull();
    expect(store().collapsed).toEqual([]);
  });

  it("reports a failed load instead of hanging on a spinner", async () => {
    core.uiStateLoad.mockResolvedValue({});
    core.projectsList.mockRejectedValue({
      code: "git_not_installed",
      message: "git is not installed",
    });
    await store().load();
    expect(store()).toMatchObject({ loaded: true, error: "git is not installed" });
  });

  it("adds an opened project expanded, with its local workspace selected and saved", async () => {
    useProjectsStore.setState({ collapsed: ["p-app"] });
    core.projectOpen.mockResolvedValue(added("app"));
    expect(await store().openFolder("/code/app")).toEqual({ status: "added" });
    expect(store().projects.map((p) => p.name)).toEqual(["app"]);
    expect(store().selectedWorkspaceId).toBe("w-app");
    expect(store().collapsed).toEqual([]);
    expect(saved("sidebar.selectedWorkspace")).toEqual(["w-app"]);
  });

  it("asks about git instead of failing when the folder is not a repository", async () => {
    core.projectOpen.mockRejectedValue({ code: "not_a_git_repo", message: "nope" });
    expect(await store().openFolder("/tmp/plain")).toEqual({ status: "needs-git" });
    expect(store().error).toBeNull();

    core.projectOpen.mockResolvedValue(added("plain"));
    expect(await store().openFolder("/tmp/plain", true)).toEqual({ status: "added" });
    expect(core.projectOpen).toHaveBeenLastCalledWith("/tmp/plain", true);
  });

  it("explains when a project was already known or its root was added instead", async () => {
    useProjectsStore.setState({ projects: [project("app")] });
    core.projectOpen.mockResolvedValue(added("app", { alreadyKnown: true }));
    await store().openFolder("/code/app");
    expect(store().projects).toHaveLength(1);
    expect(store().notice).toMatch(/already in your projects/);

    core.projectOpen.mockResolvedValue(added("mono", { openedRootInstead: true }));
    await store().openFolder("/code/mono/packages/web");
    expect(store().notice).toMatch(/its root was added/);

    core.projectOpen.mockResolvedValue(added("back", { revived: true }));
    await store().openFolder("/code/back");
    expect(store().notice).toMatch(/back is back, with its workspaces and their conversations/);
  });

  it("remembers where a project was created, and keeps the error when it fails", async () => {
    core.projectCreate.mockResolvedValue(added("fresh"));
    expect(await store().createProject("fresh", "/code")).toBe(true);
    expect(store().lastParentDir).toBe("/code");
    expect(saved("projects.lastParentDir")).toEqual(["/code"]);

    core.projectCreate.mockRejectedValue({
      code: "already_exists",
      message: "/code/fresh already exists.",
    });
    expect(await store().createProject("fresh", "/code")).toBe(false);
    expect(store().error).toBe("/code/fresh already exists.");
  });

  it("clears the selection when the selected project is removed", async () => {
    useProjectsStore.setState({
      projects: [project("a"), project("b")],
      selectedWorkspaceId: "w-a",
      collapsed: ["p-a"],
    });
    await store().remove("p-a", true);
    expect(core.projectRemove).toHaveBeenCalledWith("p-a", true);
    expect(store().projects.map((p) => p.name)).toEqual(["b"]);
    expect(store().selectedWorkspaceId).toBeNull();
    expect(store().collapsed).toEqual([]);
  });

  it("moves projects and tells the core the whole new order", async () => {
    useProjectsStore.setState({ projects: [project("a"), project("b"), project("c")] });
    await store().move("p-c", -1);
    expect(store().projects.map((p) => p.name)).toEqual(["a", "c", "b"]);
    expect(core.projectsReorder).toHaveBeenLastCalledWith(["p-a", "p-c", "p-b"]);

    await store().move("p-a", -1); // already first
    expect(core.projectsReorder).toHaveBeenCalledTimes(1);
  });

  it("persists collapsing, and saves a selection only when it changes", () => {
    store().toggleCollapsed("p-a");
    store().toggleCollapsed("p-b");
    store().toggleCollapsed("p-a");
    expect(saved("sidebar.collapsedProjects").at(-1)).toEqual(["p-b"]);

    store().select("w-a");
    store().select("w-a");
    expect(saved("sidebar.selectedWorkspace")).toEqual(["w-a"]);
  });
});
