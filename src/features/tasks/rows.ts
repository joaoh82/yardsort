import type { Project, ProjectTasks, Task, Workspace } from "@/lib/ipc";

/** A task in the list: the task, and the project it belongs to. */
export interface Row {
  /** Unique across projects: two repositories both have a #1. */
  key: string;
  project: Project;
  task: Task;
  /** Who the source says is asking: what "me" means for this row. */
  viewer: string | null;
  /** The workspaces started from this task that there is somewhere to go to, oldest first. */
  workspaces: Workspace[];
}

/** Whether a workspace was started from this task. By link: a key alone is not unique. */
export const startedFrom = (workspace: Workspace, task: Pick<Task, "url">): boolean =>
  workspace.tasks.some((from) => from.url.toLowerCase() === task.url.toLowerCase());

export const rowKey = (projectId: string, taskKey: string) => `${projectId}${taskKey}`;

/** A time the source wrote, in milliseconds. One it did not write sorts last and shows no age. */
export function at(time: string): number | null {
  const parsed = Date.parse(time);
  return Number.isNaN(parsed) ? null : parsed;
}

/**
 * Every project's tasks as one list, most recently changed first.
 *
 * Derived, never stored: the core's answer per project is the only copy.
 */
export function rowsOf(projects: Project[], byProject: Record<string, ProjectTasks>): Row[] {
  const rows: Row[] = [];
  for (const project of projects) {
    const found = byProject[project.id];
    if (project.missing || !found) continue;
    // A folder that is gone is not somewhere to go.
    const there = project.workspaces.filter(
      (workspace) => !workspace.archived && !workspace.missing,
    );
    for (const task of found.tasks) {
      rows.push({
        key: rowKey(project.id, task.key),
        project,
        task,
        viewer: found.viewer,
        workspaces: there.filter((workspace) => startedFrom(workspace, task)),
      });
    }
  }
  const number = (row: Row) => Number(row.task.key.replace(/\D/g, "")) || 0;
  return rows.sort(
    (a, b) => (at(b.task.updatedAt) ?? 0) - (at(a.task.updatedAt) ?? 0) || number(b) - number(a),
  );
}

/** How many open tasks a project's list holds. */
export const openIn = (found: ProjectTasks | undefined): number =>
  found?.tasks.filter((task) => task.state === "open").length ?? 0;

/** How many of a project's tasks are waiting on a maintainer: the sidebar's number. */
export const needingAnswer = (found: ProjectTasks | undefined): number =>
  found?.tasks.filter((task) => task.needsAnswer).length ?? 0;

/** The source has more open tasks than the list holds: it stops at a cap. */
export const moreOpenThanListed = (found: ProjectTasks | undefined): boolean =>
  !!found && found.openTotal !== null && found.openTotal > openIn(found);

/** Open, Closed, or why it was closed when that is not "done". */
export function stateLabel(task: Task): string {
  if (task.state === "open") return "Open";
  if (task.closedAs === "notPlanned") return "Not planned";
  if (task.closedAs === "duplicate") return "Duplicate";
  return "Closed";
}

/** Green while open, violet when done, grey when set aside: GitHub's own three. */
export function stateColour(task: Task): string {
  if (task.state === "open") return "bg-green-500/20 text-green-400";
  if (task.closedAs === "notPlanned" || task.closedAs === "duplicate")
    return "bg-raised text-ink-muted";
  return "bg-violet-500/20 text-violet-300";
}
