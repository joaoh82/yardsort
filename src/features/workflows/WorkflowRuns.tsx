import { useEffect, useState } from "react";
import { timeAgo } from "@/features/terminal/sessionText";
import { errorMessage, ipc, type WorkflowRun } from "@/lib/ipc";
import { statusColour, useWorkflowStore } from "@/stores/workflows";
import { ACTION_WORDS } from "./words";

const EMPTY: WorkflowRun[] = [];

interface Props {
  workflowId: string;
  /** The run whose steps the chart shows. */
  selected: string | null;
  onSelect: (runId: string | null) => void;
}

/** A workflow's runs, newest first; one open shows its steps, and an active one can be stopped. */
export function WorkflowRuns({ workflowId, selected, onSelect }: Props) {
  const runs = useWorkflowStore((s) => s.runs[workflowId]) ?? EMPTY;
  const steps = useWorkflowStore((s) => (selected ? s.steps[selected] : undefined));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void useWorkflowStore.getState().loadRuns(workflowId);
  }, [workflowId]);

  useEffect(() => {
    if (selected) void useWorkflowStore.getState().loadSteps(selected);
  }, [selected]);

  async function cancel(run: WorkflowRun) {
    setError(null);
    try {
      await ipc.workflowCancel(run.id);
      await useWorkflowStore.getState().refresh();
    } catch (failed) {
      setError(errorMessage(failed));
    }
  }

  return (
    <section aria-label="Runs" className="flex min-h-0 flex-1 flex-col">
      <h3 className="shrink-0 px-3 pt-3 pb-2 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
        Runs
      </h3>
      {error && (
        <p role="alert" className="px-3 pb-2 text-red-400 select-text">
          {error}
        </p>
      )}
      {runs.length === 0 ? (
        <p className="px-3 text-ink-faint">No runs yet.</p>
      ) : (
        <ul className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {runs.map((run) => {
            const open = run.id === selected;
            const active = run.status === "queued" || run.status === "running";
            return (
              <li key={run.id} className="mb-1 rounded border border-line bg-canvas/40">
                <button
                  type="button"
                  aria-expanded={open}
                  aria-label={`Run in ${run.workspaceName}, ${run.status}`}
                  className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left hover:bg-raised"
                  onClick={() => onSelect(open ? null : run.id)}
                >
                  <span
                    className={`shrink-0 rounded-full px-1.5 text-[10px] ${statusColour(run.status)}`}
                  >
                    {run.status}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{run.workspaceName}</span>
                  <span className="shrink-0 text-[11px] text-ink-faint">
                    {timeAgo(run.createdAt)}
                  </span>
                </button>
                {open && (
                  <div className="border-t border-line px-2.5 py-2">
                    {run.pullRequest && (
                      <p className="mb-1.5 text-ink-muted">
                        Pull request #{run.pullRequest.number} {run.pullRequest.title}
                      </p>
                    )}
                    {run.error && <p className="mb-1.5 text-red-400 select-text">{run.error}</p>}
                    <ol className="flex flex-col gap-1">
                      {(steps ?? []).map((step) => (
                        <li key={step.stepId} className="flex flex-col">
                          <span className="flex items-center gap-2">
                            <span
                              className={`shrink-0 rounded-full px-1.5 text-[10px] ${statusColour(step.status)}`}
                            >
                              {step.status}
                            </span>
                            <span className="truncate">
                              {ACTION_WORDS[step.action as keyof typeof ACTION_WORDS] ??
                                step.action}
                            </span>
                            <span className="truncate font-mono text-[11px] text-ink-faint">
                              {step.stepId}
                            </span>
                          </span>
                          {step.note && (
                            <span className="pl-1 text-[11px] text-ink-muted select-text">
                              {step.note}
                            </span>
                          )}
                        </li>
                      ))}
                    </ol>
                    {active && (
                      <button
                        type="button"
                        className="mt-2 rounded border border-line px-2 py-0.5 text-[11px] text-ink-muted hover:border-accent hover:text-ink"
                        onClick={() => void cancel(run)}
                      >
                        Cancel run
                      </button>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
