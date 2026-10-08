import { useMemo } from "react";
import { create } from "zustand";
import {
  errorMessage,
  ipc,
  isIpcError,
  type AddedProject,
  type NewWorkspace,
  type PreparedBranch,
  type TaskRef,
  type Project,
  type RemoteRepository,
  type Workspace,
  type SessionInfo,
} from "@/lib/ipc";

/** What the clone dialog knows of the `gh` account's repositories. */
export type RemoteRepositories = {
  status: "idle" | "loading" | "ready" | "failed";
  /** The ones listed, most recently pushed first, up to the core's cap. */
  list: RemoteRepository[];
  /** How many the account has, when the forge said; more than `list` holds past the cap. */
  total: number | null;
  /** Why the reading stopped short, or why there is no list at all. */
  problem: string | null;
  notInstalled: boolean;
  loggedOut: boolean;
};

const noRepositories: RemoteRepositories = {
  status: "idle",
  list: [],
  total: null,
  problem: null,
  notInstalled: false,
  loggedOut: false,
};

const KEYS = {
  selected: "sidebar.selectedWorkspace",
  collapsed: "sidebar.collapsedProjects",
  lastParent: "projects.lastParentDir",
} as const;

/** Outcome of trying to add a folder that may not be a repository yet. */
export type OpenResult = { status: "added" } | { status: "needs-git" } | { status: "failed" };

interface ProjectsState {
  projects: Project[];
  loaded: boolean;
  selectedWorkspaceId: string | null;
  /** The project a new workspace is being composed for; takes over the center panel. */
  composingProjectId: string | null;
  /** Composing to run in a workspace that already exists — `local`, whose checkout is the repo
   *  itself, so there is no worktree or branch to create. */
  composingWorkspaceId: string | null;
  /** A handoff packet to start the composer's message with, when composing to run in a
   *  workspace that already has history. Cleared with the composer. */
  composingPrompt: string | null;
  /** The workflow open in the center panel, which it takes over like the composer. Selecting
   *  a workspace or composing closes it; the selected workspace is kept to go back to. */
  workflowId: string | null;
  /** The Usage view is open in the center panel, which it takes over like a workflow. */
  usageOpen: boolean;
  /** The Pull requests view is open in the center panel, which it takes over the same way. */
  pullRequestsOpen: boolean;
  /** The Tasks view is open in the center panel, which it takes over the same way. */
  tasksOpen: boolean;
  /** The branch the composer should open rather than start from: a pull request's, got ready
   *  by the core. Cleared with the composer. */
  composingBranch: PreparedBranch | null;
  /** The task the composer's workspace is being started from: it is recorded against the
   *  workspace, and names it. Cleared with the composer. */
  composingTask: TaskRef | null;
  /** Raw persisted UI state, for features that remember small things (see `remember`). */
  ui: Record<string, string>;
  /** Projects are expanded unless listed here, so new ones start open. */
  collapsed: string[];
  /** Where the last project was created; the next one is offered the same parent. */
  lastParentDir: string | null;
  /** In-flight clone requests; the Rust command owns the actual operation. */
  pendingClones: { id: number; name: string }[];
  /** The repositories offered by the clone dialog; read when it opens, kept while the app runs. */
  repositories: RemoteRepositories;
  error: string | null;
  notice: string | null;

