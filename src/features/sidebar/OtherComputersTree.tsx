import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { OtherHost, OtherHostView, OtherProject, OtherTask, OtherWorkspace } from "@/lib/ipc";
import { treeNavigation } from "@/lib/treeNavigation";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { activityThere, startAgentThere } from "@/features/remote/there";
import { elapsedWords } from "@/features/terminal/BusyClock";
import { StatusDot } from "@/features/terminal/StatusDot";
import { useLayoutStore } from "@/stores/layout";
import { tabsThere, thereKey, useOtherComputersStore } from "@/stores/otherComputers";
import { recall, useProjectsStore } from "@/stores/projects";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { pairingText } from "./otherComputersText";

/** Which computers the person opened or folded themselves, by host id. */
const COMPUTERS = "sidebar.computers";
/** The projects folded under a computer, each as `host/project`. */
const FOLDED_PROJECTS = "sidebar.computerProjects";

/**
 * Below the projects: the other computers this one reaches as a client, as a tree. Each computer
 * has its projects under it, and their workspaces under those, drawn as this computer's own are.
 * They are kept apart from the projects, so nothing of another computer is taken for this one's,
 * and a computer folds, so many of them stay manageable. Nothing shows until this computer is
 * paired with one.
 */
export function OtherComputersTree({ query = "" }: { query?: string }) {
  const status = useOtherComputersStore((s) => s.status);
  const hosts = status?.client ? status.hosts : [];
  const shown = hosts.filter((h) => h.pairing === "paired" || h.pairing === "waiting");
  if (shown.length === 0) return null;
  return (
    <section
      aria-label="Other computers"
      className="flex max-h-[55%] min-h-0 shrink-0 flex-col border-t border-line"
    >
      <header className="flex h-10 shrink-0 items-center justify-between border-b border-line px-3.5">
        <h2 className="text-small font-semibold tracking-wider text-ink-muted uppercase">
          Other computers
        </h2>
        <button
          type="button"
          aria-label="Pair another computer"
          title="Pair another computer, in Settings → Remote"
          onClick={() => useLayoutStore.getState().openSettings("remote")}
          className="size-7 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          +
        </button>
      </header>
      <ul
        role="tree"
        aria-label="Other computers"
        onKeyDown={treeNavigation}
        className="min-h-0 overflow-y-auto py-1"
      >
        {shown.map((host) => (
          <ComputerNode key={host.grantId} host={host} filter={query.trim().toLowerCase()} />
        ))}
      </ul>
    </section>
  );
}

function ComputerNode({ host, filter }: { host: OtherHost; filter: string }) {
  const view = useOtherComputersStore((s) => s.views[host.hostId]);
  const known = useOtherComputersStore((s) => host.hostId in s.harnesses);
  const chosen = useProjectsStore(
    (s) => recall<Record<string, boolean>>(s.ui, COMPUTERS, {})[host.hostId],
  );
  const selectedHere = useProjectsStore(
    (s) => (s.remote ?? s.composingThere)?.hostId === host.hostId,
  );
  const selectedId = useProjectsStore((s) =>
    s.remote?.hostId === host.hostId ? s.remote.workspaceId : null,
  );
  const paired = host.pairing === "paired";
  const online = paired && host.link?.kind === "online";
  const offline = paired && host.link?.kind === "offline";
  // An offline computer folds by itself, until the person opens it; the one in view stays open.
  const open = paired && (selectedHere || (chosen ?? !offline));
  const mayStart = online && !!view?.mayStart;

  // Its agents, for what its rows offer to start. Asked once, when it can first be asked.
  useEffect(() => {
    if (mayStart && !known) void useOtherComputersStore.getState().loadHarnesses(host.hostId);
  }, [mayStart, known, host.hostId]);

  const projects = (view?.projects ?? []).filter(
    (project) =>
      filter === "" ||
      host.name.toLowerCase().includes(filter) ||
      project.name.toLowerCase().includes(filter) ||
      project.workspaces.some((workspace) => workspace.id === selectedId),
  );
  // Filtering shows the computers that have something to show for it.
  if (filter !== "" && projects.length === 0 && !host.name.toLowerCase().includes(filter)) {
    return null;
  }

  const toggle = () => {
    const projectsStore = useProjectsStore.getState();
    const all = recall<Record<string, boolean>>(projectsStore.ui, COMPUTERS, {});
    projectsStore.remember(COMPUTERS, { ...all, [host.hostId]: !open });
  };

  return (
    <li role="treeitem" aria-expanded={paired ? open : undefined} aria-label={host.name}>
      <div className="group flex h-8 items-center pr-2 hover:bg-raised">
        <button
          type="button"
          disabled={!paired}
          onClick={toggle}
          title={paired ? `${host.name}, another computer` : pairingText(host)}
          className="flex h-full min-w-0 flex-1 items-center gap-1.5 pl-2 text-left disabled:cursor-default"
        >
          <span
            aria-hidden
            className={`w-3 text-micro text-ink-faint ${open ? "rotate-90" : ""} ${paired ? "" : "invisible"}`}
          >
            ▶
          </span>
          <ComputerIcon />
          <span className={`truncate font-medium ${online ? "" : "text-ink-muted"}`}>
            {host.name}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-1.5 pl-2 text-small text-ink-faint">
            {online && <span aria-hidden className="size-2 rounded-full bg-green-400" />}
            {standing(host, view)}
          </span>
        </button>
      </div>
      {open && (
        <ul role="group" className={online ? undefined : "opacity-60"}>
          {view === undefined ? null : view === null || view.readAt === null ? (
            <li className="py-1 pr-2 pl-9 text-small text-ink-faint">
              {offline ? "Not reached yet." : "Reading it…"}
            </li>
          ) : projects.length === 0 ? (
            <li className="py-1 pr-2 pl-9 text-small text-ink-faint">
              {filter === ""
                ? "It shows no workspaces to this computer."
                : "No projects match your search."}
            </li>
          ) : (
            projects.map((project) => (
              <ProjectNode
                key={project.id}
                hostId={host.hostId}
                hostName={host.name}
                project={project}
                mayStart={mayStart}
              />
            ))
          )}
        </ul>
      )}
    </li>
  );
}

