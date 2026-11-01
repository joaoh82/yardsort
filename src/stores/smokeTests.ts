import { create } from "zustand";
import {
  errorMessage,
  hasCore,
  ipc,
  type TestListing,
  type TestRun,
  type TestStepRun,
} from "@/lib/ipc";

/** How many runs the window keeps in view, newest first. The Tests view pages further. */
const RUNS_IN_VIEW = 200;

interface SmokeTestsState {
  /** Runs across every workspace, newest first, as the core lists them. */
  runs: TestRun[];
  loaded: boolean;
  /** Every test across every workspace, as the Tests view lists them. Read on opening the view
   *  and on Refresh — a scan of each worktree, so not on every run event. */
  catalog: TestListing[];
  catalogLoaded: boolean;
  /** Steps of each run whose steps were asked for. */
  steps: Record<string, TestStepRun[] | undefined>;
  /** Run tabs open in each workspace, in the order they were opened. */
  open: Record<string, string[] | undefined>;
  /** The run tab in front in each workspace; none means its terminals are. */
  shown: Record<string, string | undefined>;
  error: string | null;

  load: () => Promise<void>;
  /** Read every test across every workspace, for the Tests view. */
  loadCatalog: () => Promise<void>;
  loadSteps: (runId: string) => Promise<void>;
  /** Everything already loaded, again: the runs, and the steps on screen. */
  refresh: () => Promise<void>;
  /** Open a run's tab in its workspace and bring it to the front. */
  openRun: (workspaceId: string, runId: string) => void;
  /** Close a run's tab. The run itself goes on; the tab is a view of rows and files. */
  closeRun: (workspaceId: string, runId: string) => void;
  /** Bring the workspace's terminals back in front of its run tabs. */
  showTerminals: (workspaceId: string) => void;
  approve: (runId: string) => Promise<boolean>;
  refuse: (runId: string) => Promise<boolean>;
  cancel: (runId: string) => Promise<boolean>;
  answer: (runId: string, stepId: string, passed: boolean, note: string | null) => Promise<boolean>;
  dismissError: () => void;
}

export const useSmokeTestsStore = create<SmokeTestsState>((set, get) => ({
  runs: [],
  loaded: false,
  catalog: [],
  catalogLoaded: false,
  steps: {},
  open: {},
  shown: {},
  error: null,

  async load() {
    if (!hasCore()) return;
    try {
      // History is capped, but a run waiting for approval or still going must never be crowded
      // out of the list by newer finished runs — nor must a run an open tab points at. So the
      // capped history is merged with every active run, and with the open tabs' runs by id.
      const openIds = new Set(Object.values(get().open).flatMap((ids) => ids ?? []));
      const [history, active] = await Promise.all([
        ipc.testRuns(null, RUNS_IN_VIEW),
        ipc.testActiveRuns(),
      ]);
      const byId = new Map<string, TestRun>();
      for (const run of [...history, ...active]) byId.set(run.id, run);
      const missing = [...openIds].filter((id) => !byId.has(id));
      if (missing.length > 0) {
        for (const run of await ipc.testRunsById(missing)) byId.set(run.id, run);
      }
      const runs = [...byId.values()].sort((a, b) => b.createdAt - a.createdAt);
      set({ runs, loaded: true });
    } catch (error) {
      set({ error: errorMessage(error), loaded: true });
    }
  },

  async loadCatalog() {
    if (!hasCore()) return;
    try {
      const catalog = await ipc.testCatalog();
      set({ catalog, catalogLoaded: true });
    } catch (error) {
      set({ error: errorMessage(error), catalogLoaded: true });
    }
  },

  async loadSteps(runId) {
    if (!hasCore()) return;
    try {
      const steps = await ipc.testRunSteps(runId);
      set((state) => ({ steps: { ...state.steps, [runId]: steps } }));
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async refresh() {
    const { loaded, steps, load, loadSteps } = get();
    await Promise.all([
      loaded ? load() : Promise.resolve(),
      ...Object.keys(steps).map((id) => loadSteps(id)),
    ]);
  },

  openRun: (workspaceId, runId) =>
    set((state) => {
      const open = state.open[workspaceId] ?? [];
      return {
        open: { ...state.open, [workspaceId]: open.includes(runId) ? open : [...open, runId] },
        shown: { ...state.shown, [workspaceId]: runId },
      };
    }),

  closeRun: (workspaceId, runId) =>
    set((state) => {
      const open = (state.open[workspaceId] ?? []).filter((id) => id !== runId);
      const shown = { ...state.shown };
      if (shown[workspaceId] === runId) shown[workspaceId] = undefined;
      const steps = { ...state.steps };
      // Nothing else shows this run's steps once its tab is gone: stop refreshing them.
      const elsewhere = Object.entries(state.open).some(
        ([other, ids]) => other !== workspaceId && ids?.includes(runId),
      );
      if (!elsewhere) delete steps[runId];
      return { open: { ...state.open, [workspaceId]: open }, shown, steps };
    }),

  showTerminals: (workspaceId) =>
    set((state) => ({ shown: { ...state.shown, [workspaceId]: undefined } })),

  approve: (runId) => act(() => ipc.testRunApprove(runId)),
  refuse: (runId) => act(() => ipc.testRunRefuse(runId)),
  cancel: (runId) => act(() => ipc.testRunCancel(runId)),
  answer: (runId, stepId, passed, note) =>
    act(() => ipc.testStepAnswer(runId, stepId, passed, note)),

  dismissError: () => set({ error: null }),
}));

/** Do something to a run and show what went wrong, if anything; the core's events bring the
 *  new state back. */
async function act(action: () => Promise<boolean>): Promise<boolean> {
  try {
    useSmokeTestsStore.setState({ error: null });
    const done = await action();
    await useSmokeTestsStore.getState().refresh();
    return done;
  } catch (error) {
    useSmokeTestsStore.setState({ error: errorMessage(error) });
    return false;
  }
}

/** Keep what is shown of runs current as the core moves them on. Mount once. */
export function listenForTestRuns(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  void useSmokeTestsStore.getState().load();
  return ipc.onTestRunsChanged(() => void useSmokeTestsStore.getState().refresh());
}

/** The runs waiting for the user in a workspace, oldest first: what the approval card shows. */
export function waitingIn(runs: TestRun[], workspaceId: string): TestRun[] {
  return runs
    .filter((run) => run.workspaceId === workspaceId && run.status === "awaiting_approval")
    .sort((a, b) => a.createdAt - b.createdAt);
}

/** How many runs, across every workspace, are waiting for the user to approve them: the number
 *  on the Tests sidebar row. */
export function waitingTotal(runs: TestRun[]): number {
  return runs.filter((run) => run.status === "awaiting_approval").length;
}

/** Not ended yet: waiting for approval, queued or running. */
export function isActive(status: string): boolean {
  return status === "awaiting_approval" || status === "queued" || status === "running";
}
