import { useEffect, useState } from "react";
import { PanelHeader } from "@/features/shell/PanelHeader";
import { hasCore } from "@/lib/ipc";
import { formatShortcut } from "@/lib/platform";
import { useLayoutStore } from "@/stores/layout";
import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { useUpdatesStore } from "@/stores/updates";
import { reviewVanishedWorkspaces } from "./actions";
import { AddProjectDialog } from "./AddProjectDialog";
import { ProjectTree } from "./ProjectTree";

/**
 * How often the forge is asked again what became of each project's pull requests, while the
 * window is open. A check finishing is the thing worth noticing, and it takes minutes, not
 * seconds; the core also holds each answer briefly, so switching workspaces costs nothing.
 */
const POLL_MS = 60_000;

/**
 * Keep every project's pull requests loaded, so each workspace row can show its own.
 *
 * One request per project rather than one per workspace — see `crate::publish`. Coming back to
 * the window asks again, because that is when something has usually moved.
 */
function usePullRequests() {
  // A joined string, not an array: a selector that built an array would be a new value on every
  // render and would re-render for ever.
  const ids = useProjectsStore((s) =>
    s.projects
      .filter((project) => !project.missing)
      .map((project) => project.id)
      .join(" "),
  );

  useEffect(() => {
    if (!hasCore() || ids === "") return;
    const load = (refresh: boolean) => {
      for (const id of ids.split(" ")) void usePublishStore.getState().loadProject(id, refresh);
    };
    load(false);
    const onFocus = () => load(true);
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(() => load(true), POLL_MS);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, [ids]);
}

/** Left panel: projects and their workspaces. */
export function Sidebar() {
  const loaded = useProjectsStore((s) => s.loaded);
  const empty = useProjectsStore((s) => s.projects.length === 0);
  const error = useProjectsStore((s) => s.error);
  const notice = useProjectsStore((s) => s.notice);
  const dismiss = useProjectsStore((s) => s.dismiss);
  const [adding, setAdding] = useState(false);
  usePullRequests();

  useEffect(() => {
    if (!hasCore()) return;
    const { load, refresh } = useProjectsStore.getState();
    // Coming back is also when a worktree someone removed with git is noticed, so every catch-up
    // ends by asking about the workspaces that were left with nothing behind them.
    void load().then(reviewVanishedWorkspaces);
    // Branches get switched in other tools; catch up whenever the window comes back.
    const onFocus = () => void refresh().then(reviewVanishedWorkspaces);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  return (
    <aside aria-label="Projects" className="flex h-full flex-col bg-surface">
      <PanelHeader title="Projects">
        <button
          type="button"
          aria-label="Add project"
          title={`Add project (open a folder: ${formatShortcut("O")})`}
          onClick={() => setAdding(true)}
          className="size-6 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          +
        </button>
      </PanelHeader>

      {loaded && empty ? (
        <div className="p-3 text-ink-faint">
          <p>No projects yet.</p>
          <button
            type="button"
            onClick={() => setAdding(true)}
            className="mt-2 rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink"
          >
            Add a project
          </button>
        </div>
      ) : (
        <ProjectTree />
      )}

      {/* While the dialog is open it shows errors itself. */}
      {!adding && (error ?? notice) && (
        <div
          role={error ? "alert" : "status"}
          className="flex items-start gap-2 border-t border-line p-3"
        >
          <p
            className={`min-w-0 flex-1 break-words select-text ${error ? "text-red-400" : "text-ink-muted"}`}
          >
            {error ?? notice}
          </p>
          <button
            type="button"
            aria-label="Dismiss"
            onClick={dismiss}
            className="text-ink-faint hover:text-ink"
          >
            ×
          </button>
        </div>
      )}
      <div className="flex items-center gap-1 border-t border-line p-1">
        <button
          type="button"
          title={`Settings (${formatShortcut(",")})`}
          onClick={() => useLayoutStore.getState().setSettingsOpen(true)}
          className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded px-2 text-ink-muted hover:bg-raised hover:text-ink"
        >
          <span aria-hidden>⚙</span> Settings
        </button>
        <UpdatePill />
      </div>
      {adding && <AddProjectDialog onClose={() => setAdding(false)} />}
    </aside>
  );
}

/**
 * Where a waiting update announces itself: beside Settings, the one control that is always on
 * screen and never moves. It appears only once a check has found something, and pressing it
 * opens the dialog that explains what the update costs before anything is downloaded.
 */
function UpdatePill() {
  const update = useUpdatesStore((s) => s.status?.available);
  if (!update) return null;

  return (
    <button
      type="button"
      aria-label={`Update to ${update.version}`}
      title={`Yardsort ${update.version} is available`}
      onClick={() => useUpdatesStore.getState().show(true)}
      className="flex h-7 shrink-0 items-center gap-1.5 rounded-full bg-accent/15 px-2.5 text-[11px] font-medium text-accent hover:bg-accent/25"
    >
      <span aria-hidden className="size-1.5 rounded-full bg-accent" />
      update
    </button>
  );
}
