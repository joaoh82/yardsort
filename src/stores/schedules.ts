import { create } from "zustand";
import {
  errorMessage,
  hasCore,
  ipc,
  type Schedule,
  type ScheduleRun,
  type ScheduledRun,
} from "@/lib/ipc";

interface SchedulesState {
  /** Every schedule of every project, by name, as the core lists them. */
  items: Schedule[];
  loaded: boolean;
  /** The history of each schedule whose history was asked for, newest first. */
  history: Record<string, ScheduleRun[] | undefined>;
  /** The runs schedules started, for run lists to mark. */
  started: ScheduledRun[];
  error: string | null;

  load: () => Promise<void>;
  loadHistory: (scheduleId: string) => Promise<void>;
  /** Everything already loaded, again: the list, and the histories on screen. */
  refresh: () => Promise<void>;
  /** Switch a schedule on, from now, or off. */
  setEnabled: (scheduleId: string, enabled: boolean) => Promise<boolean>;
  /** Remove a schedule and its history. The runs it started stay. */
  remove: (scheduleId: string) => Promise<boolean>;
  /** Start a schedule's run now. */
  runNow: (scheduleId: string) => Promise<boolean>;
  dismissError: () => void;
}

export const useSchedulesStore = create<SchedulesState>((set, get) => ({
  items: [],
  loaded: false,
  history: {},
  started: [],
  error: null,

  async load() {
    if (!hasCore()) return;
    try {
      const [items, started] = await Promise.all([ipc.scheduleList(), ipc.scheduleStartedRuns()]);
      set({ items, started, loaded: true });
    } catch (error) {
      set({ error: errorMessage(error), loaded: true });
    }
  },

  async loadHistory(scheduleId) {
    if (!hasCore()) return;
    try {
      const runs = await ipc.scheduleRuns(scheduleId);
      set((state) => ({ history: { ...state.history, [scheduleId]: runs } }));
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async refresh() {
    const { history, load, loadHistory } = get();
    await Promise.all([load(), ...Object.keys(history).map((id) => loadHistory(id))]);
  },

  setEnabled: (scheduleId, enabled) => act(() => ipc.scheduleSetEnabled(scheduleId, enabled)),

  async remove(scheduleId) {
    const removed = await act(() => ipc.scheduleRemove(scheduleId));
    if (removed) {
      set((state) => {
        const history = { ...state.history };
        delete history[scheduleId];
        return { history };
      });
    }
    return removed;
  },

  runNow: (scheduleId) => act(() => ipc.scheduleRunNow(scheduleId)),

  dismissError: () => set({ error: null }),
}));

/** Do something to a schedule and show what went wrong, if anything; then read it all again. */
async function act(action: () => Promise<unknown>): Promise<boolean> {
  try {
    useSchedulesStore.setState({ error: null });
    await action();
    await useSchedulesStore.getState().refresh();
    return true;
  } catch (error) {
    useSchedulesStore.setState({ error: errorMessage(error) });
    return false;
  }
}

/** Keep what is shown of schedules current as the core fires them. Mount once. */
export function listenForSchedules(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  void useSchedulesStore.getState().load();
  return ipc.onSchedulesChanged(() => void useSchedulesStore.getState().refresh());
}

/** How many schedules need the user before they can do their job: the number on the sidebar
 *  row. One waiting for approval, one whose last time was refused, one with no workspace. */
export function needingAttention(items: Schedule[]): number {
  return items.filter((item) => needsAttention(item)).length;
}

export function needsAttention(item: Pick<Schedule, "standing">): boolean {
  return (
    item.standing === "awaiting_approval" ||
    item.standing === "refused" ||
    item.standing === "no_workspace"
  );
}

/** The schedule that started a run, by the run's id; undefined for a run nobody scheduled. */
export function scheduleOf(started: ScheduledRun[], runId: string): ScheduledRun | undefined {
  return started.find((mark) => mark.runId === runId);
}
