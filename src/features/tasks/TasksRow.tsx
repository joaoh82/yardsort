import { useProjectsStore } from "@/stores/projects";
import { useTasksStore } from "@/stores/tasks";
import { needingAnswer } from "./rows";

/**
 * Under Pull requests in the sidebar: one row that opens every project's tasks in the center
 * panel.
 *
 * The number is how many are waiting on a maintainer, not how many are open: it is the one that
 * asks something of you, and it can reach zero. Nothing is counted without `gh`.
 */
export function TasksRow() {
  const open = useProjectsStore((s) => s.tasksOpen);
  const projects = useProjectsStore((s) => s.projects);
  // The whole record, counted outside the selector, which must hand back a stable value.
  const byProject = useTasksStore((s) => s.byProject);

  const count = projects
    .filter((p) => !p.missing)
    .reduce((sum, p) => sum + needingAnswer(byProject[p.id]), 0);

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={
        count === 0 ? "Tasks" : `Tasks, ${count} ${count === 1 ? "needs" : "need"} an answer`
      }
      title="Every project's tasks: its GitHub issues"
      onClick={() => useProjectsStore.getState().openTasks(!open)}
      className={`flex h-9 w-full shrink-0 items-center gap-2 border-b border-line px-3 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <TasksIcon />
      <span className="min-w-0 flex-1 truncate">Tasks</span>
      {count > 0 && (
        <span
          aria-hidden
          title={`${count} ${count === 1 ? "needs" : "need"} an answer`}
          className="shrink-0 rounded-full border border-amber-300/40 px-1.5 text-[10px] leading-4 text-amber-300 tabular-nums"
        >
          {count}
        </span>
      )}
    </button>
  );
}

export function TasksIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-3.5 shrink-0"
    >
      <circle cx="8" cy="8" r="5.9" />
      <circle cx="8" cy="8" r="1.4" fill="currentColor" stroke="none" />
    </svg>
  );
}
