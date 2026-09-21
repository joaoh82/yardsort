import { create } from "zustand";
import type { RendererKind } from "@/features/terminal/renderer";
import {
  errorMessage,
  HARNESS_LABEL,
  ipc,
  RECORD_LABEL,
  WORKSPACE_LABEL,
  type ExitInfo,
  type HarnessRequest,
  type SessionId,
  type SessionInfo,
  type TermSize,
} from "@/lib/ipc";

export interface TerminalTab {
  id: SessionId;
  /** The workspace this session runs in. */
  workspaceId: string;
  title: string;
  /** Set once the process has ended. The tab stays so its output can still be read. */
  exit: ExitInfo | null;
  /** The harness conversation this terminal runs, if it is one. Shells have none. */
  recordId: string | null;
  /** Printing right now, as opposed to sitting at a prompt. */
  busy: boolean;
  /** Finished a long stretch of work while nobody was watching; cleared by looking at it. */
  attention: boolean;
}

/** A burst of output shorter than this is somebody typing, not an agent finishing work. */
export const WORTH_NOTICING_MS = 8_000;

interface TerminalState {
  tabs: TerminalTab[];
  /** The tab showing in each workspace. */
  active: Record<string, SessionId | undefined>;
  error: string | null;
  renderer: RendererKind | null;
  /** Size of the most recently fitted terminal, so new sessions start at the right size. */
  lastSize: TermSize;

  /** Adopt sessions already running in the core (e.g. after the webview reloads). */
  hydrate: () => Promise<void>;
  /** Start a harness — or the user's shell when omitted — in a workspace. */
  open: (workspaceId: string, harness?: HarnessRequest) => Promise<void>;
  /** Show a session the core started on our behalf (a new workspace's first harness). */
  adopt: (session: SessionInfo) => void;
  close: (id: SessionId) => Promise<void>;
  /** Close every session of the given workspaces, e.g. when their project is removed. */
  closeWorkspaces: (workspaceIds: string[]) => Promise<void>;
  /** Continue an ended conversation in a new terminal, replacing `replacing` if given. */
  resume: (recordId: string, replacing?: SessionId) => Promise<boolean>;
  /** Start a copy of a conversation in a new terminal next to the others. */
  fork: (recordId: string) => Promise<boolean>;
  activate: (id: SessionId) => void;
  markExited: (id: SessionId, exit: ExitInfo) => void;
  markBusy: (id: SessionId) => void;
  /** Returns the tab if this was work worth telling the user about, else `null`. */
  markQuiet: (id: SessionId, busyMs: number, watched: boolean) => TerminalTab | null;
  setRenderer: (kind: RendererKind) => void;
  setLastSize: (size: TermSize) => void;
  dismissError: () => void;
}

/**
 * Exits seen for sessions that have no tab yet. A program can exit before `ptySpawn` has even
 * returned, in which case the event overtakes the tab it belongs to.
 */
const earlyExits = new Map<SessionId, ExitInfo>();

function tabFor(session: SessionInfo, adopting = false): TerminalTab | null {
  const workspaceId = session.labels[WORKSPACE_LABEL];
  if (!workspaceId) return null;
  const name = session.program.split(/[\\/]/).pop() ?? session.program;
  return {
    id: session.id,
    workspaceId,
    title: session.labels[HARNESS_LABEL] ?? name.replace(/\.(exe|cmd|bat)$/i, ""),
    exit:
      session.state.status === "exited" ? session.state.exit : (earlyExits.get(session.id) ?? null),
    recordId: session.labels[RECORD_LABEL] ?? null,
    busy: session.busy && session.state.status === "running",
    // A session adopted rather than started by us has been running unwatched — since a webview
    // reload, or since the last time the app was open. If it is an agent that has printed
    // something and is now quiet, that is work waiting to be looked at.
    attention: adopting && isWaitingAgent(session),
  };
}

/** A harness that has said something and then fallen silent: it is waiting for a person. */
function isWaitingAgent(session: SessionInfo): boolean {
  return (
    session.labels[HARNESS_LABEL] !== undefined &&
    session.state.status === "running" &&
    session.hasOutput &&
    !session.busy
  );
}

const update = (tabs: TerminalTab[], id: SessionId, patch: Partial<TerminalTab>) =>
  tabs.map((tab) => (tab.id === id ? { ...tab, ...patch } : tab));

