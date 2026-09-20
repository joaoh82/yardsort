import { useEffect, useState } from "react";
import { PanelHeader } from "@/features/shell/PanelHeader";
import { hasCore } from "@/lib/ipc";
import { formatShortcut } from "@/lib/platform";
import { useLayoutStore } from "@/stores/layout";
import { useProjectsStore } from "@/stores/projects";
import { reviewVanishedWorkspaces } from "./actions";
import { AddProjectDialog } from "./AddProjectDialog";
import { ProjectTree } from "./ProjectTree";

/** Left panel: projects and their workspaces. */
export function Sidebar() {
  const loaded = useProjectsStore((s) => s.loaded);
  const empty = useProjectsStore((s) => s.projects.length === 0);
  const error = useProjectsStore((s) => s.error);
  const notice = useProjectsStore((s) => s.notice);
  const dismiss = useProjectsStore((s) => s.dismiss);
  const [adding, setAdding] = useState(false);

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
      <div className="border-t border-line p-1">
        <button
          type="button"
          title={`Settings (${formatShortcut(",")})`}
          onClick={() => useLayoutStore.getState().setSettingsOpen(true)}
          className="flex h-7 w-full items-center gap-2 rounded px-2 text-ink-muted hover:bg-raised hover:text-ink"
        >
          <span aria-hidden>⚙</span> Settings
        </button>
      </div>
      {adding && <AddProjectDialog onClose={() => setAdding(false)} />}
    </aside>
  );
}
