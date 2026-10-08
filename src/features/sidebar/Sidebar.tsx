import { useEffect, useRef, useState } from "react";
import { useOutcomesStore } from "@/stores/outcomes";
import { PanelHeader } from "@/features/shell/PanelHeader";
import { hasCore } from "@/lib/ipc";
import { useShortcutLabel } from "@/stores/preferences";
import { useAppStore } from "@/stores/app";
import { useLayoutStore } from "@/stores/layout";
import { useProjectsStore } from "@/stores/projects";
import { useUpdatesStore } from "@/stores/updates";
import { reviewVanishedWorkspaces } from "./actions";
import { AddProjectDialog } from "./AddProjectDialog";
import { ProjectTree } from "./ProjectTree";
import { WorkflowsSection } from "@/features/workflows/WorkflowsSection";
import { PullRequestsRow } from "@/features/pull-requests/PullRequestsRow";
import { usePullRequestPolling } from "@/features/pull-requests/usePullRequestPolling";
import { TasksRow } from "@/features/tasks/TasksRow";
import { useTaskPolling } from "@/features/tasks/useTaskPolling";

/** Left panel: projects and their workspaces. */
export function Sidebar() {
  const openKey = useShortcutLabel("openProject");
  const settingsKey = useShortcutLabel("settings");
  const loaded = useProjectsStore((s) => s.loaded);
  const empty = useProjectsStore((s) => s.projects.length === 0);
  const error = useProjectsStore((s) => s.error);
  const pendingClones = useProjectsStore((s) => s.pendingClones);
  const notice = useProjectsStore((s) => s.notice);
  const dismiss = useProjectsStore((s) => s.dismiss);
  const adding = useProjectsStore((s) => s.adding);
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const searchButton = useRef<HTMLButtonElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const closeSearch = () => {
    setQuery("");
    setSearching(false);
    requestAnimationFrame(() => searchButton.current?.focus());
  };
  useEffect(() => {
    if (searching) searchInput.current?.focus();
  }, [searching]);
  usePullRequestPolling();
  useTaskPolling();

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
      <PullRequestsRow />
      <TasksRow />
      <WorkflowsSection />
      <PanelHeader
        title="Projects"
        search={
          searching ? (
            <div className="mr-2 flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-full bg-raised px-2 focus-within:ring-1 focus-within:ring-accent">
              <SearchIcon />
              <input
                ref={searchInput}
                type="text"
                aria-label="Filter projects"
                placeholder="Filter projects…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    closeSearch();
                  }
                }}
                className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-ink-faint"
              />
              <button
                type="button"
                aria-label={query ? "Clear project filter" : "Close project search"}
                title={query ? "Clear project filter" : "Close project search"}
                onClick={() => {
                  if (query) {
                    setQuery("");
                    searchInput.current?.focus();
                  } else {
                    closeSearch();
                  }
                }}
                className="size-5 shrink-0 rounded text-ink-muted hover:text-ink"
              >
                ×
              </button>
            </div>
          ) : null
        }
      >
        <div className="flex shrink-0 items-center gap-1">
          {!searching && (
            <button
              ref={searchButton}
              type="button"
              aria-label="Search projects"
              title="Search projects"
              onClick={() => setSearching(true)}
              className="flex size-6 items-center justify-center rounded text-ink-muted hover:bg-raised hover:text-ink"
            >
              <SearchIcon />
            </button>
          )}
          <button
            type="button"
            aria-label="Add project"
            title={`Add project (open a folder: ${openKey})`}
            onClick={() => useProjectsStore.getState().openAddProject()}
            className="size-6 rounded text-ink-muted hover:bg-raised hover:text-ink"
          >
            +
          </button>
        </div>
      </PanelHeader>

      {loaded && empty ? (
        <div className="p-3 text-ink-faint">
          <p>No projects yet.</p>
          <button
            type="button"
            onClick={() => useProjectsStore.getState().openAddProject()}
            className="mt-2 rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink"
          >
            Add a project
          </button>
        </div>
      ) : (
        <ProjectTree query={query} />
      )}

      {pendingClones.map((request) => (
        <p key={request.id} role="status" className="px-3 py-2 text-ink-muted">
          Cloning {request.name}…
        </p>
      ))}
      <OutcomePrompt />
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
          title={`Settings (${settingsKey})`}
          onClick={() => useLayoutStore.getState().setSettingsOpen(true)}
          className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded px-2 text-ink-muted hover:bg-raised hover:text-ink"
        >
          <span aria-hidden>⚙</span> Settings
        </button>
        <UsageButton />
        <UpdatePill />
      </div>
      {adding && (
        <AddProjectDialog
          preset={adding}
          onClose={() => useProjectsStore.getState().closeAddProject()}
        />
      )}
    </aside>
  );
}

/** Usage, beside Settings, unless the user hid it; the command palette offers it either way. */
function UsageButton() {
  const show = useAppStore((s) => s.showUsageInSidebar);
  const open = useProjectsStore((s) => s.usageOpen);
  if (!show) return null;
  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      title="Usage: machine resources and tokens"
      onClick={() => useProjectsStore.getState().openUsage(!open)}
      className={`flex h-7 shrink-0 items-center gap-1.5 rounded px-2 hover:bg-raised hover:text-ink ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <svg aria-hidden="true" viewBox="0 0 16 16" className="size-3.5" fill="currentColor">
        <rect x="2" y="9" width="2.5" height="5" rx="0.5" />
        <rect x="6.75" y="5" width="2.5" height="9" rx="0.5" />
        <rect x="11.5" y="2" width="2.5" height="12" rx="0.5" />
      </svg>
      Usage
    </button>
  );
}

function SearchIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      className="size-4 shrink-0 text-ink-muted"
    >
      <circle cx="10.5" cy="10.5" r="7" />
      <path d="m16 16 5 5" />
    </svg>
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

/**
 * Right after a workspace is archived or deleted: how did that attempt go? One optional click;
 * dismissing leaves it unlabelled, and the project's Outcomes view can label it any time.
 */
function OutcomePrompt() {
  const asking = useOutcomesStore((s) => s.asking);
  const error = useOutcomesStore((s) => s.error);
  if (!asking) return null;
  const { answer, dismiss } = useOutcomesStore.getState();
  const choice =
    "rounded border border-line px-2 py-0.5 text-[11px] text-ink-muted hover:border-accent hover:text-ink";
  return (
    <div role="status" aria-label="How did it go?" className="border-t border-line p-3">
      <div className="flex items-start gap-2">
        <p className="min-w-0 flex-1 text-ink-muted">
          How did <span className="text-ink">{asking.name}</span> go?
        </p>
        <button
          type="button"
          aria-label="Not now"
          onClick={dismiss}
          className="text-ink-faint hover:text-ink"
        >
          ×
        </button>
      </div>
      <div className="mt-2 flex gap-1.5">
        <button type="button" onClick={() => void answer("kept")} className={choice}>
          Kept
        </button>
        <button type="button" onClick={() => void answer("partly")} className={choice}>
          Partly
        </button>
        <button type="button" onClick={() => void answer("discarded")} className={choice}>
          Discarded
        </button>
      </div>
      {error && (
        <p role="alert" className="mt-2 text-red-400 select-text">
          {error}
        </p>
      )}
    </div>
  );
}
