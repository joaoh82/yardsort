import {
  composeInCurrentProject,
  enterWorkspace,
  openProjectFromDisk,
} from "@/features/sidebar/actions";
import type { CommandId } from "@/lib/shortcuts";
import { useLayoutStore } from "@/stores/layout";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";

export function availableWorkspaces() {
  return useProjectsStore
    .getState()
    .projects.filter((p) => !p.missing)
    .flatMap((project) =>
      project.workspaces
        .filter((w) => !w.archived && !w.missing)
        .map((workspace) => ({ project, workspace })),
    );
}
export function selectWorkspace(id: string) {
  const item = availableWorkspaces().find(({ workspace }) => workspace.id === id);
  if (!item) return;
  const state = useProjectsStore.getState();
  if (state.collapsed.includes(item.project.id)) state.toggleCollapsed(item.project.id);
  enterWorkspace(id, item.workspace.kind === "worktree");
}
export function focusPanel(panel: "Projects" | "Workspace" | "Changes") {
  const layout = useLayoutStore.getState();
  if (panel !== "Workspace") layout.setCollapsed(panel === "Projects" ? "left" : "right", false);
  requestAnimationFrame(() => {
    const region = document.querySelector<HTMLElement>(`[data-navigation="${panel}"]`);
    if (!region) return;
    const terminal = region.querySelector<HTMLElement>(".xterm-helper-textarea");
    const control = region.querySelector<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), [tabindex="0"]',
    );
    (terminal ?? control ?? region).focus();
  });
}
export function commandEnabled(id: CommandId): boolean {
  const projects = useProjectsStore.getState();
  const workspaceId = projects.selectedWorkspaceId;
  const usable = availableWorkspaces().some(({ workspace }) => workspace.id === workspaceId);
  if (id === "newWorkspace") return projects.projects.some((p) => !p.missing);
  if (id === "newTerminal")
    return (
      usable &&
      !projects.composingProjectId &&
      !projects.composingWorkspaceId &&
      !projects.workflowId &&
      !projects.usageOpen
    );
  if (["closeTerminal", "previousTerminal", "nextTerminal"].includes(id))
    return (
      !!workspaceId &&
      !!useTerminalStore.getState().active[workspaceId] &&
      !projects.composingProjectId &&
      !projects.composingWorkspaceId &&
      !projects.workflowId &&
      !projects.usageOpen
    );
  if (["nextWorkspace", "previousWorkspace"].includes(id)) return availableWorkspaces().length > 0;
  return true;
}
export function runCommand(id: CommandId) {
  if (!commandEnabled(id)) return;
  const layout = useLayoutStore.getState();
  const projects = useProjectsStore.getState();
  const terminals = useTerminalStore.getState();
  const workspaceId = projects.selectedWorkspaceId;
  switch (id) {
    case "palette":
      layout.setPaletteOpen(true);
      break;
    case "settings":
      layout.openSettings("harnesses");
      break;
    case "shortcuts":
      layout.openSettings("keyboard");
      break;
    case "usage":
      projects.openUsage(true);
      break;
    case "tour":
      layout.setTourOpen(true);
      break;
    case "openProject":
      void openProjectFromDisk();
      break;
    case "newWorkspace":
      composeInCurrentProject();
      break;
    case "newTerminal":
      if (workspaceId) void terminals.open(workspaceId);
      break;
    case "closeTerminal":
      if (workspaceId && terminals.active[workspaceId])
        void terminals.close(terminals.active[workspaceId]!);
      break;
    case "toggleLeft":
      layout.toggle("left");
      break;
    case "toggleRight":
      layout.toggle("right");
      break;
    case "focusProjects":
      focusPanel("Projects");
      break;
    case "focusWorkspace":
      focusPanel("Workspace");
      break;
    case "focusChanges":
      focusPanel("Changes");
      break;
    case "previousWorkspace":
    case "nextWorkspace": {
      const items = availableWorkspaces();
      const index = items.findIndex(({ workspace }) => workspace.id === workspaceId);
      const next =
        index < 0
          ? id === "nextWorkspace"
            ? 0
            : items.length - 1
          : (index + (id === "nextWorkspace" ? 1 : -1) + items.length) % items.length;
      selectWorkspace(items[next]!.workspace.id);
      break;
    }
    case "previousTerminal":
    case "nextTerminal": {
      const tabs = terminals.tabs.filter((t) => t.workspaceId === workspaceId);
      const index = tabs.findIndex((t) => t.id === terminals.active[workspaceId!]);
      const next = tabs[(index + (id === "nextTerminal" ? 1 : -1) + tabs.length) % tabs.length];
      if (next) terminals.activate(next.id);
      break;
    }
  }
}
