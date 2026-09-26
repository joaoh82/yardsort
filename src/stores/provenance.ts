import { create } from "zustand";
import { ipc, type Provenance } from "@/lib/ipc";
import { useChangesStore } from "@/stores/changes";

interface ProvenanceState {
  workspaceId: string | null;
  /** What the core joined for `workspaceId`; `null` until it answers, or when it could not. */
  provenance: Provenance | null;
  /** Point at a workspace (or none) and load what its agents reported. */
  follow: (workspaceId: string | null) => Promise<void>;
  /**
   * Ask again: a hook landed, or the files changed. The change list's files are what the core
   * looks at on disk, so this runs after the list has been loaded.
   */
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
    const changes = useChangesStore.getState();
    const listed =
      changes.workspaceId === workspaceId && changes.changes
        ? [
            ...new Set(
              [...changes.changes.uncommitted, ...changes.changes.committed].map((c) => c.path),
            ),
          ]
        : [];
    try {
      const provenance = await ipc.workspaceProvenance(workspaceId, listed);
      if (get().workspaceId === workspaceId) set({ provenance });
    } catch (error) {
      console.error(error);
    }
  },
}));
