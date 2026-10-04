import { openUrl } from "@tauri-apps/plugin-opener";
import type { TaskRef } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useTasksStore } from "@/stores/tasks";
import { rowKey } from "./rows";

/**
 * Show a task a workspace was started from: the Tasks view, with that task open. A task the
 * view has no row for — closed since, or past what is read — is opened on its source instead,
 * rather than opening a view with nothing selected in it.
 */
export function showTask(projectId: string, task: TaskRef) {
  const listed = useTasksStore
    .getState()
    .byProject[projectId]?.tasks.some((it) => it.url.toLowerCase() === task.url.toLowerCase());
  if (!listed) {
    void openUrl(task.url).catch(console.error);
    return;
  }
  useProjectsStore.getState().openTasks(true);
  useTasksStore.getState().select(rowKey(projectId, task.key));
}
