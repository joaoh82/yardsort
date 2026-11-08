import { create } from "zustand";
import { errorMessage, hasCore, ipc, type OtherComputers, type OtherHostView } from "@/lib/ipc";

interface OtherComputersState {
  /** This computer as a client of other hosts, and the hosts it asked to reach. */
  status: OtherComputers | null;
  /** What each host showed when last read, by its id, for the hosts the window has open. */
  views: Record<string, OtherHostView | null>;
  error: string | null;
  load: () => Promise<void>;
  /** Read what one host shows, as the core last read it. */
  loadHost: (hostId: string) => Promise<void>;
  /** An action that answers with the new status: shown, or its failure said. */
  run: (action: () => Promise<OtherComputers>) => Promise<boolean>;
}

/**
 * Remote the other way round: the hosts this computer reaches as a client, which the sidebar lists
 * and Settings → Remote manages. The core keeps the links and reads each host; this mirrors it.
 */
export const useOtherComputersStore = create<OtherComputersState>((set, get) => ({
  status: null,
  views: {},
  error: null,

  async load() {
    try {
      set({ status: await ipc.otherComputers(), error: null });
    } catch (reason) {
      set({ error: errorMessage(reason) });
    }
  },

  async loadHost(hostId) {
    try {
      const view = await ipc.otherHost(hostId);
      set((state) => ({ views: { ...state.views, [hostId]: view } }));
    } catch (reason) {
      set({ error: errorMessage(reason) });
    }
  },

  async run(action) {
    set({ error: null });
    try {
      set({ status: await action() });
      return true;
    } catch (reason) {
      // Where things stand now, then why the action failed: reading them again must not hide it.
      await get().load();
      set({ error: errorMessage(reason) });
      return false;
    }
  },
}));

/** How soon after the core says something changed the window looks: links can change in bursts. */
const SETTLE_MS = 250;

/** Follow the core's word that something changed. Mounted once, with the rest of the shell. */
export function listenForOtherComputers(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  void useOtherComputersStore.getState().load();
  let timer: ReturnType<typeof setTimeout> | undefined;
  return ipc.onOtherComputersChanged(() => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const store = useOtherComputersStore.getState();
      void store.load();
      for (const hostId of Object.keys(store.views)) void store.loadHost(hostId);
    }, SETTLE_MS);
  });
}
