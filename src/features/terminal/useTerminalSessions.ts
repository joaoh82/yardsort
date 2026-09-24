import { useEffect } from "react";
import { hasCore, ipc, type HostEvent } from "@/lib/ipc";
import { native } from "@/lib/native";
import { useActivityStore } from "@/stores/activity";
import { useAppStore } from "@/stores/app";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";

/** Whether the user can see this terminal right now. */
function isWatched(id: string): boolean {
  if (!document.hasFocus()) return false;
  const { selectedWorkspaceId, composingProjectId } = useProjectsStore.getState();
  if (!selectedWorkspaceId || composingProjectId !== null) return false;
  return useTerminalStore.getState().active[selectedWorkspaceId] === id;
}

function announce(tab: TerminalTab) {
  if (document.hasFocus() || !useAppStore.getState().notifyWhenQuiet) return;
  for (const project of useProjectsStore.getState().projects) {
    const workspace = project.workspaces.find((w) => w.id === tab.workspaceId);
    if (!workspace) continue;
    const where = `${project.name} / ${workspace.name}`;
    void native.notify(`${tab.title} is waiting`, where).catch(console.error);
    return;
  }
}

export function handleHostEvent(event: HostEvent) {
  const terminals = useTerminalStore.getState();
  switch (event.type) {
    case "busy":
      return terminals.markBusy(event.id);
    case "quiet": {
      const notable = terminals.markQuiet(event.id, event.busyMs, isWatched(event.id));
      if (notable) announce(notable);
      return;
    }
    case "exited": {
      const tab = terminals.tabs.find((t) => t.id === event.id);
      terminals.markExited(event.id, event.exit);
      // The core has already settled the record; refresh what Resume and Fork are offered on.
      // A tab the user closed is gone by the time its process dies, so its workspace is no
      // longer known here — refresh every history on screen rather than leave one stale.
      const sessions = useSessionsStore.getState();
      const stale = tab
        ? tab.recordId
          ? [tab.workspaceId]
          : []
        : Object.keys(sessions.byWorkspace);
      for (const workspaceId of stale) void sessions.load(workspaceId);
      // An exit is an event on the timeline, if one is open for that workspace.
      if (tab) void useActivityStore.getState().refresh(tab.workspaceId);
    }
  }
}

/** Keep the tab list in step with the sessions the core owns. Mount once. */
export function useTerminalSessions() {
  useEffect(() => {
    if (!hasCore()) return;
    void useTerminalStore.getState().hydrate().catch(console.error);
    const unlisten = ipc.onHostEvent(handleHostEvent);
    return () => void unlisten.then((stop) => stop());
  }, []);
}
