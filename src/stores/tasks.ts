import { create } from "zustand";
import { errorMessage, ipc, type ProjectTasks, type Task, type TaskDetail } from "@/lib/ipc";
import { useProjectsStore } from "./projects";

const NO_TASKS: ProjectTasks = {
  gh: false,
  tasks: [],
  problem: null,
  loggedOut: false,
  repo: null,
  viewer: null,
  openTotal: null,
  disabled: false,
  closed: false,
};

/** The task a detail is about. */
export interface Target {
  /** The row's key: see `rowKey`. */
  key: string;
  projectId: string;
  task: Task;
}

/** One task read in full, or the reading of it. */
export interface DetailState {
  /** The last one read. Kept while a newer one is on its way, so the pane does not blink. */
  detail: TaskDetail | null;
  loading: boolean;
  error: string | null;
  /** What the list said about the task when this was asked for: see `loadDetail`. */
  stamp: string;
}

/** What the list knows about a task that its detail would also change with. */
export const detailStamp = (task: Task): string =>
  [task.state, task.updatedAt, task.comments].join("|");

interface TasksStore {
  /**
   * Tasks per project. Keyed by project so that reloading one leaves every other project's
   * entry identical — a selector reading one must not see a new object because a different
   * project was refreshed.
   */
  byProject: Record<string, ProjectTasks>;
  /** The view is showing closed tasks, so they are asked for along with the open ones. */
  closedWanted: boolean;
  /** The row open in the detail pane. */
  selected: string | null;
  /** Tasks read in full, by row key. */
  details: Record<string, DetailState>;
  /** The row being got ready for an agent. One at a time. */
  busy: string | null;
  /** Why the last thing asked of a task could not be done. */
  error: string | null;

  /** Load one project's tasks. Never throws: a source that will not answer says so. */
  loadProject: (projectId: string, refresh?: boolean, closed?: boolean) => Promise<void>;
  wantClosed: (wanted: boolean) => void;
  select: (key: string | null) => void;
  /**
   * Read a task in full, unless that was already done for what the list says now. The list is
   * asked every minute while the view is open, so a detail follows it: when the row changes,
   * the detail is read again. `force` reads it regardless — Retry.
   */
  loadDetail: (target: Target, force?: boolean) => Promise<void>;
  /**
   * Hand a task to an agent: have the core write the first message from the task as it stands
   * now, and open the composer on it. The composer is where it is read, changed and started;
   * nothing starts here.
   */
  delegate: (target: Target) => Promise<void>;
  dismiss: () => void;
}

export const useTasksStore = create<TasksStore>((set, get) => {
  /** The latest request per project and per row: an answer that is no longer the latest is dropped. */
  const projectRequests = new Map<string, number>();
  const detailRequests = new Map<string, number>();
  const patch = (key: string, next: Partial<DetailState>) =>
    set((s) => {
      const before = s.details[key];
      if (!before) return s;
      return { details: { ...s.details, [key]: { ...before, ...next } } };
    });

  return {
    byProject: {},
    closedWanted: false,
    selected: null,
    details: {},
    busy: null,
    error: null,

    async delegate(target) {
      if (get().busy) return;
      set({ busy: target.key, error: null });
      try {
        const { prompt, task } = await ipc.taskPrompt(target.projectId, target.task.key);
        useProjectsStore.getState().compose(target.projectId, undefined, prompt, task);
      } catch (error) {
        set({ error: errorMessage(error) });
      } finally {
        set({ busy: null });
      }
    },

    dismiss() {
      set({ error: null });
    },

    async loadProject(projectId, refresh = false, closed = false) {
      const request = (projectRequests.get(projectId) ?? 0) + 1;
      projectRequests.set(projectId, request);
      try {
        const found = await ipc.projectTasks(projectId, refresh, closed);
        if (projectRequests.get(projectId) !== request) return;
        set((s) => ({ byProject: { ...s.byProject, [projectId]: found } }));
      } catch (error) {
        if (projectRequests.get(projectId) !== request) return;
        set((s) => ({
          byProject: {
            ...s.byProject,
            // `gh` is not what failed: the core could not be asked at all.
            [projectId]: { ...NO_TASKS, gh: true, problem: errorMessage(error) },
          },
        }));
      }
    },

    wantClosed(wanted) {
      if (get().closedWanted !== wanted) set({ closedWanted: wanted });
    },

    select(key) {
      set({ selected: key, error: null });
    },

    async loadDetail(target, force = false) {
      const { key } = target;
      const stamp = detailStamp(target.task);
      const before = get().details[key];
      if (!force && before && before.stamp === stamp && (before.loading || !before.error)) return;
      const request = (detailRequests.get(key) ?? 0) + 1;
      detailRequests.set(key, request);
      set((s) => ({
        details: {
          ...s.details,
          [key]: { detail: before?.detail ?? null, loading: true, error: null, stamp },
        },
      }));
      try {
        // The first look may use what the core read a moment ago; anything after it is because
        // something changed, and must not be answered from before the change.
        const detail = await ipc.taskDetail(
          target.projectId,
          target.task.key,
          force || before !== undefined,
        );
        if (detailRequests.get(key) === request) patch(key, { detail, loading: false });
      } catch (error) {
        if (detailRequests.get(key) === request)
          patch(key, { loading: false, error: errorMessage(error) });
      }
    },
  };
});
