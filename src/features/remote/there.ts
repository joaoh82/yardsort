import type { Activity } from "@/features/terminal/activity";
import { errorMessage, ipc, type OtherStarted } from "@/lib/ipc";
import { type ThereTab, useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";

/** Roll a workspace's terminals there up into one dot, as a local workspace's are. */
export function activityThere(tabs: ThereTab[]): Activity {
  const live = tabs.filter((tab) => tab.state !== "exited");
  if (live.some((tab) => tab.state === "busy")) return "busy";
  return live.length > 0 ? "waiting" : "idle";
}

/** One terminal's dot: printing, running and quiet, or over. */
export function tabActivity(tab: ThereTab): Activity {
  return tab.state === "exited" ? "idle" : tab.state === "busy" ? "busy" : "waiting";
}

/**
 * Start something on another computer and open it: the workspace it runs in is selected, its
 * terminal put in front, and control of it asked for at once, since starting it was this
 * person's act. A start whose answer is lost is not tried again; why it failed is said in the
 * workspace panel.
 */
export async function startThere(
  hostId: string,
  start: () => Promise<OtherStarted>,
): Promise<boolean> {
  const others = useOtherComputersStore.getState();
  others.couldNotStart(null);
  try {
    const started = await start();
    others.started(hostId, started);
    useProjectsStore.getState().selectRemote(hostId, started.workspaceId);
    return true;
  } catch (error) {
    others.couldNotStart(errorMessage(error));
    return false;
  }
}

/** Another agent in a workspace there, waiting to be told what to do. */
export function startAgentThere(hostId: string, workspaceId: string, harness: string | null) {
  return startThere(hostId, () => ipc.otherHostStartSession(hostId, workspaceId, harness, null));
}

/** An ended conversation there again, under its own record. */
export function resumeThere(hostId: string, sessionId: string) {
  return startThere(hostId, () => ipc.otherHostResume(hostId, sessionId, null));
}
