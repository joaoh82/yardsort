import { create } from "zustand";
import {
  errorMessage,
  ipc,
  type ProjectPullRequests,
  type PublishState,
  type PullRequest,
  type PullRequestOpened,
} from "@/lib/ipc";
import { useProjectsStore } from "./projects";

/** Which button is mid-flight. Only one at a time: each of them moves the others' ground. */
export type Busy = "commit" | "push" | "pullRequest";

const NO_PULL_REQUESTS: ProjectPullRequests = {
  gh: false,
  pullRequests: [],
  problem: null,
  loggedOut: false,
};

interface PublishStore {
  /** The workspace `state` describes, which is the one the right panel is showing. */
  workspaceId: string | null;
  state: PublishState | null;
  error: string | null;
  busy: Busy | null;
  /**
   * Pull requests per project, for the sidebar. Keyed by project so that reloading one leaves
   * every other project's entry identical — a selector reading one must not see a new object
   * because a different project was refreshed.
   */
  byProject: Record<string, ProjectPullRequests>;

  follow: (workspaceId: string | null) => Promise<void>;
  refresh: (refreshForge?: boolean) => Promise<void>;
  commit: (message: string) => Promise<boolean>;
  push: () => Promise<boolean>;
  openPullRequest: (pr: {
    title: string;
    body: string;
    draft: boolean;
  }) => Promise<PullRequestOpened | null>;
  /** Load one project's pull requests. Never throws: a forge that will not answer says so. */
  loadProject: (projectId: string, refresh?: boolean) => Promise<void>;
  /** Ask the forge again about the project the followed workspace belongs to. */
  reloadProject: () => Promise<void>;
  clearError: () => void;
}

/** The pull request for a branch, out of what a project answered. */
export function pullRequestFor(
  found: ProjectPullRequests | undefined,
  branch: string | undefined,
): PullRequest | undefined {
  if (!found || !branch) return undefined;
  return found.pullRequests.find((pr) => pr.branch === branch);
}

export const usePublishStore = create<PublishStore>((set, get) => {
  /**
   * Run one of the three actions, keeping the panel honest about what is happening.
   *
   * They all end the same way — the core hands back where the workspace now stands — so the
   * busy flag, the error and the fresh state are handled once rather than three times.
   */
  async function act<T>(
    busy: Busy,
    run: (workspaceId: string) => Promise<{ state: PublishState; result: T }>,
  ): Promise<T | null> {
    const { workspaceId } = get();
    if (!workspaceId || get().busy) return null;
    set({ busy, error: null });
    try {
      const { state, result } = await run(workspaceId);
      // The panel may have moved on while the network was busy; its state is not ours to set.
      if (get().workspaceId === workspaceId) set({ state });
      return result;
    } catch (error) {
      if (get().workspaceId === workspaceId) set({ error: errorMessage(error) });
      return null;
    } finally {
      set({ busy: null });
    }
  }

  return {
    workspaceId: null,
    state: null,
    error: null,
    busy: null,
    byProject: {},

    async follow(workspaceId) {
      if (get().workspaceId === workspaceId) return;
      set({ workspaceId, state: null, error: null, busy: null });
      await get().refresh();
    },

    async refresh(refreshForge = false) {
      const { workspaceId } = get();
      if (!workspaceId) return;
      try {
        const state = await ipc.workspacePublishState(workspaceId, refreshForge);
        if (get().workspaceId === workspaceId) set({ state });
      } catch (error) {
        // A workspace whose folder has gone is the panel's news to break, not ours: it says so
        // already, and a second complaint under the commit box would only repeat it.
        if (get().workspaceId === workspaceId) set({ state: null, error: errorMessage(error) });
      }
    },

    async commit(message) {
      const state = await act("commit", async (workspaceId) => ({
        state: await ipc.workspaceCommit(workspaceId, message),
        result: true,
      }));
      return state === true;
    },

    async push() {
      const pushed = await act("push", async (workspaceId) => ({
        state: await ipc.workspacePush(workspaceId),
        result: true,
      }));
      if (pushed) await get().reloadProject();
      return pushed === true;
    },

    async openPullRequest(pr) {
      const opened = await act("pullRequest", async (workspaceId) => {
        const result = await ipc.workspaceOpenPullRequest(workspaceId, pr);
        // Pushing and opening both moved the forge on, so ask it rather than reuse its answer.
        return { state: await ipc.workspacePublishState(workspaceId, true), result };
      });
      if (opened) await get().reloadProject();
      return opened;
    },

    async loadProject(projectId, refresh = false) {
      try {
        const found = await ipc.projectPullRequests(projectId, refresh);
        set((s) => ({ byProject: { ...s.byProject, [projectId]: found } }));
      } catch {
        // Nothing to say: no pull requests simply means no badges on those rows.
        set((s) => ({ byProject: { ...s.byProject, [projectId]: NO_PULL_REQUESTS } }));
      }
    },

    async reloadProject() {
      const projectId = projectOf(get().workspaceId);
      if (projectId) await get().loadProject(projectId, true);
    },

    clearError() {
      set({ error: null });
    },
  };
});

/**
 * The project a workspace belongs to. Asked of the projects store rather than carried on
 * `PublishState`: the sidebar already has the mapping, and a second copy could disagree with it.
 */
function projectOf(workspaceId: string | null): string | null {
  if (!workspaceId) return null;
  const projects = useProjectsStore.getState().projects;
  const owner = projects.find((project) =>
    project.workspaces.some((workspace) => workspace.id === workspaceId),
  );
  return owner?.id ?? null;
}
