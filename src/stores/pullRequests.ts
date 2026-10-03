import { create } from "zustand";
import {
  errorMessage,
  ipc,
  type MergeMethod,
  type LineComment,
  type LinePlace,
  type PullRequest,
  type PullRequestChanges,
  type PullRequestSummary,
} from "@/lib/ipc";
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

/** One pull request read in full, or the reading of it. */
export interface SummaryState {
  /** The last one read. Kept while a newer one is on its way, so the pane does not blink. */
  summary: PullRequestSummary | null;
  loading: boolean;
  error: string | null;
  /** What the list said about the pull request when this was asked for: see `loadSummary`. */
  stamp: string;
}

/**
 * What the list knows about a pull request that the summary would also change with: a push, a
 * comment or a review (both move `updatedAt`), and its checks, which move nothing else.
 */
export function summaryStamp(pr: PullRequest): string {
  const counts = pr.details?.checkCounts;
  return [
    pr.state,
    pr.details?.headOid ?? "",
    pr.details?.updatedAt ?? "",
    counts ? `${counts.passed}/${counts.failed}/${counts.running}` : "",
  ].join("|");
}

/** The files a pull request changes, or the fetching of them. */
export interface ChangesState {
  /** The last list read. Kept while a newer one is on its way. */
  changes: PullRequestChanges | null;
  loading: boolean;
  error: string | null;
  /** The head commit the list named when this was asked for: a push moves it, nothing else. */
  head: string;
}

export type DetailTab = "summary" | "code";

/** The comments on lines of a pull request's diff, or the reading of them. */
export interface LineCommentsState {
  comments: LineComment[];
  loading: boolean;
  error: string | null;
  /** As `SummaryState::stamp`: a comment moves `updatedAt`, so the stamp says when to ask again. */
  stamp: string;
}

interface PullRequestsStore {
  /** The row open in the detail pane. */
  selected: string | null;
  /** Which of its tabs is showing. Kept from one pull request to the next, until you quit:
   *  someone reading diffs wants the next one's diff too. */
  tab: DetailTab;
  showTab: (tab: DetailTab) => void;
  /** Pull requests read in full, by row key: what the Summary shows. */
  summaries: Record<string, SummaryState>;
  /** The files each pull request changes, by row key: what Code shows. */
  changes: Record<string, ChangesState>;
  /** The comments on lines of each pull request's diff, by row key: shown beside the lines. */
  lineComments: Record<string, LineCommentsState>;
  /** The row something is being done to. One at a time: each action moves the others' ground. */
  busy: string | null;
  error: string | null;
  notice: string | null;

  select: (key: string | null) => void;
  dismiss: () => void;
  /**
   * Read a pull request in full, unless that was already done for what the list says now. The
   * list is asked every minute while the view is open, so a summary follows it: when the row
   * changes, the summary is read again. `force` reads it regardless — Retry.
   */
  loadSummary: (target: Target, force?: boolean) => Promise<void>;
  /**
   * Read the files a pull request changes, unless that was done for the head commit the list
   * names now. The first time, and after a push, this fetches the pull request's commits —
   * which is git over the network, and can take a moment or fail.
   */
  loadChanges: (target: Target, force?: boolean) => Promise<void>;
  /** Read the comments on lines, unless that was done for what the list says now. */
  loadLineComments: (target: Target, force?: boolean) => Promise<void>;
  /** Post a comment on the conversation. Resolves to whether it was posted; the error is kept. */
  comment: (target: Target, body: string) => Promise<boolean>;
  /** Post a comment on lines of the diff. */
  lineComment: (target: Target, place: LinePlace, body: string) => Promise<boolean>;
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
  /** The latest request per row: an answer that is no longer the latest is dropped. */
  const asked = new Map<string, number>();
  const askedChanges = new Map<string, number>();
  const askedComments = new Map<string, number>();

