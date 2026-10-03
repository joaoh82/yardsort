import { create } from "zustand";
import {
  errorMessage,
  ipc,
  type ProjectPullRequests,
  type PublishState,
  type PullRequest,
  type PullRequestOpened,
  type Workspace,
} from "@/lib/ipc";
import { useProjectsStore } from "./projects";

/** Which button is mid-flight. Only one at a time: each of them moves the others' ground. */
export type Busy = "commit" | "push" | "pullRequest";

const NO_PULL_REQUESTS: ProjectPullRequests = {
  gh: false,
  pullRequests: [],
  problem: null,
  loggedOut: false,
  workspaces: {},
  repo: null,
  viewer: null,
  openTotal: null,
  openProblem: null,
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
  /**
   * Load one project's pull requests. Never throws: a forge that will not answer says so.
   * `full` also reads every open one, for the Pull requests view.
   */
  loadProject: (projectId: string, refresh?: boolean, full?: boolean) => Promise<void>;
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
  return found.pullRequests
    .filter((pr) => pr.branch === branch)
    .reduce<PullRequest | undefined>((best, pr) => {
      if (!best) return pr;
      if ((pr.state === "open") !== (best.state === "open")) return pr.state === "open" ? pr : best;
      return pr.number > best.number ? pr : best;
    }, undefined);
}

/**
 * Every pull request a workspace opened: the one for the branch it is on first — what it has
 * always shown — then the others the core matched to it, newest first. Which ones belong to it
 * is the core's call (`pull_requests_from`); this only puts the two answers together.
 */
export function pullRequestsFor(
  found: ProjectPullRequests | undefined,
  workspace: Pick<Workspace, "id" | "head">,
): PullRequest[] {
  if (!found) return [];
  const head = workspace.head;
  const current = pullRequestFor(found, head && !head.detached ? head.label : undefined);
  const others = (found.workspaces[workspace.id] ?? [])
    .filter((number) => number !== current?.number)
    .map((number) => found.pullRequests.find((pr) => pr.number === number))
    .filter((pr) => pr !== undefined);
  return current ? [current, ...others] : others;
}

export const usePublishStore = create<PublishStore>((set, get) => {
  const projectRequests = new Map<string, number>();
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

    async loadProject(projectId, refresh = false, full = false) {
      const request = (projectRequests.get(projectId) ?? 0) + 1;
      projectRequests.set(projectId, request);
      try {
        const found = await ipc.projectPullRequests(projectId, refresh, full);
        if (projectRequests.get(projectId) !== request) return;
        set((s) => ({ byProject: { ...s.byProject, [projectId]: found } }));
        // The right panel's pull request is worked out from the same answer, so it follows the
        // poll too: otherwise it would say "checks running" until the workspace's files moved.
        // The core has that answer cached, so this asks the forge nothing.
        if (projectOf(get().workspaceId) === projectId) await get().refresh();
      } catch (error) {
        if (projectRequests.get(projectId) !== request) return;
        set((s) => ({
          byProject: {
            ...s.byProject,
            [projectId]: { ...NO_PULL_REQUESTS, problem: errorMessage(error) },
          },
        }));
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
