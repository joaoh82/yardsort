import { useEffect, useState } from "react";
import { ActivityPanel } from "@/features/activity/ActivityPanel";
import { Composer } from "@/features/composer/Composer";
import { GettingStarted } from "@/features/onboarding/GettingStarted";
import { BenchRunner } from "@/features/terminal/BenchRunner";
import { LatencyRunner } from "@/features/terminal/LatencyRunner";
import { bareHarness } from "@/features/terminal/quickLaunch";
import { EndedBar } from "@/features/terminal/EndedBar";
import { SessionHistory } from "@/features/terminal/SessionHistory";
import { TerminalTabs } from "@/features/terminal/TerminalTabs";
import { TerminalView } from "@/features/terminal/TerminalView";
import { useTerminalSessions } from "@/features/terminal/useTerminalSessions";
import { ipc, errorMessage, type HarnessRequest, type Project, type Workspace } from "@/lib/ipc";
import { ProjectSettingsDialog } from "@/features/sidebar/ProjectSettingsDialog";
import { useAppStore } from "@/stores/app";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
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

  // A benchmark run takes the panel over: it reports to the core, which prints and quits.
  const bench = dev?.benchLatency ? (
    <LatencyRunner renderer={dev.renderer} />
  ) : dev?.bench ? (
    <BenchRunner script={dev.bench} renderer={dev.renderer} />
  ) : null;
  if (bench) {
    return (
      <main aria-label="Workspace" className="h-full bg-canvas">
        {bench}
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
  // The experimental timeline: offered only when the setting says so, opened per workspace.
  const timelineOffered = useAppStore((s) => s.showTimeline);
  const [timelineOpen, setTimelineOpen] = useState(false);

  return (
    <>
      <ProjectRun key={workspace.id} project={project} workspace={workspace} />
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
      {timelineOffered && timelineOpen && (
        <ActivityPanel workspaceId={workspace.id} onClose={() => setTimelineOpen(false)} />
      )}
      <footer className="flex h-6 shrink-0 items-center gap-2 border-t border-line bg-surface px-3 font-mono text-[11px] whitespace-nowrap text-ink-faint *:shrink-0">
        {timelineOffered && (
          <button
            type="button"
            aria-pressed={timelineOpen}
            onClick={() => setTimelineOpen((open) => !open)}
            className="rounded px-1 text-ink-muted hover:text-ink aria-pressed:bg-raised aria-pressed:text-ink"
          >
            Activity
          </button>
        )}
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
      {harnesses.map(({ id, label }) => (
        <button
          key={id}
          type="button"
          onClick={() => onLaunch(bareHarness(id))}
          className={`${button} inline-flex items-center gap-1.5`}
        >
          <HarnessIcon id={id} label={label} />
          {id}
        </button>
      ))}
    </div>
  );
}

function ProjectRun({ project, workspace }: { project: Project; workspace: Workspace }) {
  const [busy, setBusy] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function run() {
    setBusy(true);
    setError(null);
    try {
      const config = await ipc.projectAutomationGet(project.id);
      if (!config.run) {
        setSettingsOpen(true);
        return;
      }
      const terminals = useTerminalStore.getState();
      terminals.adopt(await ipc.workspaceRun(workspace.id, terminals.lastSize));
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <div className="flex items-center gap-3 border-b border-line px-3 py-1 text-ink-muted">
        <button
          type="button"
          disabled={busy || workspace.missing || workspace.archived || project.missing}
          onClick={() => void run()}
          className="hover:text-ink disabled:opacity-40"
        >
          {busy ? "Starting…" : "▶ Run"}
        </button>
        <button
          type="button"
          onClick={() => setSettingsOpen(true)}
          className="ml-auto hover:text-ink"
        >
          Project settings
        </button>
      </div>
      {error && (
        <p role="alert" className="px-3 py-2 text-red-400">
          {error}
        </p>
      )}
      {settingsOpen && (
        <ProjectSettingsDialog project={project} onClose={() => setSettingsOpen(false)} />
      )}
    </>
  );
}
