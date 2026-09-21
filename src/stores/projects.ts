import { useMemo } from "react";
import { create } from "zustand";
import {
  errorMessage,
  ipc,
  isIpcError,
  type AddedProject,
  type NewWorkspace,
  type Project,
  type Workspace,
  type SessionInfo,
} from "@/lib/ipc";

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
  /** Raw persisted UI state, for features that remember small things (see `remember`). */
  ui: Record<string, string>;
  /** Projects are expanded unless listed here, so new ones start open. */
  collapsed: string[];
  /** Where the last project was created; the next one is offered the same parent. */
  lastParentDir: string | null;
  error: string | null;
  notice: string | null;

  load: () => Promise<void>;
  /** Re-read projects from the core (branches change behind our back). */
  refresh: () => Promise<void>;
  openFolder: (path: string, initGit?: boolean) => Promise<OpenResult>;
  createProject: (name: string, parent: string) => Promise<boolean>;
  remove: (id: string) => Promise<void>;
  move: (id: string, by: -1 | 1) => Promise<void>;
  select: (workspaceId: string | null) => void;
  compose: (projectId: string | null) => void;
  /** Compose a run inside a workspace that already exists, rather than a new one. */
  composeIn: (workspace: Workspace) => void;
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
  /** Put a just-added project on screen: in the list, expanded, its `local` selected. */
  const adopt = (added: AddedProject) => {
    const { project } = added;
    const local = project.workspaces[0]?.id ?? null;
    set((state) => ({
      projects: state.projects.some((p) => p.id === project.id)
        ? state.projects.map((p) => (p.id === project.id ? project : p))
        : [...state.projects, project],
      collapsed: state.collapsed.filter((id) => id !== project.id),
      selectedWorkspaceId: local,
      error: null,
      notice: added.alreadyKnown
        ? `${project.name} was already in your projects.`
        : added.openedRootInstead
          ? `That folder is inside a repository, so its root was added: ${project.rootPath}`
          : null,
    }));
    save(KEYS.selected, local);
    save(KEYS.collapsed, get().collapsed);
  };

  return {
    projects: [],
    loaded: false,
    selectedWorkspaceId: null,
    composingProjectId: null,
    composingWorkspaceId: null,
    ui: {},
    collapsed: [],
    lastParentDir: null,
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

    async remove(id) {
      const project = get().projects.find((p) => p.id === id);
      if (!project) return;
      try {
        await ipc.projectRemove(id);
      } catch (error) {
        return set({ error: errorMessage(error) });
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
        return set({ composingProjectId: null, composingWorkspaceId: null });
      }
      set({
        selectedWorkspaceId: workspaceId,
        composingProjectId: null,
        composingWorkspaceId: null,
      });
      save(KEYS.selected, workspaceId);
    },

    compose(projectId) {
      set((state) => ({
        composingProjectId: projectId,
        composingWorkspaceId: null,
        // Composing inside a collapsed project would hide where the workspace will appear.
        collapsed: state.collapsed.filter((id) => id !== projectId),
      }));
    },

    composeIn(workspace) {
      set({
        selectedWorkspaceId: workspace.id,
        composingProjectId: null,
        composingWorkspaceId: workspace.id,
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
