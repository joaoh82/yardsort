import { useEffect } from "react";
import { Composer } from "@/features/composer/Composer";
import { GettingStarted } from "@/features/onboarding/GettingStarted";
import { BenchRunner } from "@/features/terminal/BenchRunner";
import { bareHarness } from "@/features/terminal/quickLaunch";
import { EndedBar } from "@/features/terminal/EndedBar";
import { SessionHistory } from "@/features/terminal/SessionHistory";
import { TerminalTabs } from "@/features/terminal/TerminalTabs";
import { TerminalView } from "@/features/terminal/TerminalView";
import { useTerminalSessions } from "@/features/terminal/useTerminalSessions";
import type { HarnessRequest, Project, Workspace } from "@/lib/ipc";
import { useAppStore } from "@/stores/app";
import { launchable, useHarnessStore } from "@/stores/harnesses";
import { shortcutKeys } from "@/lib/platform";
import { useProjectsStore, useSelectedWorkspace } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";

/** Center panel: the composer while a workspace is being started, otherwise its terminals. */
export function WorkspacePanel() {
  useTerminalSessions();
  const dev = useAppStore((s) => s.info?.dev);
  const selection = useSelectedWorkspace();
  const composingFor = useProjectsStore((s) =>
    s.projects.find((project) => project.id === s.composingProjectId),
  );
  // Composing a run inside a workspace that already exists — `local`. Selection is what holds it,
  // so the panel needs both the workspace and the project it belongs to.
  const composingWorkspaceId = useProjectsStore((s) => s.composingWorkspaceId);
  const runIn =
    selection && selection.workspace.id === composingWorkspaceId ? selection : undefined;

  if (dev?.bench) {
    return (
      <main aria-label="Workspace" className="h-full bg-canvas">
        <BenchRunner script={dev.bench} renderer={dev.renderer} />
      </main>
    );
  }
  return (
    <main aria-label="Workspace" className="flex h-full flex-col bg-canvas">
      {composingFor ? (
        <Composer key={composingFor.id} project={composingFor} />
      ) : runIn ? (
        <Composer key={runIn.workspace.id} project={runIn.project} runIn={runIn.workspace} />
      ) : selection ? (
        <WorkspaceTerminals {...selection} rendererOverride={dev?.renderer} />
      ) : (
        <GettingStarted />
      )}
    </main>
  );
}

