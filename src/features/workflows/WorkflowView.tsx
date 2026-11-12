import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { errorMessage, ipc, type Step, type WorkflowCheck } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import {
  NEW_WORKFLOW,
  removeWorkflow,
  TEMPLATE,
  useWorkflowStore,
  WORKFLOW_LIST,
} from "@/stores/workflows";
import { ScheduleDialog } from "@/features/schedules/ScheduleDialog";
import { DescribeWorkflow } from "./DescribeWorkflow";
import { RunWorkflowDialog } from "./RunWorkflowDialog";
import { WorkflowRuns } from "./WorkflowRuns";

// Heavy, and only needed here: loaded when a workflow is first opened.
const WorkflowEditor = lazy(() =>
  import("./WorkflowEditor").then((m) => ({ default: m.WorkflowEditor })),
);
const WorkflowChart = lazy(() =>
  import("./WorkflowChart").then((m) => ({ default: m.WorkflowChart })),
);

/** How long typing must pause before the file is checked again. */
const CHECK_AFTER_MS = 250;

const quiet =
  "rounded border border-line px-3 py-1.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40";
const primary = "rounded bg-accent px-3 py-1.5 font-medium text-canvas disabled:opacity-40";

/**
 * One workflow in the center panel: its steps as a chart, its file in an editor, and its runs.
 * A built-in is read-only until it is customized; a file of the user's is edited in place.
 */
