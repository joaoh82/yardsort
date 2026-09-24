import { useState } from "react";
import type { Project, Workspace } from "@/lib/ipc";
import { native } from "@/lib/native";
import { recall, useProjectsStore } from "@/stores/projects";
import { pullRequestFor, usePublishStore } from "@/stores/publish";
import { useTerminalStore } from "@/stores/terminals";
import { archiveWorkspace, deleteWorkspace, enterWorkspace, restoreWorkspace } from "./actions";
import { harnessState, summarise } from "@/features/terminal/activity";
import { HarnessBadge } from "@/features/terminal/HarnessBadge";
import { StatusDot } from "@/features/terminal/StatusDot";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { ForgetDialog } from "./ForgetDialog";
import { ImportWorktreesDialog } from "./ImportWorktreesDialog";
import { PullRequestBadge } from "./PullRequestBadge";
import { RemoveProjectDialog } from "./RemoveProjectDialog";
import { ProjectSettingsDialog } from "./ProjectSettingsDialog";
import { RenameDialog } from "./RenameDialog";

export function ProjectTree() {
  const projects = useProjectsStore((s) => s.projects);
  return (
    <ul role="tree" aria-label="Projects" className="min-h-0 flex-1 overflow-y-auto py-1">
      {projects.map((project, index) => (
        <ProjectNode
          key={project.id}
          project={project}
          isFirst={index === 0}
          isLast={index === projects.length - 1}
        />
      ))}
    </ul>
  );
}

function ProjectNode(props: { project: Project; isFirst: boolean; isLast: boolean }) {
  const { project } = props;
  const expanded = useProjectsStore((s) => !s.collapsed.includes(project.id));
  const toggleCollapsed = useProjectsStore((s) => s.toggleCollapsed);
  const move = useProjectsStore((s) => s.move);
  const compose = useProjectsStore((s) => s.compose);
  const composing = useProjectsStore((s) => s.composingProjectId === project.id);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [importing, setImporting] = useState(false);
  const [removing, setRemoving] = useState(false);

  const items: MenuItem[] = [
    { label: "Project settings…", onSelect: () => setSettingsOpen(true) },
    { label: "New workspace", disabled: project.missing, onSelect: () => compose(project.id) },
    {
      label: "Import worktrees…",
      disabled: project.missing,
      onSelect: () => setImporting(true),
    },
    {
      label: "Reveal in file manager",
      disabled: project.missing,
      onSelect: () => void native.revealInFileManager(project.rootPath).catch(console.error),
    },
    { label: "Move up", disabled: props.isFirst, onSelect: () => void move(project.id, -1) },
    { label: "Move down", disabled: props.isLast, onSelect: () => void move(project.id, 1) },
    { label: "Remove from Yardsort…", danger: true, onSelect: () => setRemoving(true) },
  ];

  return (
    <li role="treeitem" aria-expanded={expanded} aria-label={project.name}>
      <div
        className="group flex h-7 items-center pr-1 hover:bg-raised"
        onContextMenu={(event) => {
          event.preventDefault();
          setMenuAt({ x: event.clientX, y: event.clientY });
        }}
      >
        <button
          type="button"
          onClick={() => toggleCollapsed(project.id)}
          title={project.rootPath}
          className="flex h-full min-w-0 flex-1 items-center gap-1 pl-2 text-left"
        >
          <span
            aria-hidden
            className={`w-3 text-[10px] text-ink-faint ${expanded ? "rotate-90" : ""}`}
          >
            ▶
          </span>
          <span
            className={`truncate font-medium ${project.missing ? "text-ink-faint line-through" : ""}`}
          >
            {project.name}
          </span>
          {project.missing && <span className="text-[11px] text-red-400">missing</span>}
        </button>
        <RowButton
          label={`More actions for ${project.name}`}
          onClick={(event) => {
            const box = event.currentTarget.getBoundingClientRect();
            setMenuAt({ x: box.left, y: box.bottom + 2 });
          }}
        >
          ⋯
        </RowButton>
        <RowButton
          label={`New workspace in ${project.name}`}
          disabled={project.missing}
          onClick={() => compose(project.id)}
        >
          +
        </RowButton>
      </div>

      {expanded && (
        <ul role="group">
          {project.workspaces
            .filter((workspace) => !workspace.archived)
            .map((workspace) => (
              <WorkspaceNode
                key={workspace.id}
                workspace={workspace}
                projectId={project.id}
                disabled={project.missing}
              />
            ))}
          {composing && (
            <li className="flex h-7 items-center gap-2 bg-raised pr-2 pl-7 text-ink-muted italic">
              <span aria-hidden className="size-1.5 shrink-0 rounded-full border border-accent" />
              new workspace…
            </li>
          )}
          <ArchivedGroup project={project} />
        </ul>
      )}
      {menuAt && <ContextMenu at={menuAt} items={items} onClose={() => setMenuAt(null)} />}
      {settingsOpen && (
        <ProjectSettingsDialog project={project} onClose={() => setSettingsOpen(false)} />
      )}
      {importing && <ImportWorktreesDialog project={project} onClose={() => setImporting(false)} />}
      {removing && <RemoveProjectDialog project={project} onClose={() => setRemoving(false)} />}
    </li>
  );
}

