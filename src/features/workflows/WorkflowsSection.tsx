import { useEffect } from "react";
import { PanelHeader } from "@/features/shell/PanelHeader";
import { useProjectsStore } from "@/stores/projects";
import { NEW_WORKFLOW, useWorkflowStore } from "@/stores/workflows";

/**
 * The top of the sidebar: every workflow, built in and the user's. A row opens the workflow in
 * the center panel; **+** starts a new one. A count on a row is its runs going now.
 */
export function WorkflowsSection() {
  const items = useWorkflowStore((s) => s.items);
  const waits = useWorkflowStore((s) => s.waits);
  const drafts = useWorkflowStore((s) => s.drafts);
  const open = useProjectsStore((s) => s.workflowId);
  const openWorkflow = useProjectsStore((s) => s.openWorkflow);

  useEffect(() => {
    void useWorkflowStore.getState().load();
  }, []);

  return (
    <section
      aria-label="Workflows"
      className="flex max-h-[35%] shrink-0 flex-col border-b border-line"
    >
      <PanelHeader title="Workflows">
        <button
          type="button"
          aria-label="New workflow"
          title="New workflow"
          onClick={() => openWorkflow(NEW_WORKFLOW)}
          className="size-7 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          +
        </button>
      </PanelHeader>
      {waits.length > 0 && (
        <div className="border-b border-line px-3 py-1">
          <h3 className="py-1 text-micro font-semibold tracking-wider text-ink-faint uppercase">
            Waiting on you
          </h3>
          <ul>
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
                    {wait.workflowName} · {wait.blockedLine}
                  </span>
                  <span className="truncate text-small text-ink-faint">{wait.actionNeeded}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
      <ul className="min-h-0 overflow-y-auto py-1">
        {open === NEW_WORKFLOW && (
          <li className="flex h-8 items-center bg-raised px-3.5 text-ink-muted italic">
            New workflow
          </li>
        )}
        {items.map((item) => {
          const selected = open === item.id;
          const unsaved = drafts[item.id] !== undefined && drafts[item.id] !== item.text;
          return (
            <li key={item.id}>
              <button
                type="button"
                aria-current={selected ? "page" : undefined}
                onClick={() => openWorkflow(item.id)}
                className={`flex h-8 w-full items-center gap-2 px-3.5 text-left ${
                  selected ? "bg-raised text-ink" : "text-ink-muted hover:bg-raised"
                }`}
              >
                <span className="min-w-0 flex-1 truncate">{item.name}</span>
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
                    className="text-red-400"
                  >
                    !
                  </span>
                )}
                {item.activeRuns > 0 && (
                  <span
                    title={`${item.activeRuns} running`}
                    aria-label={`${item.activeRuns} running`}
                    className="shrink-0 rounded-full border border-accent/50 px-1.5 text-micro leading-4.5 text-accent"
                  >
                    {item.activeRuns}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