export const useTerminalStore = create<TerminalState>((set, get) => ({
  tabs: [],
  active: {},
  error: null,
  renderer: null,
  lastSize: { cols: 80, rows: 24 },

  async hydrate() {
    const sessions = await ipc.ptyList();
    const known = new Set(get().tabs.map((tab) => tab.id));
    const adopted = sessions
      .filter((session) => !known.has(session.id))
      .map((session) => tabFor(session, true))
      .filter((tab) => tab !== null);
    if (adopted.length === 0) return;
    set((state) => {
      const active = { ...state.active };
      for (const tab of adopted) active[tab.workspaceId] ??= tab.id;
      return { tabs: [...state.tabs, ...adopted], active };
    });
  },

  async open(workspaceId, harness) {
    try {
      const session = await ipc.ptySpawn({
        program: null,
        args: [],
        cwd: null,
        workspaceId,
        harness: harness ?? null,
        size: get().lastSize,
      });
      get().adopt(session);
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  adopt(session) {
    const tab = tabFor(session);
    if (!tab) return console.error("Session without a workspace label:", session.id);
    set((state) => ({
      tabs: state.tabs.some((known) => known.id === tab.id) ? state.tabs : [...state.tabs, tab],
      active: { ...state.active, [tab.workspaceId]: tab.id },
      error: null,
    }));
  },

  async close(id) {
    set((state) => {
      const closing = state.tabs.find((tab) => tab.id === id);
      if (!closing) return state;
      const siblings = state.tabs.filter((tab) => tab.workspaceId === closing.workspaceId);
      const index = siblings.findIndex((tab) => tab.id === id);
      const remaining = siblings.filter((tab) => tab.id !== id);
      const active = { ...state.active };
      if (active[closing.workspaceId] === id) {
        active[closing.workspaceId] = remaining[Math.min(index, remaining.length - 1)]?.id;
      }
      return { tabs: state.tabs.filter((tab) => tab.id !== id), active };
    });
    await ipc.ptyClose(id).catch(() => {});
  },

  async closeWorkspaces(workspaceIds) {
    const gone = new Set(workspaceIds);
    const closing = get().tabs.filter((tab) => gone.has(tab.workspaceId));
    set((state) => ({
      tabs: state.tabs.filter((tab) => !gone.has(tab.workspaceId)),
      active: Object.fromEntries(Object.entries(state.active).filter(([id]) => !gone.has(id))),
    }));
    await Promise.all(closing.map((tab) => ipc.ptyClose(tab.id).catch(() => {})));
  },

  async resume(recordId, replacing) {
    try {
      const session = await ipc.sessionResume(recordId, get().lastSize);
      if (replacing) await get().close(replacing);
      get().adopt(session);
      return true;
    } catch (error) {
      set({ error: errorMessage(error) });
      return false;
    }
  },

  async fork(recordId) {
    try {
      get().adopt(await ipc.sessionFork(recordId, get().lastSize));
      return true;
    } catch (error) {
      set({ error: errorMessage(error) });
      return false;
    }
  },

  activate: (id) =>
    set((state) => {
      const tab = state.tabs.find((candidate) => candidate.id === id);
      if (!tab) return state;
      return {
        active: { ...state.active, [tab.workspaceId]: id },
        tabs: update(state.tabs, id, { attention: false }),
      };
    }),
  markExited: (id, exit) => {
    if (!get().tabs.some((tab) => tab.id === id)) earlyExits.set(id, exit);
    set((state) => ({ tabs: update(state.tabs, id, { exit, busy: false }) }));
  },
  markBusy: (id) => set((state) => ({ tabs: update(state.tabs, id, { busy: true }) })),
  markQuiet(id, busyMs, watched) {
    const tab = get().tabs.find((candidate) => candidate.id === id);
    if (!tab) return null;
    // Only agents finishing real work are news; shells and keystroke echoes are not.
    const notable = tab.recordId !== null && busyMs >= WORTH_NOTICING_MS && !watched;
    set((state) => ({
      tabs: update(state.tabs, id, { busy: false, attention: tab.attention || notable }),
    }));
    return notable ? tab : null;
  },
  setRenderer: (renderer) => set({ renderer }),
  setLastSize: (lastSize) => set({ lastSize }),
  dismissError: () => set({ error: null }),
}));
