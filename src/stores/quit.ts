import { create } from "zustand";
import { hasCore, ipc } from "@/lib/ipc";

interface QuitState {
  /** The PTY sessions the core says are still working, or `null` when nothing is being asked. */
  agents: string[] | null;
  /** An answer is on its way to the core; the buttons wait rather than be pressed twice. */
  deciding: boolean;
  asked: (agents: string[]) => void;
  quit: (stopAgents: boolean) => Promise<void>;
  cancel: () => Promise<void>;
}

/**
 * The question the core asks when the window is closed with agents still running. The core
 * decides *whether* to ask — it owns the session list — and this only carries the answer back.
 */
export const useQuitStore = create<QuitState>((set) => ({
  agents: null,
  deciding: false,

  asked(agents) {
    set({ agents, deciding: false });
  },

  async quit(stopAgents) {
    set({ deciding: true });
    try {
      await ipc.appQuit(stopAgents);
    } catch (error) {
      // The app is still here, so the user must be able to try again or carry on working.
      console.error(error);
      set({ agents: null, deciding: false });
      await ipc.quitCancelled().catch(console.error);
    }
  },

  async cancel() {
    set({ agents: null, deciding: false });
    await ipc.quitCancelled().catch(console.error);
  },
}));

/** Listen for the core's question. Mounted once, with the rest of the shell. */
export function listenForQuitRequests(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  return ipc.onQuitRequested((agents) => useQuitStore.getState().asked(agents));
}
