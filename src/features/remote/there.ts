import type { Activity } from "@/features/terminal/activity";
import { linkText } from "@/features/sidebar/otherComputersText";
import {
  errorMessage,
  ipc,
  type OtherHost,
  type OtherHostView,
  type OtherStarted,
} from "@/lib/ipc";
import { isIpcError } from "@/lib/ipc";
import { native } from "@/lib/native";
import { thereKey, type ThereTab, useOtherComputersStore } from "@/stores/otherComputers";
import { recall, useProjectsStore } from "@/stores/projects";

/** Which project on another computer was chosen for one of this computer's, by `thereKey` of
 *  that computer's id and this project's: kept on this computer, and offered again. */
export const PROJECT_THERE = "composer.projectThere";

/** A project there offered for one of this computer's, and why that one. */
export interface OfferedThere {
  projectId: string | null;
  how: "remembered" | "same repository" | "same name" | null;
}

/**
 * The project on another computer to offer for one here: the one chosen for it last time, if that
 * computer still has it, or else the one that is the same repository, or else the one of the same
 * name. A local project and one there are
 * separate records, so this is an offer the composer shows, never a silent pick. `local.id` is
 * `null` when only a name is known.
 */
export function offeredThere(
  view: OtherHostView | null | undefined,
  ui: Record<string, string>,
  hostId: string,
  local: { id: string | null; name: string; repository?: string | null },
): OfferedThere {
  const projects = view?.projects ?? [];
  const kept = local.id
    ? recall<Record<string, string>>(ui, PROJECT_THERE, {})[thereKey(hostId, local.id)]
    : undefined;
  if (kept && projects.some((project) => project.id === kept)) {
    return { projectId: kept, how: "remembered" };
  }
  // The same repository, when both computers said where theirs is fetched from: surer than a name.
  const same = local.repository
    ? projects.find((project) => project.repository === local.repository)
    : undefined;
  if (same) return { projectId: same.id, how: "same repository" };
  const named =
    projects.find((project) => project.name === local.name) ??
    projects.find((project) => project.name.toLowerCase() === local.name.toLowerCase());
  return named ? { projectId: named.id, how: "same name" } : { projectId: null, how: null };
}

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

/** That computer's shell in a workspace there. */
export function startShellThere(hostId: string, workspaceId: string) {
  return startThere(hostId, () => ipc.otherHostStartShell(hostId, workspaceId));
}

/** Rename a workspace there. Resolves to why it could not be, or `null`. */
export async function renameThere(
  hostId: string,
  workspaceId: string,
  name: string,
): Promise<string | null> {
  try {
    await ipc.otherHostRenameWorkspace(hostId, workspaceId, name);
    void useOtherComputersStore.getState().loadHost(hostId);
    return null;
  } catch (error) {
    return errorMessage(error);
  }
}

/**
 * Archive a workspace on another computer, or with `remove` delete it: its folder goes there, its
 * branch stays. Asked first, as for a local one, and uncommitted work there is never destroyed
 * without a second confirmation that says so. A failure is shown where the sidebar shows them.
 */
export async function removeThere(
  hostId: string,
  hostName: string,
  workspace: { id: string; name: string; branch: string | null },
  remove: boolean,
): Promise<void> {
  const doing = remove ? "Delete" : "Archive";
  try {
    const agreed = await native.confirm(
      `${doing} workspace "${workspace.name}" on ${hostName}?\n\nIts folder is removed from ${hostName}. ` +
        (workspace.branch
          ? `The branch "${workspace.branch}" and all its commits are kept there.`
          : "Its commits stay in the repository there.") +
        (remove
          ? ""
          : " Its conversations are kept too, and it can be restored at that computer.") +
        " Terminals running in it are not stopped by this.",
      { title: `${doing} workspace on ${hostName}`, okLabel: doing },
    );
    if (!agreed) return;
    const ask = (force: boolean) =>
      ipc.otherHostRemoveWorkspace(hostId, workspace.id, remove, force);
    try {
      await ask(false);
    } catch (error) {
      if (!isIpcError(error) || error.code !== "remote_uncommitted") throw error;
      const force = await native.confirm(
        `"${workspace.name}" has uncommitted changes or untracked files on ${hostName}.\n\n` +
          `${remove ? "Deleting" : "Archiving"} it now destroys that work for good: it is in no commit and cannot be recovered.`,
        { title: "Uncommitted work will be lost", okLabel: `${doing} anyway` },
      );
      if (!force) return;
      await ask(true);
    }
    // It is gone from that computer: nothing of it stays in view here.
    const projects = useProjectsStore.getState();
    if (projects.remote?.hostId === hostId && projects.remote.workspaceId === workspace.id) {
      useProjectsStore.setState({ remote: null });
    }
    void useOtherComputersStore.getState().loadHost(hostId);
  } catch (error) {
    useProjectsStore.setState({ error: errorMessage(error) });
  }
}

/** An ended conversation there again, under its own record. */
export function resumeThere(hostId: string, sessionId: string) {
  return startThere(hostId, () => ipc.otherHostResume(hostId, sessionId, null));
}

/** Whether this computer reaches other computers at all: without that, a composer has no menu. */
export function useRunsElsewhere(): boolean {
  return useOtherComputersStore((s) => !!s.status?.client);
}

/**
 * Why a new workspace cannot be started on a computer from here just now, in the words its row in
 * the menu shows, or `null` when it can. Nothing is queued for one that is away, so it is listed
 * and not offered.
 */
export function whyNotThere(
  host: OtherHost,
  view: OtherHostView | null | undefined,
): string | null {
  const link = host.link;
  if (!link) return "not connected";
  if (link.kind === "offline") return link.since === null ? "offline" : linkText(link);
  if (link.kind === "connecting") return linkText(link);
  if (!view || view.readAt === null) return "not read yet";
  if (!view.mayType) return "lets this computer watch only";
  if (!view.mayStart) return "needs a newer Yardsort to start work from here";
  return null;
}
