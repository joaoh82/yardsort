import { useState } from "react";
import type { Project, Workspace } from "@/lib/ipc";
import { native } from "@/lib/native";
import { recall, useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";
import {
  archiveWorkspace,
  deleteWorkspace,
  enterWorkspace,
  removeProject,
  restoreWorkspace,
} from "./actions";
import { summarise } from "@/features/terminal/activity";
import { StatusDot } from "@/features/terminal/StatusDot";
import { ContextMenu, type MenuItem } from "./ContextMenu";
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

  const items: MenuItem[] = [
    { label: "New workspace", disabled: project.missing, onSelect: () => compose(project.id) },
    {
      label: "Reveal in file manager",
      disabled: project.missing,
      onSelect: () => void native.revealInFileManager(project.rootPath).catch(console.error),
    },
    { label: "Move up", disabled: props.isFirst, onSelect: () => void move(project.id, -1) },
    { label: "Move down", disabled: props.isLast, onSelect: () => void move(project.id, 1) },
    { label: "Remove from Yardsort…", danger: true, onSelect: () => void removeProject(project) },
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
              <WorkspaceNode key={workspace.id} workspace={workspace} disabled={project.missing} />
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
            <WorkspaceNode key={workspace.id} workspace={workspace} disabled={project.missing} />
          ))}
        </ul>
      )}
    </li>
  );
}

function WorkspaceNode({ workspace, disabled }: { workspace: Workspace; disabled: boolean }) {
  const selected = useProjectsStore(
    (s) => s.selectedWorkspaceId === workspace.id && s.composingProjectId === null,
  );
  const allTabs = useTerminalStore((s) => s.tabs);
  const tabs = allTabs.filter((tab) => tab.workspaceId === workspace.id);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [launchAt, setLaunchAt] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const head = workspace.head;
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
          onClick={(event) => {
            // `local` is the project's own checkout, so there is no one obvious thing to open
            // there: a shell to work by hand, or an agent on the branch as it stands. It asks —
            // but only when there is nothing running and nothing to resume, which is exactly
            // when a worktree would have opened a shell by itself.
            const box = event.currentTarget.getBoundingClientRect();
            const ask = () => setLaunchAt({ x: box.left + 24, y: box.bottom });
            enterWorkspace(workspace.id, isWorktree ? undefined : ask);
          }}
          title={workspace.archived ? `Archived — was at ${workspace.path}` : workspace.path}
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
          {/* A worktree's branch is its name with a prefix; only `local` has news to tell. */}
          {head && !isWorktree && (
            <span
              className="ml-auto max-w-[55%] truncate font-mono text-[11px] text-ink-faint"
              title={head.detached ? "Detached HEAD" : head.unborn ? "No commits yet" : "Branch"}
            >
              {head.detached ? `@${head.label}` : head.label}
            </span>
          )}
        </button>
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
      {launchAt && (
        <ContextMenu
          at={launchAt}
          header={
            <>
              <img src="/icon.svg" alt="" className="size-4" />
              <span className="text-[12px] text-ink-muted">Yardsort</span>
            </>
          }
          items={[
            {
              label: "Open Terminal",
              onSelect: () => void useTerminalStore.getState().open(workspace.id),
            },
            {
              label: "Open Composer",
              onSelect: () => useProjectsStore.getState().composeIn(workspace),
            },
          ]}
          onClose={() => setLaunchAt(null)}
        />
      )}
      {renaming && <RenameDialog workspace={workspace} onClose={() => setRenaming(false)} />}
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
