import { create } from "zustand";
import { errorMessage, ipc, type MergeMethod, type PullRequest } from "@/lib/ipc";
import { native } from "@/lib/native";
import { useProjectsStore } from "./projects";
import { usePublishStore } from "./publish";

/** The pull request an action is about, with what its confirmation has to name. */
export interface Target {
  /** The row's key: see `rowKey`. */
  key: string;
  projectId: string;
  pr: PullRequest;
  /** Who `gh` is logged in as, so a confirmation can say when the pull request is not yours. */
  viewer: string | null;
}

interface PullRequestsStore {
  /** The row open in the detail pane. */
  selected: string | null;
  /** The row something is being done to. One at a time: each action moves the others' ground. */
  busy: string | null;
  error: string | null;
  notice: string | null;

  select: (key: string | null) => void;
  dismiss: () => void;
  merge: (target: Target, method: MergeMethod, label: string) => Promise<void>;
  close: (target: Target) => Promise<void>;
  reopen: (target: Target) => Promise<void>;
  /** Get the pull request's branch ready and open the composer on it. */
  startWorkspace: (target: Target) => Promise<void>;
}

/** `ys/fix → main`, when the base is known. */
const branches = (pr: PullRequest) =>
  pr.details?.base ? `${pr.branch} → ${pr.details.base}` : pr.branch;

/**
 * Whose pull request this is, said plainly when it is not the user's own. From this view any
 * pull request of the project can be acted on, so the confirmation is what stands between a
 * slip and someone else's work.
 */
function whose(target: Target): string {
  const author = target.pr.author;
  if (!author) return "Its author's account no longer exists.";
  const yours = !!target.viewer && author.toLowerCase() === target.viewer.toLowerCase();
  return yours ? "You opened it." : `${author} opened it, not you.`;
}

export const usePullRequestsStore = create<PullRequestsStore>((set, get) => {
  /**
   * Run one action on one pull request. The forge is asked again afterwards whatever happened:
   * a refusal usually means the list was out of date.
   */
  async function act(target: Target, run: () => Promise<string | null>) {
    if (get().busy) return;
    set({ busy: target.key, error: null, notice: null });
    try {
      const notice = await run();
      set({ notice });
    } catch (error) {
      set({ error: errorMessage(error) });
    } finally {
      set({ busy: null });
      await usePublishStore.getState().loadProject(target.projectId, true, true);
    }
  }

  return {
    selected: null,
    busy: null,
    error: null,
    notice: null,

    select(key) {
      set({ selected: key, error: null, notice: null });
    },

    dismiss() {
      set({ error: null, notice: null });
    },

    async merge(target, method, label) {
      const { pr } = target;
      const head = pr.details?.headOid;
      if (!head) return;
      const confirmed = await native.confirm(
        `${label} pull request #${pr.number}: ${pr.title}\n\n${whose(target)}\n\n${branches(pr)}\nCommit ${head.slice(0, 12)}\n\nGitHub may queue the merge if this branch requires it. No branch is deleted.`,
        { title: "Merge pull request", okLabel: label },
      );
      if (!confirmed) return;
      await act(target, async () => {
        await ipc.pullRequestMerge(target.projectId, pr.number, head, method);
        return "Merge request sent. GitHub may queue it; refresh to check its status.";
      });
    },

    async close(target) {
      const { pr } = target;
      const confirmed = await native.confirm(
        `Close pull request #${pr.number} without merging it?\n\n${pr.title}\n${branches(pr)}\n\n${whose(target)}\n\nIts branch is kept and nothing is posted on it. It can be reopened.`,
        { title: "Close pull request", okLabel: "Close pull request" },
      );
      if (!confirmed) return;
      await act(target, async () => {
        await ipc.pullRequestClose(target.projectId, pr.number);
        return `Closed #${pr.number}.`;
      });
    },

    async reopen(target) {
      const { pr } = target;
      const confirmed = await native.confirm(
        `Reopen pull request #${pr.number}?\n\n${pr.title}\n${branches(pr)}\n\n${whose(target)}\n\nIts reviewers are told, and its checks may run again.`,
        { title: "Reopen pull request", okLabel: "Reopen" },
      );
      if (!confirmed) return;
      await act(target, async () => {
        await ipc.pullRequestReopen(target.projectId, pr.number);
        return `Reopened #${pr.number}.`;
      });
    },

    async startWorkspace(target) {
      if (get().busy) return;
      set({ busy: target.key, error: null, notice: null });
      try {
        const prepared = await ipc.pullRequestPrepareBranch(target.projectId, target.pr.number);
        // The composer takes it from here — the agent, the message and the worktree are its.
        useProjectsStore.getState().compose(target.projectId, prepared);
      } catch (error) {
        set({ error: errorMessage(error) });
      } finally {
        set({ busy: null });
      }
    },
  };
});
