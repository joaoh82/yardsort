import { create } from "zustand";
import {
  errorMessage,
  hasCore,
  ipc,
  type OtherComputers,
  type OtherHarness,
  type OtherHostView,
  type OtherProject,
  type OtherStarted,
  type OtherWorkspace,
} from "@/lib/ipc";

interface OtherComputersState {
  /** This computer as a client of other hosts, and the hosts it asked to reach. */
  status: OtherComputers | null;
  /** What each paired host showed when last read, by its id. */
  views: Record<string, OtherHostView | null>;
  /** The agents each host can start, by its id, once it was asked. */
  harnesses: Record<string, OtherHarness[]>;
  /** The terminal in front in each workspace there, by `thereKey`. */
  active: Record<string, string>;
  /** Terminals started from this computer that its computer has not listed yet, by `thereKey`,
   *  with when each was started: shown at once, until a reading lists it or a later one does not. */
  startedHere: Record<string, number>;
  /** Those of them whose control is still to be asked for, which is done once, by itself. */
  taking: string[];
  error: string | null;
  /** Why work could not be started on another computer, until it is dismissed or one starts. */
  startProblem: string | null;
  /** Read the hosts, and what each one this computer is let into shows. */
  load: () => Promise<void>;
  /** Read what one host shows, as the core last read it. */
  loadHost: (hostId: string) => Promise<void>;
  /** Ask a host which agents it can start. What it said before stays if it cannot be asked. */
  loadHarnesses: (hostId: string) => Promise<void>;
  /** Put a terminal in front in its workspace there. */
  activate: (hostId: string, workspaceId: string, terminalId: string) => void;
  /** An agent was started there from this computer: in front, and this computer's to type into. */
  started: (hostId: string, started: OtherStarted) => void;
  /** Control of a terminal started here was asked for: it is not asked for by itself again. */
  asked: (hostId: string, terminalId: string) => void;
  /** Say why a start there failed, or take that away with `null`. */
  couldNotStart: (why: string | null) => void;
  /** An action that answers with the new status: shown, or its failure said. */
  run: (action: () => Promise<OtherComputers>) => Promise<boolean>;
}

/** One thing on another computer, by that computer's id and its own. */
export const thereKey = (hostId: string, id: string) => `${hostId}/${id}`;

/**
 * Remote the other way round: the hosts this computer reaches as a client, which the sidebar lists
 * as a tree and Settings → Remote manages. The core keeps the links and reads each host; this
 * mirrors it, and holds what only the window knows: which terminal there is in front.
 */
export const useOtherComputersStore = create<OtherComputersState>((set, get) => ({
  status: null,
  views: {},
  harnesses: {},
  active: {},
  startedHere: {},
  taking: [],
  error: null,
  startProblem: null,

  async load() {
    try {
      const status = await ipc.otherComputers();
      set({ status, error: null });
      const letIn = status.client ? status.hosts.filter((h) => h.pairing === "paired") : [];
      // A host no longer paired with is not shown as it last was.
      set((state) => ({
        views: Object.fromEntries(
          Object.entries(state.views).filter(([id]) => letIn.some((h) => h.hostId === id)),
        ),
      }));
      await Promise.all(letIn.map((host) => get().loadHost(host.hostId)));
    } catch (reason) {
      set({ error: errorMessage(reason) });
    }
  },

  async loadHost(hostId) {
    try {
      const view = await ipc.otherHost(hostId);
      set((state) => {
        const settled = Object.entries(state.startedHere)
          .filter(([key, at]) => key.startsWith(`${hostId}/`) && !awaited(view, hostId, key, at))
          .map(([key]) => key);
        if (settled.length === 0) return { views: { ...state.views, [hostId]: view } };
        return {
          views: { ...state.views, [hostId]: view },
          startedHere: Object.fromEntries(
            Object.entries(state.startedHere).filter(([key]) => !settled.includes(key)),
          ),
          taking: state.taking.filter((key) => !settled.includes(key) || lists(view, hostId, key)),
        };
      });
    } catch (reason) {
      set({ error: errorMessage(reason) });
    }
  },

  async loadHarnesses(hostId) {
    try {
      const listed = await ipc.otherHostHarnesses(hostId);
      set((state) => ({ harnesses: { ...state.harnesses, [hostId]: listed } }));
    } catch {
      // Not reachable just now: the view says so, and what it listed before is still offered.
    }
  },

  activate(hostId, workspaceId, terminalId) {
    set((state) => ({
      active: { ...state.active, [thereKey(hostId, workspaceId)]: terminalId },
    }));
  },

  couldNotStart(why) {
    set({ startProblem: why });
  },

  started(hostId, started) {
    set((state) => ({
      startProblem: null,
      active: { ...state.active, [thereKey(hostId, started.workspaceId)]: started.terminal },
      startedHere: { ...state.startedHere, [thereKey(hostId, started.terminal)]: Date.now() },
      taking: [...state.taking, thereKey(hostId, started.terminal)],
    }));
    void get().loadHost(hostId);
  },

  asked(hostId, terminalId) {
    const key = thereKey(hostId, terminalId);
    set((state) => ({ taking: state.taking.filter((k) => k !== key) }));
  },

  async run(action) {
    set({ error: null });
    try {
      set({ status: await action() });
      return true;
    } catch (reason) {
      // Where things stand now, then why the action failed: reading them again must not hide it.
      await get().load();
      set({ error: errorMessage(reason) });
      return false;
    }
  },
}));