/** How a computer stands, in a word or two, for the end of its row. */
function standing(host: OtherHost, view: OtherHostView | null | undefined): string {
  if (host.pairing === "waiting") return "waiting";
  const link = host.link;
  if (!link) return "";
  if (link.kind === "connecting") return "connecting…";
  if (link.kind === "offline") {
    return link.since === null ? "offline" : `offline · ${elapsedWords(Date.now() - link.since)}`;
  }
  // Let in to watch, and told so once it was read.
  return view && !view.mayType ? "watch only" : "online";
}

function ProjectNode({
  hostId,
  hostName,
  project,
  mayStart,
}: {
  hostId: string;
  hostName: string;
  project: OtherProject;
  mayStart: boolean;
}) {
  const key = thereKey(hostId, project.id);
  const composing = useProjectsStore(
    (s) => s.composingThere?.hostId === hostId && s.composingThere.projectId === project.id,
  );
  const selectedIn = useProjectsStore(
    (s) =>
      s.remote?.hostId === hostId &&
      project.workspaces.some((workspace) => workspace.id === s.remote?.workspaceId),
  );
  const folded = useProjectsStore((s) => recall<string[]>(s.ui, FOLDED_PROJECTS, []).includes(key));
  // Where the selection is, and where a new workspace will appear, is never hidden.
  const expanded = !folded || composing || selectedIn;
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const compose = () => useProjectsStore.getState().composeThere({ hostId, projectId: project.id });
  const toggle = () => {
    const projects = useProjectsStore.getState();
    const all = recall<string[]>(projects.ui, FOLDED_PROJECTS, []);
    projects.remember(
      FOLDED_PROJECTS,
      expanded ? [...all.filter((k) => k !== key), key] : all.filter((k) => k !== key),
    );
  };
  const items: MenuItem[] = [{ label: "New workspace", disabled: !mayStart, onSelect: compose }];
  const live = project.workspaces.filter((workspace) => !workspace.archived);
  const archived = project.workspaces.filter((workspace) => workspace.archived);

  return (
    <li role="treeitem" aria-expanded={expanded} aria-label={project.name}>
      <div
        className="group flex h-8 items-center pr-1 hover:bg-raised"
        onContextMenu={(event) => {
          event.preventDefault();
          setMenuAt({ x: event.clientX, y: event.clientY });
        }}
      >
        <button
          type="button"
          onClick={toggle}
          className="flex h-full min-w-0 flex-1 items-center gap-1 pl-6 text-left"
        >
          <span
            aria-hidden
            className={`w-3 text-micro text-ink-faint ${expanded ? "rotate-90" : ""}`}
          >
            ▶
          </span>
          <span className="truncate font-medium">{project.name}</span>
        </button>
        {mayStart && (
          <button
            type="button"
            aria-label={`New workspace in ${project.name} on ${hostName}`}
            title={`New workspace in ${project.name} on ${hostName}`}
            onClick={compose}
            className="size-7 shrink-0 rounded text-ink-faint opacity-0 group-hover:opacity-100 hover:bg-line hover:text-ink focus-visible:opacity-100"
          >
            +
          </button>
        )}
      </div>
      {expanded && (
        <ul role="group">
          {live.map((workspace) => (
            <WorkspaceNode
              key={workspace.id}
              hostId={hostId}
              workspace={workspace}
              mayStart={mayStart}
            />
          ))}
          {composing && (
            <li className="flex h-8 items-center gap-2 bg-raised pr-2 pl-12 text-ink-muted italic">
              <span aria-hidden className="size-2 shrink-0 rounded-full border border-accent" />
              new workspace…
            </li>
          )}
          {archived.length > 0 && (
            <li className="flex h-7 items-center pl-12 text-small text-ink-faint">
              archived ({archived.length})
            </li>
          )}
        </ul>
      )}
      {menuAt && <ContextMenu at={menuAt} items={items} onClose={() => setMenuAt(null)} />}
    </li>
  );
}