function WorkspaceTerminals(props: {
  project: Project;
  workspace: Workspace;
  rendererOverride?: string | null;
}) {
  const { project, workspace } = props;
  const activeId = useTerminalStore((s) => s.active[workspace.id]);
  const activeTab = useTerminalStore((s) => s.tabs.find((tab) => tab.id === activeId));
  const sessionError = useSessionsStore((s) => s.error);

  // The history is what Resume and Fork are offered on; keep it fresh for the workspace in view:
  // when it comes into view, and whenever a terminal opens or closes in it.
  const tabCount = useTerminalStore(
    (s) => s.tabs.filter((tab) => tab.workspaceId === workspace.id).length,
  );
  useEffect(() => {
    void useSessionsStore.getState().load(workspace.id);
  }, [workspace.id, tabCount]);
  // Looking at a terminal is what clears its "finished, not seen yet" mark.
  useEffect(() => {
    if (activeId) useTerminalStore.getState().activate(activeId);
  }, [activeId]);
  const error = useTerminalStore((s) => s.error);
  const dismissError = useTerminalStore((s) => s.dismissError);
  const open = useTerminalStore((s) => s.open);
  const head = workspace.head;

  return (
    <>
      <TerminalTabs workspaceId={workspace.id} />
      {error && (
        <div
          role="alert"
          className="flex items-start gap-3 border-b border-line bg-raised px-3 py-2"
        >
          <p className="flex-1 text-red-400 select-text">{error}</p>
          <button type="button" onClick={dismissError} className="text-ink-faint hover:text-ink">
            Dismiss
          </button>
        </div>
      )}
      {sessionError && (
        <div
          role="alert"
          className="flex items-start gap-3 border-b border-line bg-raised px-3 py-2"
        >
          <p className="flex-1 text-red-400 select-text">{sessionError}</p>
          <button
            type="button"
            onClick={() => useSessionsStore.getState().dismissError()}
            className="text-ink-faint hover:text-ink"
          >
            Dismiss
          </button>
        </div>
      )}
      {activeTab && <EndedBar tab={activeTab} />}
      <div className="min-h-0 flex-1">
        {/* Only the active terminal is mounted; the others keep running in the core and are
            repainted from a snapshot when they come back. `key` forces a fresh view per session. */}
        {activeId ? (
          <TerminalView
            key={activeId}
            sessionId={activeId}
            rendererOverride={props.rendererOverride}
          />
        ) : (
          <div className="flex h-full flex-col items-center justify-center overflow-y-auto p-6 text-center">
            {workspace.kind === "local" ? (
              <LocalActions workspace={workspace} />
            ) : (
              <>
                <p className="text-ink-muted">Nothing running in this workspace.</p>
                <LaunchButtons onLaunch={(harness) => void open(workspace.id, harness)} />
              </>
            )}
            <SessionHistory workspaceId={workspace.id} />
          </div>
        )}
      </div>
      <footer className="flex h-6 shrink-0 items-center gap-2 border-t border-line bg-surface px-3 font-mono text-[11px] whitespace-nowrap text-ink-faint *:shrink-0">
        <span className="text-ink-muted">{project.name}</span>
        <span>/</span>
        <span className="text-ink-muted">{workspace.name}</span>
        {head && (
          <span title={head.detached ? "Detached HEAD" : head.unborn ? "No commits yet" : "Branch"}>
            · {head.detached ? `@${head.label}` : head.label}
            {head.unborn ? " (no commits yet)" : ""}
          </span>
        )}
        <span className="ml-auto min-w-0 shrink! truncate select-text" title={workspace.path}>
          {workspace.path}
        </span>
      </footer>
    </>
  );
}

/**
 * What `local` offers when nothing is running in it: the project's own checkout is not a
 * workspace you came to watch, it is a place you might do one of several things. Both answers
 * open something here — a shell to work by hand, or an agent on the branch as it stands.
 */
function LocalActions({ workspace }: { workspace: Workspace }) {
  const actions = [
    {
      label: "Open Terminal",
      keys: shortcutKeys("T"),
      onSelect: () => void useTerminalStore.getState().open(workspace.id),
    },
    {
      label: "Open Composer",
      keys: null,
      onSelect: () => useProjectsStore.getState().composeIn(workspace),
    },
  ];
  return (
    <div className="flex flex-col items-center">
      <img src="/icon.svg" alt="" className="mb-6 size-10 opacity-80" />
      <ul className="w-72">
        {actions.map((action) => (
          <li key={action.label}>
            <button
              type="button"
              onClick={action.onSelect}
              className="flex w-full items-center justify-between rounded px-3 py-2 text-left text-ink-muted hover:bg-raised hover:text-ink"
            >
              <span>{action.label}</span>
              {action.keys && (
                <span className="flex gap-1">
                  {action.keys.map((cap) => (
                    <kbd
                      key={cap}
                      className="rounded border border-line bg-canvas px-1.5 py-0.5 font-mono text-[10px] text-ink-faint"
                    >
                      {cap}
                    </kbd>
                  ))}
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function LaunchButtons({ onLaunch }: { onLaunch: (harness?: HarnessRequest) => void }) {
  const harnesses = launchable(useHarnessStore((s) => s.harnesses));
  const button =
    "rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink";
  return (
    <div className="mt-4 flex flex-wrap justify-center gap-2">
      <button type="button" onClick={() => onLaunch()} className={button}>
        shell
      </button>
      {harnesses.map(({ id }) => (
        <button key={id} type="button" onClick={() => onLaunch(bareHarness(id))} className={button}>
          {id}
        </button>
      ))}
    </div>
  );
}
