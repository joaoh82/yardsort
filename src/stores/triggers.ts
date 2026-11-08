import { create } from "zustand";
import {
  errorMessage,
  hasCore,
  ipc,
  type TriggeredRun,
  type TriggerFire,
  type TriggerItem,
  type TriggerPreset,
} from "@/lib/ipc";

interface TriggersState {
  /** Every trigger of every project, by name, as the core lists them. */
  items: TriggerItem[];
  loaded: boolean;
  /** The history of each trigger whose history was asked for, newest first. */
  history: Record<string, TriggerFire[] | undefined>;
  /** The runs triggers started, for run lists to mark. */
  started: TriggeredRun[];
  /** The ready-made triggers. They never change while the app runs. */
  presets: TriggerPreset[];
  error: string | null;

  load: () => Promise<void>;
  loadHistory: (triggerId: string) => Promise<void>;
  loadPresets: () => Promise<void>;
  /** Everything already loaded, again: the list, and the histories on screen. */
  refresh: () => Promise<void>;
  /** Switch a trigger on, from now, or off. */
  setEnabled: (triggerId: string, enabled: boolean) => Promise<boolean>;
  /** Lift a trigger's cap. */
  reset: (triggerId: string) => Promise<boolean>;
  /** Remove a trigger and its history. The runs it started stay. */
  remove: (triggerId: string) => Promise<boolean>;
  dismissError: () => void;
}

export const useTriggersStore = create<TriggersState>((set, get) => ({
  items: [],
  loaded: false,
  history: {},
  started: [],
  presets: [],
  error: null,

  async load() {
    if (!hasCore()) return;
    try {
      const [items, started] = await Promise.all([ipc.triggerList(), ipc.triggerStartedRuns()]);
      set({ items, started, loaded: true });
    } catch (error) {
      set({ error: errorMessage(error), loaded: true });
    }
  },

  async loadHistory(triggerId) {
    if (!hasCore()) return;
    try {
      const fires = await ipc.triggerFires(triggerId);
      set((state) => ({ history: { ...state.history, [triggerId]: fires } }));
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async loadPresets() {
    if (!hasCore() || get().presets.length > 0) return;
    try {
      set({ presets: await ipc.triggerPresets() });
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async refresh() {
    const { history, load, loadHistory } = get();
    await Promise.all([load(), ...Object.keys(history).map((id) => loadHistory(id))]);
  },

  setEnabled: (triggerId, enabled) => act(() => ipc.triggerSetEnabled(triggerId, enabled)),

  reset: (triggerId) => act(() => ipc.triggerReset(triggerId)),

  async remove(triggerId) {
    const removed = await act(() => ipc.triggerRemove(triggerId));
    if (removed) {
      set((state) => {
        const history = { ...state.history };
        delete history[triggerId];
        return { history };
      });
    }
    return removed;
  },

  dismissError: () => set({ error: null }),
}));

/** Do something to a trigger and show what went wrong, if anything; then read it all again. */
async function act(action: () => Promise<unknown>): Promise<boolean> {
  try {
    useTriggersStore.setState({ error: null });
    await action();
    await useTriggersStore.getState().refresh();
    return true;
  } catch (error) {
    useTriggersStore.setState({ error: errorMessage(error) });
    return false;
  }
}

/** Keep what is shown of triggers current as the core fires them. Mount once. */
export function listenForTriggers(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  void useTriggersStore.getState().load();
  return ipc.onTriggersChanged(() => void useTriggersStore.getState().refresh());
}

/** How many triggers need the user before they can do their job: the number on the sidebar
 *  row. One that is capped, one whose last fire was refused, one whose run waits for approval,
 *  one with no workspace. */
export function needingAttention(items: TriggerItem[]): number {
  return items.filter((item) => needsAttention(item)).length;
}

export function needsAttention(item: Pick<TriggerItem, "standing">): boolean {
  return (
    item.standing === "awaiting_approval" ||
    item.standing === "capped" ||
    item.standing === "refused" ||
    item.standing === "no_workspace"
  );
}

/** The trigger that started a run, by the run's id; undefined for a run no trigger started. */
export function triggerOf(started: TriggeredRun[], runId: string): TriggeredRun | undefined {
  return started.find((mark) => mark.runId === runId);
}