function WorkspaceNode({
  hostId,
  workspace,
  mayStart,
}: {
  hostId: string;
  workspace: OtherWorkspace;
  mayStart: boolean;
}) {
  const selected = useProjectsStore(
    (s) => s.remote?.hostId === hostId && s.remote.workspaceId === workspace.id,
  );
  const harnesses = useOtherComputersStore((s) => s.harnesses[hostId]);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const tabs = tabsThere(workspace);
  const agents = tabs.filter((tab) => tab.harness !== null && tab.state !== "exited");
  const firstAgent = agents[0];
  const waiting = agents.some((tab) => tab.state !== "busy");
  const task = workspace.tasks[0];
  const items: MenuItem[] = (harnesses ?? []).map((harness) => ({
    label: `Start ${harness.name}`,
    disabled: !mayStart || workspace.missing,
    onSelect: () => void startAgentThere(hostId, workspace.id, harness.id),
  }));

  return (
    <li role="treeitem" aria-selected={selected} aria-label={workspace.name}>
      <div
        className={`group flex h-8 items-center pr-2 ${
          selected ? "bg-raised text-ink" : "text-ink-muted hover:bg-raised"
        }`}
        onContextMenu={(event) => {
          if (items.length === 0) return;
          event.preventDefault();
          setMenuAt({ x: event.clientX, y: event.clientY });
        }}
      >
        <button
          type="button"
          disabled={workspace.missing}
          onClick={() => useProjectsStore.getState().selectRemote(hostId, workspace.id)}
          title={workspace.missing ? "Its folder is missing on that computer." : undefined}
          className="flex h-full min-w-0 flex-1 items-center gap-2 pr-1 pl-12 text-left disabled:opacity-40"
        >
          <StatusDot activity={activityThere(tabs)} />
          <span className={`truncate ${workspace.missing ? "line-through" : ""}`}>
            {workspace.name}
          </span>
          {workspace.missing && <span className="text-small text-red-400">missing</span>}
          {/* A worktree's branch is its name with a prefix; only `local` has news to tell. */}
          {workspace.kind === "local" && workspace.branch && (
            <span
              className="ml-auto max-w-32 truncate pl-1 font-mono text-small text-ink-faint"
              title="Branch"
            >
              {workspace.branch}
            </span>
          )}
        </button>
        {firstAgent && (
          <span
            aria-label={`${agents.length} open ${agents.length === 1 ? "harness" : "harnesses"}`}
            className={`flex shrink-0 items-center gap-1 rounded-full px-1.5 py-0.5 text-micro ${
              waiting ? "bg-accent/15 text-accent" : "bg-canvas text-ink-muted"
            }`}
          >
            <HarnessIcon id={firstAgent.title} label={firstAgent.title} size={14} />
            {agents.length}
          </span>
        )}
        {task && <TaskLink task={task} />}
      </div>
      {menuAt && <ContextMenu at={menuAt} items={items} onClose={() => setMenuAt(null)} />}
    </li>
  );
}

/** The task a workspace there was started from: its key, which opens the task's own page. */
function TaskLink({ task }: { task: OtherTask }) {
  return (
    <button
      type="button"
      aria-label={`Open task ${task.key} — ${task.title}`}
      title={`Started from task ${task.key}: ${task.title}`}
      // Its address came from that computer: only a web page is opened.
      onClick={() => {
        if (task.url.startsWith("https://")) void openUrl(task.url).catch(console.error);
      }}
      className="ml-1 shrink-0 rounded border border-line px-1 text-micro leading-4.5 font-medium text-ink-muted tabular-nums hover:bg-raised hover:text-ink"
    >
      {task.key}
    </button>
  );
}

export function ComputerIcon({ className = "size-4 shrink-0" }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <rect x="2" y="2.5" width="12" height="8.5" rx="1" />
      <path d="M6 13.5h4M8 11v2.5" />
    </svg>
  );
}