/**
 * How long after a start a reading may still have been taken before it. A reading newer than this
 * that does not list the terminal means it is not there any more, not that it is still to come.
 */
const NOT_YET_MS = 30_000;

/** Whether what `hostId` showed lists the terminal `key` names. */
function lists(view: OtherHostView | null, hostId: string, key: string): boolean {
  return (view?.projects ?? []).some((project) =>
    project.workspaces.some((workspace) =>
      tabsThere(workspace).some((tab) => thereKey(hostId, tab.id) === key),
    ),
  );
}

/** Whether a terminal started here at `at` is still to be listed by its computer's readings. */
function awaited(view: OtherHostView | null, hostId: string, key: string, at: number): boolean {
  if (lists(view, hostId, key)) return false;
  const readAt = view?.readAt ?? null;
  return readAt === null || readAt <= at + NOT_YET_MS;
}

/** A workspace on another computer, with its project and what that computer showed. */
export interface There {
  view: OtherHostView;
  project: OtherProject;
  workspace: OtherWorkspace;
}

/** Find a workspace there in what its computer last showed. */
export function findThere(
  views: Record<string, OtherHostView | null>,
  hostId: string,
  workspaceId: string,
): There | null {
  const view = views[hostId];
  if (!view) return null;
  for (const project of view.projects) {
    const workspace = project.workspaces.find((w) => w.id === workspaceId);
    if (workspace) return { view, project, workspace };
  }
  return null;
}

/** A terminal there as a tab: an agent's, named by its harness, or another program's. */
export interface ThereTab {
  /** Its terminal's id on that computer. */
  id: string;
  title: string;
  /** The harness, when an agent runs in it. */
  harness: string | null;
  /** What its conversation is called there, when it has one. */
  about: string | null;
  /** `shell`, `run`, `agent` or `program`. */
  purpose: string;
  /** `starting`, `busy`, `quiet` or `exited`. */
  state: string;
}

/** A workspace's running terminals there, its agents first, as that computer lists them. */
export function tabsThere(workspace: Pick<OtherWorkspace, "sessions" | "terminals">): ThereTab[] {
  const agents = workspace.sessions.flatMap((session): ThereTab[] =>
    session.terminal
      ? [
          {
            id: session.terminal,
            title: session.harness,
            harness: session.harness,
            about: session.title || null,
            purpose: "agent",
            state: session.state,
          },
        ]
      : [],
  );
  const others = workspace.terminals
    .filter((terminal) => !agents.some((agent) => agent.id === terminal.id))
    .map((terminal): ThereTab => ({
      id: terminal.id,
      title: terminal.program,
      harness: null,
      about: null,
      purpose: terminal.purpose,
      state: terminal.state,
    }));
  return [...agents, ...others];
}

/** How soon after the core says something changed the window looks: links can change in bursts. */
const SETTLE_MS = 250;

/** Follow the core's word that something changed. Mounted once, with the rest of the shell. */
export function listenForOtherComputers(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  void useOtherComputersStore.getState().load();
  let timer: ReturnType<typeof setTimeout> | undefined;
  return ipc.onOtherComputersChanged(() => {
    clearTimeout(timer);
    timer = setTimeout(() => void useOtherComputersStore.getState().load(), SETTLE_MS);
  });
}
