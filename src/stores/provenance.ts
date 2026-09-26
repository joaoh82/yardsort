import { create } from "zustand";
import { ipc, type Provenance } from "@/lib/ipc";

interface ProvenanceState {
  workspaceId: string | null;
  /** What the core joined for `workspaceId`; `null` until it answers, or when it could not. */
  provenance: Provenance | null;
  /** Point at a workspace (or none) and load what its agents reported. */
  follow: (workspaceId: string | null) => Promise<void>;
  /** Ask again: a hook landed, or the files changed. */
  refresh: () => Promise<void>;
}

/**
 * Which files the workspace's agents said they wrote, beside the change list. Advisory, like
 * Assist: the list reads the same with it all missing, so a failure to load is kept quiet
 * rather than shown as an error over the diff.
 */
export const useProvenanceStore = create<ProvenanceState>((set, get) => ({
  workspaceId: null,
  provenance: null,

  async follow(workspaceId) {
    if (get().workspaceId === workspaceId) return;
    set({ workspaceId, provenance: null });
    await get().refresh();
  },

  async refresh() {
    const { workspaceId } = get();
    if (!workspaceId) return;
    try {
      const provenance = await ipc.workspaceProvenance(workspaceId);
      if (get().workspaceId === workspaceId) set({ provenance });
    } catch (error) {
      console.error(error);
    }
  },
}));
