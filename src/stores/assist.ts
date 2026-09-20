import { create } from "zustand";
import { errorMessage, ipc, isIpcError, type AssistStatus, type Review } from "@/lib/ipc";

/**
 * Assist: what the core knows about the TypeSafe key and the switches, plus the review of the
 * workspace on screen.
 *
 * Nothing here is load-bearing. With Assist off, without a key, or with TypeSafe unreachable, the
 * store stays empty and every panel renders exactly as it did before.
 */

/** Agents write in bursts; wait for one to settle before asking about the diff it produced. */
export const REVIEW_DELAY_MS = 4000;

interface AssistState {
  status: AssistStatus | null;
  /** The workspace `review` belongs to. */
  workspaceId: string | null;
  review: Review | null;
  reviewing: boolean;
  /** A failure worth showing above the change list; never blocks anything. */
  error: string | null;

  load: () => Promise<void>;
  adopt: (status: AssistStatus) => void;
  /** Point the review at a workspace (or none). Its changes are checked shortly after. */
  follow: (workspaceId: string | null) => void;
  /** The files changed: check again once the writing has stopped. */
  reviewSoon: () => void;
  reviewNow: () => Promise<void>;
}

export const assistOn = (status: AssistStatus | null) =>
  status !== null && status.keySource !== "none";

let timer: ReturnType<typeof setTimeout> | undefined;

export const useAssistStore = create<AssistState>((set, get) => ({
  status: null,
  workspaceId: null,
  review: null,
  reviewing: false,
  error: null,

  async load() {
    try {
      set({ status: await ipc.assistStatus() });
    } catch {
      // A core that cannot answer leaves Assist simply unavailable.
    }
  },

  adopt(status) {
    set({ status });
    if (!status.reviewChanges) set({ review: null, error: null });
  },

  follow(workspaceId) {
    if (get().workspaceId === workspaceId) return;
    clearTimeout(timer);
    set({ workspaceId, review: null, error: null });
    get().reviewSoon();
  },

  reviewSoon() {
    clearTimeout(timer);
    if (!get().workspaceId) return;
    timer = setTimeout(() => void get().reviewNow(), REVIEW_DELAY_MS);
  },

  async reviewNow() {
    const { workspaceId, status, reviewing } = get();
    if (!workspaceId || reviewing || !status?.reviewChanges || !assistOn(status)) return;
    set({ reviewing: true });
    try {
      const review = await ipc.assistReview(workspaceId);
      if (get().workspaceId !== workspaceId) return;
      set({ review, error: review.problem });
    } catch (reason) {
      if (get().workspaceId !== workspaceId) return;
      // Switched off or without a key is not an error to shout about; the settings say so.
      const quiet = isIpcError(reason) && ["assist_off", "assist_no_key"].includes(reason.code);
      set({ error: quiet ? null : errorMessage(reason) });
    } finally {
      set({ reviewing: false });
    }
  },
}));