  load: () => Promise<void>;
  /** Re-read projects from the core (branches change behind our back). */
  refresh: () => Promise<void>;
  openFolder: (path: string, initGit?: boolean) => Promise<OpenResult>;
  /** `select` is checked on completion, so closing the dialog keeps the current workspace.
   *  `upstream` is a fork's parent, which the clone gets as a second remote. */
  cloneProject: (
    repository: string,
    name: string,
    parent: string,
    select?: () => boolean,
    upstream?: string | null,
  ) => Promise<boolean>;
  /** Read the account's repositories for the clone dialog; again only with `refresh`. */
  loadRepositories: (refresh?: boolean) => Promise<void>;
  /** Ask the forge for repositories named like `text`. Throws with `gh`'s reason. */
  searchRepositories: (text: string) => Promise<RemoteRepository[]>;
  createProject: (name: string, parent: string) => Promise<boolean>;
  /** Take a project off the list. Nothing on disk changes; with `keepHistory` its workspaces
   *  and conversations come back when the same folder is opened again. */
  remove: (id: string, keepHistory: boolean) => Promise<boolean>;
  move: (id: string, by: -1 | 1) => Promise<void>;
  select: (workspaceId: string | null) => void;
  /** `branch` opens the composer on a branch that exists — a pull request's — instead of on
   *  a new one, and `prompt` is the message it starts with: a note about the pull request. */
  compose: (
    projectId: string | null,
    branch?: PreparedBranch,
    prompt?: string,
    task?: TaskRef,
  ) => void;
  /** Show a workflow in the center panel; `null` closes it. */
  openWorkflow: (workflowId: string | null) => void;
  /** Show the Usage view in the center panel, or close it. */
  openUsage: (open: boolean) => void;
  /** Show the Pull requests view in the center panel, or close it. */
  openPullRequests: (open: boolean) => void;
  /** Show the Tasks view in the center panel, or close it. */
  openTasks: (open: boolean) => void;
  /** Compose a run inside a workspace that already exists, rather than a new one. */
  composeIn: (workspace: Workspace, prompt?: string) => void;
  /**
   * Create a worktree workspace and start its harness. Resolves to the new session, or to an
   * error message — returned rather than stored, because the composer shows it inline.
   */
  createWorkspace: (request: NewWorkspace) => Promise<SessionInfo | { error: string }>;
  /** Resolves to `"dirty"` when the workspace has uncommitted work and `force` was not given. */
  deleteWorkspace: (
    workspaceId: string,
    force?: boolean,
  ) => Promise<"deleted" | "dirty" | "failed">;
  /** Put a workspace away: folder removed, branch and history kept. */
  archiveWorkspace: (
    workspaceId: string,
    force?: boolean,
  ) => Promise<"archived" | "dirty" | "failed">;
  /** Bring back an archived or vanished workspace. */
  restoreWorkspace: (workspaceId: string) => Promise<boolean>;
  renameWorkspace: (workspaceId: string, name: string) => Promise<boolean>;
  /** Make workspaces of worktrees git already has. Resolves to what was imported, or `null`
   *  with the reason in `error`. */
  importWorktrees: (projectId: string, paths: string[]) => Promise<Workspace[] | null>;
  /** Stop showing a workspace. Its folder and branch stay; its history too unless told not to. */
  forgetWorkspace: (workspaceId: string, keepHistory: boolean) => Promise<boolean>;
  remember: (key: string, value: unknown) => void;
  toggleCollapsed: (projectId: string) => void;
  dismiss: () => void;
}

const save = (key: string, value: unknown) =>
  void ipc.uiStateSave(key, JSON.stringify(value)).catch(console.error);

