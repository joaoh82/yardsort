import { useEffect, useState } from "react";
import { errorMessage, type WorkflowItem } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { NEW_WORKFLOW, originWords, removeWorkflow, useWorkflowStore } from "@/stores/workflows";

const quiet =
  "rounded border border-line px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40";

/**
 * Every workflow, in the center panel, opened from the sidebar's **Workflows** row: the built-in
 * ones and the user's, each saying which it is, with the runs waiting on the user at the top.
 * A row opens that workflow; one of the user's can be deleted from here, and a copy that
 * replaces a built-in reset. **New workflow…** starts one.
 *
 * The list is the core's: it holds nothing but which delete is under way.
 */
export function WorkflowsView() {
  const items = useWorkflowStore((s) => s.items);
  const waits = useWorkflowStore((s) => s.waits);
  const drafts = useWorkflowStore((s) => s.drafts);
  const loaded = useWorkflowStore((s) => s.loaded);
  const storeError = useWorkflowStore((s) => s.error);
  const openWorkflow = useProjectsStore((s) => s.openWorkflow);
  const [removing, setRemoving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void useWorkflowStore.getState().load();
  }, []);

  const remove = async (item: WorkflowItem) => {
    setRemoving(item.id);
    setError(null);
    try {
      await removeWorkflow(item);
    } catch (failed) {
      setError(errorMessage(failed));
    } finally {
      setRemoving(null);
    }
  };

  const shownError = error ?? storeError;
  const custom = items.filter((item) => item.source.kind === "file").length;

  return (
    <section aria-label="Workflows" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2">
        <h1 className="text-small font-semibold tracking-wider text-ink-muted uppercase">
          Workflows
        </h1>
        {loaded && (
          <span className="text-detail text-ink-faint">
            {items.length - custom} built in · {custom} custom
          </span>
        )}
        <button
          type="button"
          onClick={() => openWorkflow(NEW_WORKFLOW)}
          className="ml-auto rounded bg-accent px-3 py-1.5 font-medium text-canvas"
        >
          New workflow…
        </button>
        <button
          type="button"
          aria-label="Close workflows"
          title="Close"
          onClick={() => openWorkflow(null)}
          className="size-7 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>

      {shownError && (
        <p
          role="alert"
          className="shrink-0 border-b border-line px-4 py-1.5 text-red-400 select-text"
        >
          {shownError}
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {waits.length > 0 && (
          <div className="border-b border-line px-4 py-2">
            <h2 className="py-1 text-micro font-semibold tracking-wider text-ink-faint uppercase">
              Waiting on you
            </h2>
            <ul aria-label="Waiting on you">
              {waits.map((wait) => (
                <li key={`${wait.runId}:${wait.stepId}`}>
                  <button
                    type="button"
                    className="flex w-full flex-col items-start py-1 text-left text-ink-muted hover:text-ink"
                    onClick={() => {
                      openWorkflow(wait.workflowId);
                      useWorkflowStore.getState().showRun(wait.workflowId, wait.runId);
                    }}
                  >
                    <span className="truncate">
                      {wait.workflowName} · {wait.workspaceName} · {wait.blockedLine}
                    </span>
                    <span className="truncate text-small text-ink-faint">{wait.actionNeeded}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}

        {items.length === 0 ? (
          <p role="status" className="p-4 text-ink-faint">
            {loaded ? "No workflows." : "Loading workflows…"}
          </p>
        ) : (
          <ul aria-label="Every workflow">
            {items.map((item) => {
              const unsaved = drafts[item.id] !== undefined && drafts[item.id] !== item.text;
              const builtIn = item.source.kind === "builtIn";
              const replaces = item.source.kind === "file" && item.source.replacesBuiltIn;
              return (
                <li key={item.id} className="flex items-center gap-2 border-b border-line pr-4">
                  <button
                    type="button"
                    onClick={() => openWorkflow(item.id)}
                    className="flex min-w-0 flex-1 flex-col gap-0.5 px-4 py-2 text-left hover:bg-raised"
                  >
                    <span className="flex items-baseline gap-2">
                      <span className="min-w-0 truncate font-medium text-ink">{item.name}</span>
                      <span
                        className={`shrink-0 rounded-full px-1.5 text-micro ${
                          builtIn ? "bg-raised text-ink-faint" : "bg-accent/15 text-accent"
                        }`}
                      >
                        {originWords(item)}
                      </span>
                      {unsaved && (
                        <span
                          title="Unsaved changes"
                          aria-label="unsaved changes"
                          className="text-accent"
                        >
                          •
                        </span>
                      )}
                      {item.problems.length > 0 && (
                        <span
                          title="It has problems and cannot run"
                          aria-label="has problems"
                          className="shrink-0 text-detail text-red-400"
                        >
                          {item.problems.length}{" "}
                          {item.problems.length === 1 ? "problem" : "problems"}
                        </span>
                      )}
                      {item.activeRuns > 0 && (
                        <span
                          title={`${item.activeRuns} running`}
                          aria-label={`${item.activeRuns} running`}
                          className="shrink-0 rounded-full border border-accent/50 px-1.5 text-micro leading-4.5 text-accent"
                        >
                          {item.activeRuns} running
                        </span>
                      )}
                    </span>
                    <span className="truncate text-detail text-ink-faint">
                      {item.description ?? item.id}
                    </span>
                  </button>
                  {!builtIn && (
                    <button
                      type="button"
                      className={`${quiet} shrink-0`}
                      disabled={removing !== null}
                      aria-label={
                        replaces ? `Reset ${item.name} to the built-in` : `Delete ${item.name}`
                      }
                      title={
                        replaces
                          ? "Delete your copy and use the built-in again"
                          : "Delete this workflow's file"
                      }
                      onClick={() => void remove(item)}
                    >
                      {replaces ? "Reset…" : "Delete…"}
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}
