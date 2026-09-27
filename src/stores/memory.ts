import { create } from "zustand";
import { hasCore, ipc } from "@/lib/ipc";

interface MemoryState {
  /** How many proposals wait for the user, per project that has any. */
  waiting: Record<string, number>;
  /**
   * Ask again. Agents propose through `ys`, which writes the database without the window
   * hearing of it, so this runs on focus, when activity lands, and after every decision.
   */
  refreshWaiting: () => Promise<void>;
}

/** The counts on the project rows. The Memory view loads its own project's entries. */
export const useMemoryStore = create<MemoryState>((set) => ({
  waiting: {},
  async refreshWaiting() {
    if (!hasCore()) return;
    try {
      const counts = await ipc.memoryWaiting();
      set({ waiting: Object.fromEntries(counts.map((c) => [c.projectId, c.count])) });
    } catch (error) {
      console.error(error);
    }
  },
}));