/** Archived workspaces, tucked away under their project until someone goes looking. */
function ArchivedGroup({ project }: { project: Project }) {
  const archived = project.workspaces.filter((workspace) => workspace.archived);
  const key = `sidebar.showArchived.${project.id}`;
  const open = useProjectsStore((s) => recall(s.ui, key, false));
  if (archived.length === 0) return null;

  return (
    <li role="treeitem" aria-expanded={open} aria-label="Archived workspaces">
      <button
        type="button"
        onClick={() => useProjectsStore.getState().remember(key, !open)}
        className="flex h-6 w-full items-center gap-1 pl-7 text-left text-[11px] text-ink-faint hover:text-ink-muted"
      >
        <span aria-hidden className={`w-3 text-[9px] ${open ? "rotate-90" : ""}`}>
          ▶
        </span>
        archived ({archived.length})
      </button>
      {open && (
        <ul role="group">
          {archived.map((workspace) => (
            <WorkspaceNode
              key={workspace.id}
              workspace={workspace}
              projectId={project.id}
              disabled={project.missing}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

function WorkspaceNode({
  workspace,
  projectId,
  disabled,
}: {
  workspace: Workspace;
  projectId: string;
  disabled: boolean;
}) {
  const selected = useProjectsStore(
    (s) => s.selectedWorkspaceId === workspace.id && s.composingProjectId === null,
  );
  // Select the project's own entry, never a derived object: a selector that built one would
  // hand back a new value on every render and re-render for ever.
  const found = usePublishStore((s) => s.byProject[projectId]);
  const head = workspace.head;
  const pr = pullRequestFor(found, head && !head.detached ? head.label : undefined);
  const allTabs = useTerminalStore((s) => s.tabs);
  const tabs = allTabs.filter((tab) => tab.workspaceId === workspace.id);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [forgetting, setForgetting] = useState(false);
  const isWorktree = workspace.kind === "worktree";
  const gone = workspace.missing || workspace.archived;
  const unusable = disabled || gone;
  // Its folder went and its branch with it: restoring is off the table, deleting is all there is.
  const unrestorable = gone && workspace.branchGone;

  const items: MenuItem[] = [
    ...(unrestorable
      ? []
      : [
          gone
            ? {
                label: workspace.archived ? "Restore workspace" : "Restore from its branch",
                disabled: disabled || !isWorktree,
                onSelect: () => void restoreWorkspace(workspace),
              }
            : {
                label: "Reveal in file manager",
                disabled,
                onSelect: () =>
                  void native.revealInFileManager(workspace.path).catch(console.error),
              },
        ]),
    ...(isWorktree
      ? [
          { label: "Rename…", onSelect: () => setRenaming(true) },
          ...(gone
            ? []
            : [{ label: "Archive…", onSelect: () => void archiveWorkspace(workspace) }]),
          { label: "Forget…", onSelect: () => setForgetting(true) },
          {
            label: workspace.archived ? "Delete for good…" : "Delete workspace…",
            danger: true,
            onSelect: () => void deleteWorkspace(workspace),
          },
        ]
      : []),
  ];

  return (
    <li role="treeitem" aria-selected={selected} aria-label={workspace.name}>
      <div
        className={`group flex h-7 items-center pr-1 ${
          selected ? "bg-raised text-ink" : "text-ink-muted hover:bg-raised"
        }`}
        onContextMenu={(event) => {
          event.preventDefault();
          setMenuAt({ x: event.clientX, y: event.clientY });
        }}
      >
        <button
          type="button"
          disabled={unusable}
          // A worktree with nothing running opens a shell, because that is what you came for.
          // `local` is the project's own checkout and a shell is only one of the things you
          // might want there, so it lands on the panel's own list of them instead.
          onClick={() => enterWorkspace(workspace.id, isWorktree)}
          // No tooltip for a row you can open. It said the path, which the bottom bar already
          // shows for the workspace in view, and a native tooltip appears wherever the pointer
          // is — including straight over the menu the click just opened. A row that *cannot* be
          // opened keeps it: it cannot be selected either, so this is the only place its path
          // and its state are written down.
          title={
            !unusable
              ? undefined
              : workspace.archived
                ? `Archived — was at ${workspace.path}`
                : workspace.missing
                  ? `Missing — was at ${workspace.path}`
                  : workspace.path
          }
          className={`flex h-full min-w-0 flex-1 items-center gap-2 pr-1 text-left disabled:opacity-40 ${
            workspace.archived ? "pl-11" : "pl-7"
          }`}
        >
          <StatusDot activity={summarise(tabs)} attention={tabs.some((tab) => tab.attention)} />
          <span className={`truncate ${workspace.missing ? "line-through" : ""}`}>
            {workspace.name}
          </span>
          {(workspace.missing || unrestorable) && !disabled && (
            <span
              className="text-[11px] text-red-400"
              title={
                unrestorable
                  ? "Its folder and its branch were both removed. There is nothing left to restore it from."
                  : "Its folder was removed. Restore it from its branch, or delete it."
              }
            >
              {unrestorable ? "gone" : "missing"}
            </span>
          )}
          <span className="ml-auto flex min-w-0 items-center gap-1.5 pl-1">
            {/* A worktree's branch is its name with a prefix; only `local` has news to tell. */}
            {head && !isWorktree && (
              <span
                className="max-w-32 truncate font-mono text-[11px] text-ink-faint"
                title={head.detached ? "Detached HEAD" : head.unborn ? "No commits yet" : "Branch"}
              >
                {head.detached ? `@${head.label}` : head.label}
              </span>
            )}
            <HarnessBadge
              state={harnessState(tabs)}
              attention={tabs.some((tab) => tab.attention)}
            />
          </span>
        </button>
        {pr && (
          <span className="flex h-full shrink-0 items-center pr-0.5 pl-1">
            <PullRequestBadge pr={pr} />
          </span>
        )}
        {isWorktree && (
          <RowButton
            label={`More actions for ${workspace.name}`}
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              setMenuAt({ x: box.left, y: box.bottom + 2 });
            }}
          >
            ⋯
          </RowButton>
        )}
      </div>
      {menuAt && <ContextMenu at={menuAt} items={items} onClose={() => setMenuAt(null)} />}
      {renaming && <RenameDialog workspace={workspace} onClose={() => setRenaming(false)} />}
      {forgetting && <ForgetDialog workspace={workspace} onClose={() => setForgetting(false)} />}
    </li>
  );
}

function RowButton(props: {
  label: string;
  disabled?: boolean;
  onClick?: (event: React.MouseEvent<HTMLButtonElement>) => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={props.label}
      title={props.label}
      disabled={props.disabled}
      onClick={props.onClick}
      className="size-6 shrink-0 rounded text-ink-faint opacity-0 group-hover:opacity-100 hover:bg-line hover:text-ink focus-visible:opacity-100 disabled:hover:bg-transparent disabled:hover:text-ink-faint"
    >
      {props.children}
    </button>
  );
}
