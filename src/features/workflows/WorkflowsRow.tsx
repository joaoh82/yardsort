import { useEffect } from "react";
import { useProjectsStore } from "@/stores/projects";
import { useWorkflowStore, WORKFLOW_LIST } from "@/stores/workflows";

/**
 * Under Tasks in the sidebar: one row that opens every workflow, built in and the user's, in the
 * center panel. It stays lit while one of them is open there.
 *
 * The number is how many runs are waiting on the user — an approval or a question — across every
 * workflow. It reaches zero when nothing is waiting.
 */
export function WorkflowsRow() {
  const workflowId = useProjectsStore((s) => s.workflowId);
  const waiting = useWorkflowStore((s) => s.waits.length);
  const open = workflowId !== null;

  useEffect(() => {
    void useWorkflowStore.getState().load();
  }, []);

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={
        waiting === 0
          ? "Workflows"
          : `Workflows, ${waiting} ${waiting === 1 ? "run" : "runs"} waiting on you`
      }
      title="Every workflow, built in and yours, and their runs"
      onClick={() =>
        useProjectsStore
          .getState()
          .openWorkflow(workflowId === WORKFLOW_LIST ? null : WORKFLOW_LIST)
      }
      className={`flex h-10 w-full shrink-0 items-center gap-2 border-b border-line px-3.5 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <WorkflowsIcon />
      <span className="min-w-0 flex-1 truncate">Workflows</span>
      {waiting > 0 && (
        <span
          aria-hidden
          title={`${waiting} waiting on you`}
          className="shrink-0 rounded-full border border-accent/50 px-1.5 text-micro leading-4.5 text-accent tabular-nums"
        >
          {waiting}
        </span>
      )}
    </button>
  );
}

/** Three steps joined: the workflow mark. */
export function WorkflowsIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-4 shrink-0"
    >
      <rect x="2" y="1.8" width="5" height="3.4" rx="0.8" />
      <rect x="9" y="6.3" width="5" height="3.4" rx="0.8" />
      <rect x="2" y="10.8" width="5" height="3.4" rx="0.8" />
      <path d="M4.5 5.2v5.6M7 3.5h2.5V6.3M9 8H7.5" />
    </svg>
  );
}
