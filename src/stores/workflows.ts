import { create } from "zustand";
import { native } from "@/lib/native";
import {
  errorMessage,
  hasCore,
  ipc,
  type WorkflowItem,
  type WorkflowRun,
  type WorkflowStepRun,
  type WorkflowWait,
} from "@/lib/ipc";

/** The id the view uses for a workflow not saved yet. */
export const NEW_WORKFLOW = ":new";

/** What the center panel holds for the list of every workflow, opened from the sidebar. Never a
 *  workflow's id, which is lowercase letters, digits and `-`. */
export const WORKFLOW_LIST = ":list";

/** Where a workflow comes from, in a word, for the list and the view's header. */
export function originWords(item: Pick<WorkflowItem, "source">): string {
  if (item.source.kind === "builtIn") return "Built in";
  return item.source.replacesBuiltIn ? "Custom, replaces the built-in" : "Custom";
}

interface WorkflowsState {
  /** Every workflow, built in and the user's, as the core lists them. */
  items: WorkflowItem[];
  /** Runs waiting on the person, across every workflow. */
  waits: WorkflowWait[];
  loaded: boolean;
  /** Runs of each workflow whose runs were asked for, newest first. */
  runs: Record<string, WorkflowRun[] | undefined>;
  /** Steps of each run whose steps were asked for. */
  steps: Record<string, WorkflowStepRun[] | undefined>;
  /** Text being edited and not saved, by workflow id. Kept while the app runs, so going to
   *  another workflow and back loses nothing. */
  drafts: Record<string, string | undefined>;
  /** Goes up with every change to a workflow's unsaved text, by workflow id. A generation
   *  remembers the number it started under, and its answer is kept only if nothing changed
   *  meanwhile: a slower answer never replaces a newer one, or what was typed, a save, or a
   *  discard. */
  serials: Record<string, number | undefined>;
  /** A run to show when its workflow's view next opens, by workflow id: asked for from outside
   *  the view, as Open run on a trigger's history does. The view takes it and clears it. */
  shownRun: Record<string, string | undefined>;
  error: string | null;

  load: () => Promise<void>;
  loadRuns: (workflowId: string) => Promise<void>;
  loadSteps: (runId: string) => Promise<void>;
  /** Everything already loaded, again: the list, and the runs and steps on screen. */
  refresh: () => Promise<void>;
  /** Keep `text` as `workflowId`'s unsaved text; `null` drops it (saved, or reverted). */
  setDraft: (workflowId: string, text: string | null) => void;
  /** A generation of `workflowId`'s text begins; the number its answer must present. */
  beginDescribe: (workflowId: string) => number;
  /** A generation's answer, kept as `workflowId`'s unsaved text only if `serial` is still
   *  current. Says whether it was. */
  finishDescribe: (workflowId: string, serial: number, text: string) => boolean;
  /** Show `runId` in `workflowId`'s view; `null` once the view has it. */
  showRun: (workflowId: string, runId: string | null) => void;
  dismissError: () => void;
}

export const useWorkflowStore = create<WorkflowsState>((set, get) => ({
  items: [],
  waits: [],
  loaded: false,
  runs: {},
  steps: {},
  drafts: {},
  serials: {},
  shownRun: {},
  error: null,

  async load() {
    if (!hasCore()) return;
    try {
      const [items, waits] = await Promise.all([ipc.workflowList(), ipc.workflowWaits()]);
      set({ items, waits, loaded: true });
    } catch (error) {
      set({ error: errorMessage(error), loaded: true });
    }
  },

  async loadRuns(workflowId) {
    if (!hasCore()) return;
    try {
      const runs = await ipc.workflowRuns(workflowId, 50);
      set((state) => ({ runs: { ...state.runs, [workflowId]: runs } }));
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async loadSteps(runId) {
    if (!hasCore()) return;
    try {
      const steps = await ipc.workflowRunSteps(runId);
      set((state) => ({ steps: { ...state.steps, [runId]: steps } }));
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  async refresh() {
    const { loaded, runs, steps, load, loadRuns, loadSteps } = get();
    await Promise.all([
      loaded ? load() : ipc.workflowWaits().then((waits) => set({ waits })),
      ...Object.keys(runs).map((id) => loadRuns(id)),
      ...Object.keys(steps).map((id) => loadSteps(id)),
    ]);
  },

  setDraft: (workflowId, text) =>
    set((state) => ({
      drafts: { ...state.drafts, [workflowId]: text ?? undefined },
      serials: { ...state.serials, [workflowId]: (state.serials[workflowId] ?? 0) + 1 },
    })),

  beginDescribe(workflowId) {
    const serial = (get().serials[workflowId] ?? 0) + 1;
    set((state) => ({ serials: { ...state.serials, [workflowId]: serial } }));
    return serial;
  },

  finishDescribe(workflowId, serial, text) {
    if (get().serials[workflowId] !== serial) return false;
    set((state) => ({ drafts: { ...state.drafts, [workflowId]: text } }));
    return true;
  },

  showRun: (workflowId, runId) =>
    set((state) => ({ shownRun: { ...state.shownRun, [workflowId]: runId ?? undefined } })),

  dismissError: () => set({ error: null }),
}));

/** Keep what is shown of runs current as the core moves them on. Mount once. */
export function listenForWorkflowRuns(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  return ipc.onWorkflowRunsChanged(() => void useWorkflowStore.getState().refresh());
}

/** The same text for a run's or a step's state everywhere it shows. */
export function statusColour(status: string): string {
  switch (status) {
    case "succeeded":
      return "bg-green-500/20 text-green-400";
    case "failed":
      return "bg-red-500/20 text-red-400";
    case "running":
    case "waiting":
    case "queued":
      return "bg-amber-500/20 text-amber-300";
    default:
      return "bg-raised text-ink-faint";
  }
}

/** A new workflow's starting text: every part a first workflow needs, and nothing more. */
export const TEMPLATE = `id: my-workflow
name: My workflow
description: What it is for, in a sentence.
version: 1

trigger:
  kind: manual

inputs:
  - id: agent
    kind: harness
    label: Which agent
    required: true

steps:
  - id: work
    action: start_session
    harness: "{{ inputs.agent }}"
    prompt: |
      Work on \`{{ workspace.branch }}\`.

  - id: done
    action: wait_session
    needs: [work]
    session: "{{ steps.work.session }}"
    timeout: 1h

  - id: tell_me
    action: notify
    needs: [done]
    title: "{{ workspace.name }}: done"
`;

/**
 * Delete one of the user's workflow files, after asking. A copy that replaces a built-in is
 * reset instead: the built-in is used again. A built-in has no file, and is never removed.
 * Says whether it was; throws what the core said when it could not be.
 */
export async function removeWorkflow(item: WorkflowItem): Promise<boolean> {
  if (item.source.kind !== "file") return false;
  const { path, replacesBuiltIn } = item.source;
  const sure = await native.confirm(
    replacesBuiltIn
      ? `Delete your copy of “${item.name}”? The built-in is used again. Your file is gone for good: ${path}`
      : `Delete “${item.name}”? The file is gone for good: ${path}`,
    {
      title: replacesBuiltIn ? "Reset to the built-in" : "Delete workflow",
      okLabel: replacesBuiltIn ? "Reset" : "Delete",
    },
  );
  if (!sure) return false;
  await ipc.workflowRemove(path);
  const store = useWorkflowStore.getState();
  store.setDraft(item.id, null);
  await store.load();
  return true;
}
