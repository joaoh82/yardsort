import {
  composeInCurrentProject,
  enterWorkspace,
  openProjectFromDisk,
} from "@/features/sidebar/actions";
import { startShellThere } from "@/features/remote/there";
import type { OtherWorkspace } from "@/lib/ipc";
import type { CommandId } from "@/lib/shortcuts";
import { useCoordinatorStore } from "@/stores/coordinator";
import { useLayoutStore } from "@/stores/layout";
import { findThere, tabsThere, thereKey, useOtherComputersStore } from "@/stores/otherComputers";
import { type RemoteSelection, useProjectsStore } from "@/stores/projects";
import { useSmokeTestsStore } from "@/stores/smokeTests";
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
/** The workspaces on other computers that can be opened, in their tree's order. */
export function workspacesThere(): RemoteSelection[] {
  const others = useOtherComputersStore.getState();
  const hosts = others.status?.client
    ? others.status.hosts.filter((host) => host.pairing === "paired")
    : [];
  return hosts.flatMap((host) =>
    (others.views[host.hostId]?.projects ?? []).flatMap((project) =>
      project.workspaces
        .filter((w) => !w.archived && !w.missing)
        .map((workspace) => ({ hostId: host.hostId, workspaceId: workspace.id })),
    ),
  );
}
/** The workspace on another computer that is in view, with what its computer showed. */
function inViewThere() {
  const { remote } = useProjectsStore.getState();
  return remote
    ? findThere(useOtherComputersStore.getState().views, remote.hostId, remote.workspaceId)
    : null;
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
  // A workspace on another computer is in view, or one is being composed there: what acts on
  // this computer's own selection, hidden beneath it, is not offered.
  const there = projects.remote !== null || projects.composingThere !== null;
  if (id === "newWorkspace") {
    if (!projects.remote) return projects.projects.some((p) => !p.missing);
    const view = inViewThere()?.view;
    return !!view?.mayStart && view.link.kind === "online";
  }
  if (id === "previousTerminal" || id === "nextTerminal") {
    if (projects.remote) return tabsThere(inViewThere()?.workspace ?? NO_TABS).length > 1;
  }
  // In a workspace on another computer, a shell is that computer's to start, when it can.
  if (id === "newTerminal" && projects.remote) {
    const view = inViewThere()?.view;
    return !!view?.mayManage && view.link.kind === "online";
  }
  if (id === "newTerminal")
    return (
      usable &&
      !there &&
      !projects.composingProjectId &&
      !projects.composingWorkspaceId &&
      !projects.workflowId &&
      !projects.usageOpen &&
      !projects.pullRequestsOpen &&
      !projects.tasksOpen &&
      !projects.testsOpen &&
      !projects.schedulesOpen &&
      !projects.triggersOpen &&
      !projects.notificationsOpen
    );
  if (["closeTerminal", "previousTerminal", "nextTerminal"].includes(id))
    return (
      !!workspaceId &&
      (!!useTerminalStore.getState().active[workspaceId] ||
        (id === "closeTerminal" && !!useSmokeTestsStore.getState().shown[workspaceId])) &&
      !there &&
      !projects.composingProjectId &&
      !projects.composingWorkspaceId &&
      !projects.workflowId &&
      !projects.usageOpen &&
      !projects.pullRequestsOpen &&
      !projects.tasksOpen &&
      !projects.testsOpen &&
      !projects.schedulesOpen &&
      !projects.triggersOpen &&
      !projects.notificationsOpen
    );
  if (["nextWorkspace", "previousWorkspace"].includes(id))
    return availableWorkspaces().length + workspacesThere().length > 0;
  return true;
}
/** A workspace with nothing in it, for one that is not shown any more. */
const NO_TABS: Pick<OtherWorkspace, "sessions" | "terminals"> = { sessions: [], terminals: [] };
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
    case "pullRequests":
      projects.openPullRequests(true);
      break;
    case "tasks":
      projects.openTasks(true);
      break;
    case "tests":
      projects.openTests(true);
      break;
    case "schedules":
      projects.openSchedules(true);
      break;
    case "triggers":
      projects.openTriggers(true);
      break;
    case "notifications":
      projects.openNotifications(true);
      break;
    case "startFromLink":
      projects.openLink(true);
      break;
    case "floorManager":
      useCoordinatorStore.getState().toggle();
      break;
    case "tour":
      layout.setTourOpen(true);
      break;
    case "openProject":
      void openProjectFromDisk();
      break;
    case "newWorkspace": {
      // From a workspace on another computer, the new one is made there too, in its project.
      const there = inViewThere();
      if (there && projects.remote)
        projects.composeThere({ hostId: projects.remote.hostId, projectId: there.project.id });
      else composeInCurrentProject();
      break;
    }
    case "newTerminal":
      if (projects.remote) {
        void startShellThere(projects.remote.hostId, projects.remote.workspaceId);
      } else if (workspaceId) void terminals.open(workspaceId);
      break;
    case "closeTerminal": {
      if (!workspaceId) break;
      // A smoke test run's tab in front is the one to close; the terminal behind it stays.
      const smoke = useSmokeTestsStore.getState();
      const shownRun = smoke.shown[workspaceId];
      if (shownRun) smoke.closeRun(workspaceId, shownRun);
      else if (terminals.active[workspaceId]) void terminals.close(terminals.active[workspaceId]!);
      break;
    }
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
      // This computer's workspaces, then the other computers', as the sidebar lists them.
      const here = availableWorkspaces();
      const there = workspacesThere();
      const count = here.length + there.length;
      const remote = projects.remote;
      const index = remote
        ? there.findIndex(
            (w) => w.hostId === remote.hostId && w.workspaceId === remote.workspaceId,
          ) + here.length
        : here.findIndex(({ workspace }) => workspace.id === workspaceId);
      const lost = remote ? index < here.length : index < 0;
      const next = lost
        ? id === "nextWorkspace"
          ? 0
          : count - 1
        : (index + (id === "nextWorkspace" ? 1 : -1) + count) % count;
      if (next < here.length) selectWorkspace(here[next]!.workspace.id);
      else {
        const target = there[next - here.length]!;
        projects.selectRemote(target.hostId, target.workspaceId);
      }
      break;
    }
    case "previousTerminal":
    case "nextTerminal": {
      if (projects.remote) {
        const { hostId, workspaceId: thereId } = projects.remote;
        const others = useOtherComputersStore.getState();
        const tabs = tabsThere(inViewThere()?.workspace ?? NO_TABS);
        const front = others.active[thereKey(hostId, thereId)] ?? tabs[0]?.id;
        const index = tabs.findIndex((t) => t.id === front);
        const next = tabs[(index + (id === "nextTerminal" ? 1 : -1) + tabs.length) % tabs.length];
        if (next) others.activate(hostId, thereId, next.id);
        break;
      }
      const tabs = terminals.tabs.filter((t) => t.workspaceId === workspaceId);
      const index = tabs.findIndex((t) => t.id === terminals.active[workspaceId!]);
      const next = tabs[(index + (id === "nextTerminal" ? 1 : -1) + tabs.length) % tabs.length];
      if (next) terminals.activate(next.id);
      break;
    }
  }
}
