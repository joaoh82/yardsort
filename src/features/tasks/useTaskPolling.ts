import { useEffect } from "react";
import { hasCore } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useTasksStore } from "@/stores/tasks";

/** How often the source is asked again while the Tasks view is open. */
const POLL_MS = 60_000;

/**
 * Keep every project's tasks loaded for the Tasks view and the sidebar's count.
 *
 * They are read once at the start, so the count is there and the view opens with rows in it,
 * and after that only while the view is showing: when it opens, when the window comes back,
 * and every minute. With the view closed nobody is looking at the answers, so nothing is asked.
 * Closed tasks are a second question, asked only while the view is showing them.
 */
export function useTaskPolling() {
  // A joined string, not an array: a selector that built an array would be a new value on every
  // render and would re-render for ever.
  const ids = useProjectsStore((s) =>
    s.projects
      .filter((project) => !project.missing)
      .map((project) => project.id)
      .join(" "),
  );
  const viewing = useProjectsStore((s) => s.tasksOpen);
  const closed = useTasksStore((s) => s.closedWanted);

  useEffect(() => {
    if (!hasCore() || ids === "") return;
    for (const id of ids.split(" ")) void useTasksStore.getState().loadProject(id);
  }, [ids]);

  useEffect(() => {
    if (!viewing || !hasCore() || ids === "") return;
    const load = (refresh: boolean) => {
      const { loadProject, closedWanted } = useTasksStore.getState();
      for (const id of ids.split(" ")) void loadProject(id, refresh, closedWanted);
    };
    // Opening the view, or turning to its closed tasks, asks unless the core's copy is fresh.
    load(false);
    const onFocus = () => load(true);
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(() => load(true), POLL_MS);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, [viewing, closed, ids]);
}