export function WorkflowView({ workflowId }: { workflowId: string }) {
  const isNew = workflowId === NEW_WORKFLOW;
  const item = useWorkflowStore((s) => s.items.find((i) => i.id === workflowId));
  const draft = useWorkflowStore((s) => s.drafts[workflowId]);
  const loaded = useWorkflowStore((s) => s.loaded);
  const original = isNew ? TEMPLATE : (item?.text ?? "");
  const text = draft ?? original;
  const dirty = isNew || (draft !== undefined && draft !== original);
  const builtIn = item?.source.kind === "builtIn";
  const path = item?.source.kind === "file" ? item.source.path : null;
  const replacesBuiltIn = item?.source.kind === "file" && item.source.replacesBuiltIn;

  const [check, setCheck] = useState<WorkflowCheck | null>(null);
  const [steps, setSteps] = useState<Step[]>([]);
  const [checkedText, setCheckedText] = useState<string | null>(null);
  const [selection, setSelection] = useState<{ id: string | null } | null>(null);
  // A fresh request also lets a second click jump back to the same step after scrolling away.
  const selectStep = useCallback((id: string | null) => setSelection({ id }), []);
  // A run asked for from elsewhere, such as the list of runs waiting on you, is the one
  // selected. A later ask while this view is open selects that run too. The ask is then
  // cleared, so opening the workflow from the sidebar next time starts with nothing picked.
  const askedRun = useWorkflowStore((s) => s.shownRun[workflowId]) ?? null;
  const [selectedRun, setSelectedRun] = useState<string | null>(askedRun);
  const [takenAsk, setTakenAsk] = useState<string | null>(askedRun);
  if (askedRun !== takenAsk) {
    setTakenAsk(askedRun);
    if (askedRun) setSelectedRun(askedRun);
  }
  useEffect(() => {
    if (askedRun) useWorkflowStore.getState().showRun(workflowId, null);
  }, [askedRun, workflowId]);
  const runSteps = useWorkflowStore((s) => (selectedRun ? s.steps[selectedRun] : undefined));
  const [running, setRunning] = useState(false);
  const [scheduling, setScheduling] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The workflow whose "describe a change" box is open. By id, so going to another workflow
  // closes it without an effect.
  const [describing, setDescribing] = useState<string | null>(null);
  const changing = describing === workflowId;

  useEffect(() => {
    if (!loaded) void useWorkflowStore.getState().load();
  }, [loaded]);

  useEffect(() => {
    let current = true;
    const timer = window.setTimeout(() => {
      void ipc.workflowCheck(text).then((found) => {
        if (!current) return;
        setCheck(found);
        setCheckedText(text);
        // The chart keeps the last shape that checked out while the file is mid-edit.
        if (found.workflow) setSteps(found.workflow.steps);
      }, console.error);
    }, CHECK_AFTER_MS);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [text]);

  if (!isNew && !item) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-ink-faint">
        {loaded ? "That workflow is not there any more." : "Loading…"}
        {loaded && (
          <button
            type="button"
            className={quiet}
            onClick={() => useProjectsStore.getState().openWorkflow(WORKFLOW_LIST)}
          >
            Every workflow
          </button>
        )}
      </div>
    );
  }

  const problems = check?.problems ?? [];
  const name = check?.workflow?.name ?? item?.name ?? "New workflow";
  const setDraft = (next: string) => useWorkflowStore.getState().setDraft(workflowId, next);

  async function act(what: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await what();
    } catch (failed) {
      setError(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  const save = () =>
    act(async () => {
      // The editor stays open while the file is written. Anything typed meanwhile is newer
      // than what was saved, and stays the draft — under the new id, when saving gave it one.
      const submitted = text;
      const id = await ipc.workflowSave(path, submitted);
      const store = useWorkflowStore.getState();
      const latest = store.drafts[workflowId];
      const typedMeanwhile = latest !== undefined && latest !== submitted;
      if (!typedMeanwhile) {
        store.setDraft(workflowId, null);
      } else if (id !== workflowId) {
        store.setDraft(id, latest);
        store.setDraft(workflowId, null);
      }
      await store.load();
      if (id !== workflowId) useProjectsStore.getState().openWorkflow(id);
    });

  const revert = () => useWorkflowStore.getState().setDraft(workflowId, null);

  // The copy is the person's file at once, and the box for saying how to change it opens on it.
  const customize = () =>
    act(async () => {
      await ipc.workflowCopy(workflowId);
      await useWorkflowStore.getState().load();
      setDescribing(workflowId);
    });

  const duplicate = () =>
    act(async () => {
      const taken = new Set(useWorkflowStore.getState().items.map((i) => i.id));
      let copy = `${workflowId}-copy`;
      for (let n = 2; taken.has(copy); n++) copy = `${workflowId}-copy-${n}`;
      const id = await ipc.workflowCopy(workflowId, copy);
      await useWorkflowStore.getState().load();
      useProjectsStore.getState().openWorkflow(id);
    });

  const remove = () =>
    act(async () => {
      if (!item) return;
      const removed = await removeWorkflow(item);
      // A reset copy leaves the built-in under the same id, which stays open.
      if (removed && !replacesBuiltIn) useProjectsStore.getState().openWorkflow(WORKFLOW_LIST);
    });

  const discard = () => {
    useWorkflowStore.getState().setDraft(workflowId, null);
    useProjectsStore.getState().openWorkflow(WORKFLOW_LIST);
  };

  // One about a pull request elsewhere is started from that pull request, never from here.
  const elsewhere = item?.workflow?.trigger.context === "tracked_pull_request";
  const runnable = !isNew && !dirty && problems.length === 0 && !!item?.workflow && !elsewhere;
  const fromThePullRequest =
    "Run it from a pull request a workspace tracks in another repository, in the Pull requests view";

  return (
    <section aria-label={`Workflow ${name}`} className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-start gap-3 border-b border-line px-4 py-3">
        <BackToList />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-base font-semibold">{name}</h2>
          <p className="truncate text-small text-ink-faint">
            {isNew
              ? "Not saved yet"
              : builtIn
                ? "Built in — customize it to change it"
                : replacesBuiltIn
                  ? `Custom: your copy, used instead of the built-in — ${path}`
                  : `Custom — ${path}`}
            {problems.length > 0 &&
              ` · ${problems.length} ${problems.length === 1 ? "problem" : "problems"}`}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          {builtIn ? (
            <>
              <button type="button" className={quiet} disabled={busy} onClick={customize}>
                Customize
              </button>
              <button type="button" className={quiet} disabled={busy} onClick={duplicate}>
                Duplicate
              </button>
            </>
          ) : isNew ? (
            <>
              <button type="button" className={quiet} disabled={busy} onClick={discard}>
                Discard
              </button>
              <button type="button" className={primary} disabled={busy} onClick={save}>
                Save
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className={quiet}
                aria-pressed={changing}
                title="Say what to change, and have an agent change it"
                onClick={() => setDescribing(changing ? null : workflowId)}
              >
                Describe a change
              </button>
              <button type="button" className={quiet} disabled={busy} onClick={remove}>
                {replacesBuiltIn ? "Reset to built-in" : "Delete"}
              </button>
              <button type="button" className={quiet} disabled={busy || dirty} onClick={duplicate}>
                Duplicate
              </button>
              {dirty && (
                <button type="button" className={quiet} disabled={busy} onClick={revert}>
                  Revert
                </button>
              )}
              <button type="button" className={primary} disabled={busy || !dirty} onClick={save}>
                Save
              </button>
            </>
          )}
          {!isNew && (
            <button
              type="button"
              className={quiet}
              disabled={!runnable}
              title={
                dirty
                  ? "Save first"
                  : problems.length
                    ? "Fix its problems first"
                    : elsewhere
                      ? fromThePullRequest
                      : "Run it at a set time, or again and again"
              }
              onClick={() => setScheduling(true)}
            >
              Schedule…
            </button>
          )}
          {!isNew && (
            <button
              type="button"
              className={primary}
              disabled={!runnable}
              title={
                dirty
                  ? "Save first"
                  : problems.length
                    ? "Fix its problems first"
                    : elsewhere
                      ? fromThePullRequest
                      : undefined
              }
              onClick={() => setRunning(true)}
            >
              Run…
            </button>
          )}
        </div>
      </header>
      {error && (
        <div
          role="alert"
          className="flex items-start gap-3 border-b border-line bg-raised px-4 py-2"
        >
          <p className="flex-1 text-red-400 select-text">{error}</p>
          <button
            type="button"
            className="text-ink-muted hover:text-ink"
            onClick={() => setError(null)}
          >
            Dismiss
          </button>
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="h-[42%] min-h-40 shrink-0 border-b border-line">
            <Suspense fallback={null}>
              {steps.length > 0 && (
                <WorkflowChart
                  steps={steps}
                  run={runSteps}
                  selectedStep={selection?.id ?? null}
                  onSelect={selectStep}
                />
              )}
            </Suspense>
          </div>
          {isNew && <DescribeWorkflow />}
          {changing && !isNew && !builtIn && (
            <DescribeWorkflow key={workflowId} workflowId={workflowId} current={text} />
          )}
          <div className="min-h-0 flex-1">
            <Suspense fallback={null}>
              <WorkflowEditor
                key={`${workflowId}:${builtIn ? "built-in" : (path ?? "new")}`}
                text={text}
                onChange={builtIn ? undefined : setDraft}
                problems={problems}
                readOnly={builtIn}
                selection={selection}
                stepLines={checkedText === text ? check?.stepLines : undefined}
              />
            </Suspense>
          </div>
          {problems.length > 0 && (
            <ul
              aria-label="Problems"
              className="max-h-28 shrink-0 overflow-y-auto border-t border-line px-4 py-2 text-detail"
            >
              {problems.map((problem, i) => (
                <li key={i} className="text-red-400 select-text">
                  {problem.line ? `${problem.line}:${problem.column ?? 1} ` : ""}
                  {problem.message}
                </li>
              ))}
            </ul>
          )}
        </div>
        {!isNew && (
          <aside className="flex w-80 shrink-0 flex-col border-l border-line">
            <WorkflowRuns
              workflowId={workflowId}
              selected={selectedRun}
              onSelect={setSelectedRun}
            />
          </aside>
        )}
      </div>

      {scheduling && (
        <ScheduleDialog
          target="workflow"
          targetId={workflowId}
          onClose={() => setScheduling(false)}
        />
      )}
      {running && (
        <RunWorkflowDialog
          workflowId={workflowId}
          onClose={() => setRunning(false)}
          onStarted={(run, started) => {
            void useWorkflowStore.getState().loadRuns(started);
            if (started === workflowId) {
              setSelectedRun(run);
            } else {
              // Run as its twin on a pull request elsewhere: its runs are on that one's page.
              useWorkflowStore.getState().showRun(started, run);
              useProjectsStore.getState().openWorkflow(started);
            }
          }}
        />
      )}
    </section>
  );
}

/** Back to the list of every workflow, which the sidebar's **Workflows** row opens. */
function BackToList() {
  return (
    <button
      type="button"
      aria-label="Back to every workflow"
      title="Every workflow"
      onClick={() => useProjectsStore.getState().openWorkflow(WORKFLOW_LIST)}
      className="-ml-1 size-7 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink"
    >
      ←
    </button>
  );
}
