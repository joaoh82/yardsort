import type { TaskRef } from "@/lib/ipc";
import { showTask } from "./showTask";

/**
 * The task a workspace was started from, on its row: the key, which opens the task.
 *
 * Beside the row's own button and not inside it, like the pull request's badge: the row opens
 * the workspace, and this opens the task.
 */
export function TaskBadge({ projectId, task }: { projectId: string; task: TaskRef }) {
  return (
    <button
      type="button"
      aria-label={`Open task ${task.key} — ${task.title}`}
      title={`Started from task ${task.key}: ${task.title}`}
      onClick={() => showTask(projectId, task)}
      className="shrink-0 rounded border border-line px-1 text-[10px] leading-4 font-medium text-ink-muted tabular-nums hover:bg-raised hover:text-ink"
    >
      {task.key}
    </button>
  );
}