function parse<T>(raw: string | undefined, fallback: T): T {
  if (raw === undefined) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

const workspaceIds = (projects: Project[]) =>
  new Set(projects.flatMap((project) => project.workspaces.map((workspace) => workspace.id)));

export const useProjectsStore = create<ProjectsState>((set, get) => {
  let nextCloneId = 0;
  /** Add and expand a project; background completions leave the selection alone. */
  const adopt = (added: AddedProject, select = true) => {
    const { project } = added;
    const local = project.workspaces[0]?.id ?? null;
    set((state) => ({
      projects: state.projects.some((p) => p.id === project.id)
        ? state.projects.map((p) => (p.id === project.id ? project : p))
        : [...state.projects, project],
      collapsed: state.collapsed.filter((id) => id !== project.id),
      selectedWorkspaceId: select ? local : state.selectedWorkspaceId,
      error: null,
      notice: added.alreadyKnown
        ? `${project.name} was already in your projects.`
        : added.revived
          ? `${project.name} is back, with its workspaces and their conversations.`
          : added.openedRootInstead
            ? `That folder is inside a repository, so its root was added: ${project.rootPath}`
            : null,
    }));
    if (select) save(KEYS.selected, local);
    save(KEYS.collapsed, get().collapsed);
  };

  return {
    projects: [],
    loaded: false,
    selectedWorkspaceId: null,
    composingProjectId: null,
    composingWorkspaceId: null,
    composingPrompt: null,
    composingBranch: null,
    composingTask: null,
    workflowId: null,
    usageOpen: false,
    pullRequestsOpen: false,
    tasksOpen: false,
    ui: {},
    collapsed: [],
    lastParentDir: null,
    repositories: noRepositories,
    pendingClones: [],
    error: null,
    notice: null,

    async load() {
      try {
        const [ui, projects] = await Promise.all([ipc.uiStateLoad(), ipc.projectsList()]);
        const selected = parse<string | null>(ui[KEYS.selected], null);
        set({
          ui,
          projects,
          loaded: true,
          // The selection may point at something removed since it was saved.
          selectedWorkspaceId: selected && workspaceIds(projects).has(selected) ? selected : null,
          collapsed: parse<string[]>(ui[KEYS.collapsed], []),
          lastParentDir: parse<string | null>(ui[KEYS.lastParent], null),
        });
      } catch (error) {
        set({ loaded: true, error: errorMessage(error) });
      }
    },

    async refresh() {
      try {
        set({ projects: await ipc.projectsList() });
      } catch (error) {
        console.error(error);
      }
    },

    async openFolder(path, initGit = false) {
      try {
        adopt(await ipc.projectOpen(path, initGit));
        return { status: "added" };
      } catch (error) {
        if (isIpcError(error) && error.code === "not_a_git_repo") return { status: "needs-git" };
        set({ error: errorMessage(error) });
        return { status: "failed" };
      }
    },

    async cloneProject(repository, name, parent, select = () => true, upstream = null) {
      const request = { id: nextCloneId++, name: name.trim() };
      set((state) => ({ pendingClones: [...state.pendingClones, request] }));
      try {
        const added = await ipc.projectClone(repository, name, parent, upstream);
        const foreground = select();
        adopt(added, foreground);
        set({
          lastParentDir: parent,
          ...(!foreground && { notice: `${added.project.name} cloned.` }),
        });
        save(KEYS.lastParent, parent);
        return true;
      } catch (error) {
        set({ error: `${request.name}: ${errorMessage(error)}` });
        return false;
      } finally {
        set((state) => ({
          pendingClones: state.pendingClones.filter((pending) => pending.id !== request.id),
        }));
      }
    },

    async loadRepositories(refresh = false) {
      const { status } = get().repositories;
      if (!refresh && status !== "idle" && status !== "failed") return;
      set({ repositories: { ...noRepositories, status: "loading" } });
      try {
        const found = await ipc.forgeRepositories();
        set({
          repositories: {
            status: "ready",
            list: found.repositories,
            total: found.total,
            problem: found.problem,
            notInstalled: false,
            loggedOut: found.loggedOut,
          },
        });
      } catch (error) {
        set({
          repositories: {
            ...noRepositories,
            status: "failed",
            problem: errorMessage(error),
            notInstalled: isIpcError(error) && error.code === "gh_not_installed",
          },
        });
      }
    },

    async searchRepositories(text) {
      return ipc.forgeSearchRepositories(text);
    },

    async createProject(name, parent) {
      try {
        adopt(await ipc.projectCreate(name, parent));
        set({ lastParentDir: parent });
        save(KEYS.lastParent, parent);
        return true;
      } catch (error) {
        set({ error: errorMessage(error) });
        return false;
      }
    },

    async remove(id, keepHistory) {
      const project = get().projects.find((p) => p.id === id);
      if (!project) return false;
      try {
        await ipc.projectRemove(id, keepHistory);
      } catch (error) {
        set({ error: errorMessage(error) });
        return false;
      }
      const gone = new Set(project.workspaces.map((workspace) => workspace.id));
      set((state) => ({
        projects: state.projects.filter((p) => p.id !== id),
        collapsed: state.collapsed.filter((c) => c !== id),
        selectedWorkspaceId:
          state.selectedWorkspaceId && gone.has(state.selectedWorkspaceId)
            ? null
            : state.selectedWorkspaceId,
        composingProjectId: state.composingProjectId === id ? null : state.composingProjectId,
      }));
      save(KEYS.selected, get().selectedWorkspaceId);
      save(KEYS.collapsed, get().collapsed);
      return true;
    },

    async move(id, by) {
      const projects = [...get().projects];
      const from = projects.findIndex((p) => p.id === id);
      const to = from + by;
      if (from < 0 || to < 0 || to >= projects.length) return;
      [projects[from], projects[to]] = [projects[to]!, projects[from]!];
      set({ projects });
      try {
        await ipc.projectsReorder(projects.map((p) => p.id));
      } catch (error) {
        set({ error: errorMessage(error) });
        await get().refresh();
      }
    },

    select(workspaceId) {
      if (get().selectedWorkspaceId === workspaceId) {
        return set({
          composingProjectId: null,
          composingWorkspaceId: null,
          composingPrompt: null,
          composingBranch: null,
          composingTask: null,
          workflowId: null,
          usageOpen: false,
          pullRequestsOpen: false,
          tasksOpen: false,
        });
      }
      set({
        selectedWorkspaceId: workspaceId,
        composingProjectId: null,
        composingWorkspaceId: null,
        composingPrompt: null,
        composingBranch: null,
        composingTask: null,
        workflowId: null,
        usageOpen: false,
        pullRequestsOpen: false,
        tasksOpen: false,
      });
      save(KEYS.selected, workspaceId);
    },

    compose(projectId, branch, prompt, task) {
      set((state) => ({
        composingProjectId: projectId,
        composingWorkspaceId: null,
        composingPrompt: (projectId && prompt) || null,
        composingBranch: (projectId && branch) || null,
        composingTask: (projectId && task) || null,
        workflowId: null,
        usageOpen: false,
        pullRequestsOpen: false,
        tasksOpen: false,
        // Composing inside a collapsed project would hide where the workspace will appear.
        collapsed: state.collapsed.filter((id) => id !== projectId),
      }));
    },

    openWorkflow(workflowId) {
      set({
        workflowId,
        usageOpen: false,
        pullRequestsOpen: false,
        tasksOpen: false,
        composingProjectId: null,
        composingWorkspaceId: null,
        composingPrompt: null,
        composingBranch: null,
        composingTask: null,
      });
    },

    openUsage(open) {
      set({
        usageOpen: open,
        pullRequestsOpen: false,
        tasksOpen: false,
        workflowId: null,
        composingProjectId: null,
        composingWorkspaceId: null,
        composingPrompt: null,
        composingBranch: null,
        composingTask: null,
      });
    },

    openPullRequests(open) {
      set({
        pullRequestsOpen: open,
        tasksOpen: false,
        usageOpen: false,
        workflowId: null,
        composingProjectId: null,
        composingWorkspaceId: null,
        composingPrompt: null,
        composingBranch: null,
        composingTask: null,
      });
    },

    openTasks(open) {
      set({
        tasksOpen: open,
        pullRequestsOpen: false,
        usageOpen: false,
        workflowId: null,
        composingProjectId: null,
        composingWorkspaceId: null,
        composingPrompt: null,
        composingBranch: null,
        composingTask: null,
      });
    },

    composeIn(workspace, prompt) {
      set({
        selectedWorkspaceId: workspace.id,
        composingProjectId: null,
        composingWorkspaceId: workspace.id,
        composingPrompt: prompt ?? null,
        composingBranch: null,
        composingTask: null,
        workflowId: null,
        usageOpen: false,
        pullRequestsOpen: false,
        tasksOpen: false,
      });
      save(KEYS.selected, workspace.id);
    },

    async createWorkspace(request) {
      try {
        const { workspace, session } = await ipc.workspaceCreate(request);
        set((state) => ({
          projects: state.projects.map((project) =>
            project.id === workspace.projectId
              ? { ...project, workspaces: [...project.workspaces, workspace] }
              : project,
          ),
          selectedWorkspaceId: workspace.id,
          composingProjectId: null,
        }));
        save(KEYS.selected, workspace.id);
        return session;
      } catch (error) {
        await get().refresh();
        return { error: errorMessage(error) };
      }
    },

    async deleteWorkspace(workspaceId, force = false) {
      try {
        await ipc.workspaceDelete(workspaceId, force);
      } catch (error) {
        if (isIpcError(error) && error.code === "worktree_dirty") return "dirty";
        set({ error: errorMessage(error) });
        return "failed";
      }
      set((state) => ({
        projects: state.projects.map((project) => ({
          ...project,
          workspaces: project.workspaces.filter((workspace) => workspace.id !== workspaceId),
        })),
        selectedWorkspaceId:
          state.selectedWorkspaceId === workspaceId ? null : state.selectedWorkspaceId,
      }));
      save(KEYS.selected, get().selectedWorkspaceId);
      return "deleted";
    },

    async archiveWorkspace(workspaceId, force = false) {
      try {
        await ipc.workspaceArchive(workspaceId, force);
      } catch (error) {
        if (isIpcError(error) && error.code === "worktree_dirty") return "dirty";
        set({ error: errorMessage(error) });
        return "failed";
      }
      set((state) => ({
        projects: patchWorkspace(state.projects, workspaceId, (workspace) => ({
          ...workspace,
          archived: true,
          missing: false,
          head: null,
        })),
        selectedWorkspaceId:
          state.selectedWorkspaceId === workspaceId ? null : state.selectedWorkspaceId,
      }));
      save(KEYS.selected, get().selectedWorkspaceId);
      return "archived";
    },

    async restoreWorkspace(workspaceId) {
      try {
        const restored = await ipc.workspaceRestore(workspaceId);
        set((state) => ({
          projects: patchWorkspace(state.projects, workspaceId, () => restored),
          error: null,
        }));
        return true;
      } catch (error) {
        await get().refresh();
        set({ error: errorMessage(error) });
        return false;
      }
    },

    async renameWorkspace(workspaceId, name) {
      try {
        const renamed = await ipc.workspaceRename(workspaceId, name);
        set((state) => ({
          projects: patchWorkspace(state.projects, workspaceId, () => renamed),
          error: null,
        }));
        return true;
      } catch (error) {
        set({ error: errorMessage(error) });
        return false;
      }
    },

    async importWorktrees(projectId, paths) {
      try {
        const imported = await ipc.workspacesImport(projectId, paths);
        set((state) => ({
          projects: state.projects.map((project) =>
            project.id === projectId
              ? { ...project, workspaces: [...project.workspaces, ...imported] }
              : project,
          ),
          error: null,
        }));
        return imported;
      } catch (error) {
        set({ error: errorMessage(error) });
        return null;
      }
    },

    async forgetWorkspace(workspaceId, keepHistory) {
      try {
        await ipc.workspaceForget(workspaceId, keepHistory);
      } catch (error) {
        set({ error: errorMessage(error) });
        return false;
      }
      set((state) => ({
        projects: state.projects.map((project) => ({
          ...project,
          workspaces: project.workspaces.filter((workspace) => workspace.id !== workspaceId),
        })),
        selectedWorkspaceId:
          state.selectedWorkspaceId === workspaceId ? null : state.selectedWorkspaceId,
      }));
      save(KEYS.selected, get().selectedWorkspaceId);
      return true;
    },

    remember(key, value) {
      set((state) => ({ ui: { ...state.ui, [key]: JSON.stringify(value) } }));
      save(key, value);
    },

    toggleCollapsed(projectId) {
      set((state) => ({
        collapsed: state.collapsed.includes(projectId)
          ? state.collapsed.filter((id) => id !== projectId)
          : [...state.collapsed, projectId],
      }));
      save(KEYS.collapsed, get().collapsed);
    },

    dismiss: () => set({ error: null, notice: null }),
  };
});

function findWorkspace(projects: Project[], workspaceId: string | null) {
  for (const project of projects) {
    const workspace = project.workspaces.find((w) => w.id === workspaceId);
    if (workspace) return { project, workspace };
  }
  return null;
}

/** The selected workspace together with the project it belongs to. */
export function useSelectedWorkspace() {
  const projects = useProjectsStore((state) => state.projects);
  const selected = useProjectsStore((state) => state.selectedWorkspaceId);
  return useMemo(() => findWorkspace(projects, selected), [projects, selected]);
}

function patchWorkspace(
  projects: Project[],
  workspaceId: string,
  patch: (workspace: Workspace) => Workspace,
): Project[] {
  return projects.map((project) => ({
    ...project,
    workspaces: project.workspaces.map((w) => (w.id === workspaceId ? patch(w) : w)),
  }));
}

/** A remembered value from persisted UI state (see `remember`), or `fallback`. */
export function recall<T>(ui: Record<string, string>, key: string, fallback: T): T {
  return parse(ui[key], fallback);
}
