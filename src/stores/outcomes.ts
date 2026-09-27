import { create } from "zustand";
import { errorMessage, ipc } from "@/lib/ipc";

interface OutcomesState {
  /** The attempt just archived or deleted, whose outcome the sidebar asks about once. */
  asking: { id: string; name: string } | null;
  error: string | null;
  ask: (id: string, name: string) => void;
  dismiss: () => void;
  /** Say how the attempt being asked about went. */
  answer: (label: "kept" | "partly" | "discarded") => Promise<void>;
}

/**
 * The one-click question after a workspace is archived or deleted: how did it go? Optional —
 * dismissing it leaves the attempt unlabelled, and the Outcomes view can label it any time.
 */
export const useOutcomesStore = create<OutcomesState>((set, get) => ({
  asking: null,
  error: null,
  ask: (id, name) => set({ asking: { id, name }, error: null }),
  dismiss: () => set({ asking: null, error: null }),
  async answer(label) {
    const asking = get().asking;
    if (!asking) return;
    try {
      await ipc.outcomeLabel(asking.id, label);
      set({ asking: null, error: null });
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },
}));