  /**
   * After something was posted: the list, the summary and the comments are all from before.
   *
   * Not awaited by the actions that post. Reading it all back is `gh` over the network, and
   * while that runs the words are already on the forge: the box they came from must clear and
   * the button must be free *now*, or a second press in those seconds posts them again.
   */
  function posted(target: Target) {
    const refresh = async () => {
      await usePublishStore.getState().loadProject(target.projectId, true, true);
      const latest = get();
      if (latest.summaries[target.key]) await latest.loadSummary(target, true);
      if (latest.lineComments[target.key]) await latest.loadLineComments(target, true);
    };
    void refresh().catch(console.error);
  }
  const patch = (key: string, next: Partial<SummaryState>) =>
    set((s) => {
      const before = s.summaries[key];
      if (!before) return s;
      return { summaries: { ...s.summaries, [key]: { ...before, ...next } } };
    });

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
      // The list may say the same as before — a refusal changes nothing — but the forge was
      // just told something, so what it says about this one is asked for again either way.
      if (get().summaries[target.key]) await get().loadSummary(target, true);
    }
  }

  return {
    selected: null,
    tab: "summary",
    summaries: {},
    changes: {},
    lineComments: {},

    showTab(tab) {
      set({ tab });
    },
    busy: null,
    error: null,
    notice: null,

    async loadSummary(target, force = false) {
      const { key } = target;
      const stamp = summaryStamp(target.pr);
      const before = get().summaries[key];
      if (!force && before && before.stamp === stamp && (before.loading || !before.error)) return;
      const request = (asked.get(key) ?? 0) + 1;
      asked.set(key, request);
      set((s) => ({
        summaries: {
          ...s.summaries,
          [key]: { summary: before?.summary ?? null, loading: true, error: null, stamp },
        },
      }));
      try {
        // The first look may use what the core read a moment ago; anything after it is because
        // something changed, and must not be answered from before the change.
        const summary = await ipc.pullRequestSummary(
          target.projectId,
          target.pr.number,
          force || before !== undefined,
        );
        if (asked.get(key) === request) patch(key, { summary, loading: false });
      } catch (error) {
        if (asked.get(key) === request) patch(key, { loading: false, error: errorMessage(error) });
      }
    },

    async loadChanges(target, force = false) {
      const { key } = target;
      const head = target.pr.details?.headOid ?? "";
      const before = get().changes[key];
      if (!force && before && before.head === head && (before.loading || !before.error)) return;
      const request = (askedChanges.get(key) ?? 0) + 1;
      askedChanges.set(key, request);
      const put = (next: Partial<ChangesState>) =>
        set((s) => {
          const now = s.changes[key];
          return now ? { changes: { ...s.changes, [key]: { ...now, ...next } } } : s;
        });
      set((s) => ({
        changes: {
          ...s.changes,
          [key]: { changes: before?.changes ?? null, loading: true, error: null, head },
        },
      }));
      try {
        const changes = await ipc.pullRequestChanges(target.projectId, target.pr.number);
        if (askedChanges.get(key) === request) put({ changes, loading: false });
      } catch (error) {
        if (askedChanges.get(key) === request) put({ loading: false, error: errorMessage(error) });
      }
    },

    async loadLineComments(target, force = false) {
      const { key } = target;
      const stamp = summaryStamp(target.pr);
      const before = get().lineComments[key];
      if (!force && before && before.stamp === stamp && (before.loading || !before.error)) return;
      const request = (askedComments.get(key) ?? 0) + 1;
      askedComments.set(key, request);
      const put = (next: Partial<LineCommentsState>) =>
        set((s) => {
          const now = s.lineComments[key];
          return now ? { lineComments: { ...s.lineComments, [key]: { ...now, ...next } } } : s;
        });
      set((s) => ({
        lineComments: {
          ...s.lineComments,
          [key]: { comments: before?.comments ?? [], loading: true, error: null, stamp },
        },
      }));
      try {
        const comments = await ipc.pullRequestLineComments(target.projectId, target.pr.number);
        if (askedComments.get(key) === request) put({ comments, loading: false });
      } catch (error) {
        if (askedComments.get(key) === request) put({ loading: false, error: errorMessage(error) });
      }
    },

    async comment(target, body) {
      if (get().busy) return false;
      set({ busy: target.key, error: null, notice: null });
      try {
        await ipc.pullRequestComment(target.projectId, target.pr.number, body);
        set({ notice: `Commented on #${target.pr.number}.` });
        return true;
      } catch (error) {
        set({ error: errorMessage(error) });
        return false;
      } finally {
        set({ busy: null });
        posted(target);
      }
    },

    async lineComment(target, place, body) {
      if (get().busy) return false;
      set({ busy: target.key, error: null, notice: null });
      try {
        await ipc.pullRequestLineComment(target.projectId, target.pr.number, place, body);
        set({ notice: `Commented on ${place.path} in #${target.pr.number}.` });
        return true;
      } catch (error) {
        set({ error: errorMessage(error) });
        return false;
      } finally {
        set({ busy: null });
        posted(target);
      }
    },

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
