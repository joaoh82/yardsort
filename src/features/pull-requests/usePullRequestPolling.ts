import { useEffect } from "react";
import { hasCore } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";

/**
 * How often the forge is asked again what became of each project's pull requests, while the
 * window is open. A check finishing is the thing worth noticing, and it takes minutes, not
 * seconds; the core also holds each answer briefly, so switching workspaces costs nothing.
 */
const POLL_MS = 60_000;

/**
 * Keep every project's pull requests loaded, so each workspace row can show its own and the
 * Pull requests view has its list.
 *
 * One request per project rather than one per workspace — see `crate::publish`. Coming back to
 * the window asks again, because that is when something has usually moved.
 *
 * *Every* open pull request is read once at the start, so the sidebar's count is right and the
 * view opens with rows in it, and after that only while the view is showing: on a busy
 * repository it is several questions to the forge, and with the view closed nobody is looking at
 * the answers. The newest fifty are what every other poll asks for, as before.
 */
export function usePullRequestPolling() {
  // A joined string, not an array: a selector that built an array would be a new value on every
  // render and would re-render for ever.
  const ids = useProjectsStore((s) =>
    s.projects
      .filter((project) => !project.missing)
      .map((project) => project.id)
      .join(" "),
  );
  const viewing = useProjectsStore((s) => s.pullRequestsOpen);

  useEffect(() => {
    if (!hasCore() || ids === "") return;
    const load = (refresh: boolean, full: boolean) => {
      for (const id of ids.split(" ")) {
        void usePublishStore.getState().loadProject(id, refresh, full);
      }
    };
    const looking = () => useProjectsStore.getState().pullRequestsOpen;
    load(false, true);
    const onFocus = () => load(true, looking());
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(() => load(true, looking()), POLL_MS);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, [ids]);

  // Opening the view asks for the whole list, unless the core's copy is still fresh.
  useEffect(() => {
    if (!viewing || !hasCore() || ids === "") return;
    for (const id of ids.split(" ")) void usePublishStore.getState().loadProject(id, false, true);
  }, [viewing, ids]);
}
