import { create } from "zustand";
import {
  errorMessage,
  hasCore,
  ipc,
  type WorkflowItem,
  type WorkflowRun,
  type WorkflowStepRun,
} from "@/lib/ipc";

/** The id the view uses for a workflow not saved yet. */
export const NEW_WORKFLOW = ":new";

interface WorkflowsState {
  /** Every workflow, built in and the user's, as the core lists them. */
  items: WorkflowItem[];
  loaded: boolean;
  /** Runs of each workflow whose runs were asked for, newest first. */
  runs: Record<string, WorkflowRun[] | undefined>;
  /** Steps of each run whose steps were asked for. */
  steps: Record<string, WorkflowStepRun[] | undefined>;
  /** Text being edited and not saved, by workflow id. Kept while the app runs, so going to
   *  another workflow and back loses nothing. */
  drafts: Record<string, string | undefined>;
  /** Goes up with every change to the new workflow's text. A generation remembers the number
   *  it started under, and its answer is kept only if nothing changed meanwhile: a slower answer
   *  never replaces a newer one, or what was typed, or a discard. */
  newSerial: number;
  error: string | null;

  load: () => Promise<void>;
  loadRuns: (workflowId: string) => Promise<void>;
  loadSteps: (runId: string) => Promise<void>;
  /** Everything already loaded, again: the list, and the runs and steps on screen. */
  refresh: () => Promise<void>;
  /** Keep `text` as `workflowId`'s unsaved text; `null` drops it (saved, or reverted). */
  setDraft: (workflowId: string, text: string | null) => void;
  /** A generation of the new workflow begins; the number its answer must present. */
  beginDescribe: () => number;
  /** A generation's answer, kept only if `serial` is still current. Says whether it was. */
  finishDescribe: (serial: number, text: string) => boolean;
  dismissError: () => void;
}

export const useWorkflowStore = create<WorkflowsState>((set, get) => ({
  items: [],
  loaded: false,
  runs: {},
  steps: {},
  drafts: {},
  newSerial: 0,
  error: null,

  async load() {
    if (!hasCore()) return;
    try {
      set({ items: await ipc.workflowList(), loaded: true });
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
      loaded ? load() : Promise.resolve(),
      ...Object.keys(runs).map((id) => loadRuns(id)),
      ...Object.keys(steps).map((id) => loadSteps(id)),
    ]);
  },

  setDraft: (workflowId, text) =>
    set((state) => ({
      drafts: { ...state.drafts, [workflowId]: text ?? undefined },
      newSerial: workflowId === NEW_WORKFLOW ? state.newSerial + 1 : state.newSerial,
    })),

  beginDescribe() {
    const serial = get().newSerial + 1;
    set({ newSerial: serial });
    return serial;
  },

  finishDescribe(serial, text) {
    if (get().newSerial !== serial) return false;
    set((state) => ({ drafts: { ...state.drafts, [NEW_WORKFLOW]: text } }));
    return true;
  },

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
