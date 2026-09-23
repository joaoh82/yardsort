import { create } from "zustand";
import { errorMessage, ipc, type DraftedPullRequest, type DraftStatus } from "@/lib/ipc";
import { useSessionsStore } from "./sessions";

/** Which drafting request is in flight. */
export type Drafting = "commitMessage" | "pullRequest";

interface DraftStore {
  status: DraftStatus | null;
  busy: Drafting | null;
  error: string | null;

  /** Ask who can write here. Cheap, and the answer decides whether a button appears at all. */
  load: (harnessId?: string | null) => Promise<void>;
  commitMessage: (workspaceId: string) => Promise<string | null>;
  pullRequest: (workspaceId: string) => Promise<DraftedPullRequest | null>;
  clearError: () => void;
}

/**
 * The agent this workspace has been using, which gets first refusal at writing.
 *
 * Read from the conversation history rather than carried anywhere: the newest record is the
 * agent the user last chose for this work, and it is already the store's business to know.
 */
function agentOf(workspaceId: string): string | null {
  const records = useSessionsStore.getState().byWorkspace[workspaceId];
  return records?.[0]?.harnessId ?? null;
}

export const useDraftStore = create<DraftStore>((set, get) => {
  async function draft<T>(busy: Drafting, run: () => Promise<T>): Promise<T | null> {
    if (get().busy) return null;
    set({ busy, error: null });
    try {
      return await run();
    } catch (error) {
      set({ error: errorMessage(error) });
      return null;
    } finally {
      set({ busy: null });
    }
  }

  return {
    status: null,
    busy: null,
    error: null,

    async load(harnessId = null) {
      try {
        set({ status: await ipc.draftStatus(harnessId) });
      } catch {
        // Not being able to ask is the same as nobody being able to write: no button.
        set({ status: null });
      }
    },

    commitMessage: (workspaceId) =>
      draft("commitMessage", () => ipc.draftCommitMessage(workspaceId, agentOf(workspaceId))),

    pullRequest: (workspaceId) =>
      draft("pullRequest", () => ipc.draftPullRequest(workspaceId, agentOf(workspaceId))),

    clearError() {
      set({ error: null });
    },
  };
});
