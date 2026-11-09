import { create } from "zustand";
import {
  errorMessage,
  hasCore,
  ipc,
  type CoordinatorStatus,
  type HostEvent,
  type TermSize,
} from "@/lib/ipc";

/** Where the harness last chosen for the Floor Manager is remembered, in the core's `ui_state`. */
export const HARNESS_KEY = "floorManager.harness";

interface CoordinatorState {
  /** The panel is showing. Closing it leaves the agent running. */
  open: boolean;
  /** What the core said last. `null` until it has been asked. */
  status: CoordinatorStatus | null;
  /** The harness picked for the next start. `null` means the first one that can start. */
  chosen: string | null;
  busy: boolean;
  error: string | null;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  choose: (harness: string) => void;
  refresh: () => Promise<void>;
  start: (harness: string, size: TermSize) => Promise<void>;
  stop: () => Promise<void>;
  /** A host event: the Floor Manager ending is the one that matters here. */
  observe: (event: HostEvent) => void;
}

/**
 * The Floor Manager panel: whether it is open, and a cache of what the core says about the one
 * agent it runs. The agent itself belongs to the daemon, like every other.
 */
export const useCoordinatorStore = create<CoordinatorState>((set, get) => ({
  open: false,
  status: null,
  chosen: null,
  busy: false,
  error: null,

  setOpen(open) {
    set({ open });
    if (open) void get().refresh();
  },
  toggle() {
    get().setOpen(!get().open);
  },
  choose(harness) {
    set({ chosen: harness });
    if (hasCore()) void ipc.uiStateSave(HARNESS_KEY, harness).catch(console.error);
  },

  async refresh() {
    if (!hasCore()) return;
    try {
      const [status, ui] = await Promise.all([ipc.coordinatorStatus(), ipc.uiStateLoad()]);
      set((state) => ({ status, chosen: state.chosen ?? ui[HARNESS_KEY] ?? null, error: null }));
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async start(harness, size) {
    if (get().busy) return;
    set({ busy: true, error: null });
    try {
      const status = await ipc.coordinatorStart(harness, size);
      set({ status });
      get().choose(harness);
    } catch (error) {
      set({ error: errorMessage(error) });
    } finally {
      set({ busy: false });
    }
  },

  async stop() {
    if (get().busy) return;
    set({ busy: true, error: null });
    try {
      set({ status: await ipc.coordinatorStop() });
    } catch (error) {
      set({ error: errorMessage(error) });
    } finally {
      set({ busy: false });
    }
  },

  observe(event) {
    if (event.type === "exited" && get().status?.session?.id === event.id) void get().refresh();
  },
}));
