import { create } from "zustand";
import { errorMessage, ipc, type ActivityEvent } from "@/lib/ipc";

/** How many events a page asks for. Small: the panel is a glance, not an archive. */
export const PAGE = 50;

interface Timeline {
  events: ActivityEvent[];
  hasMore: boolean;
}

interface ActivityState {
  /** The newest page (and any earlier ones loaded after it) per workspace. Absent until loaded. */
  byWorkspace: Record<string, Timeline | undefined>;
  loading: Record<string, boolean | undefined>;
  error: string | null;
  /** Load the newest page, replacing whatever was there. */
  load: (workspaceId: string) => Promise<void>;
  /** Load again, but only for a workspace already on screen: an exit came in. */
  refresh: (workspaceId: string) => Promise<void>;
  /** Fetch the page before the oldest one shown. */
  loadEarlier: (workspaceId: string) => Promise<void>;
  /** Forget this workspace's recorded activity. */
  clear: (workspaceId: string) => Promise<void>;
  dismissError: () => void;
}

/** The activity timeline behind the experimental panel. The core owns it; this is a cache. */
export const useActivityStore = create<ActivityState>((set, get) => ({
  byWorkspace: {},
  loading: {},
  error: null,

  async load(workspaceId) {
    set((state) => ({ loading: { ...state.loading, [workspaceId]: true } }));
    try {
      const page = await ipc.activityTimeline(workspaceId, null, PAGE);
      set((state) => ({
        byWorkspace: {
          ...state.byWorkspace,
          [workspaceId]: { events: page.events, hasMore: page.hasMore },
        },
        error: null,
      }));
    } catch (error) {
      set({ error: errorMessage(error) });
    } finally {
      set((state) => ({ loading: { ...state.loading, [workspaceId]: false } }));
    }
  },

  async refresh(workspaceId) {
    if (get().byWorkspace[workspaceId]) await get().load(workspaceId);
  },

  async loadEarlier(workspaceId) {
    const current = get().byWorkspace[workspaceId];
    const oldest = current?.events[current.events.length - 1];
    if (!current || !oldest || !current.hasMore) return;
    set((state) => ({ loading: { ...state.loading, [workspaceId]: true } }));
    try {
      const page = await ipc.activityTimeline(workspaceId, oldest.seq, PAGE);
      set((state) => {
        const known = state.byWorkspace[workspaceId];
        if (!known) return state;
        return {
          byWorkspace: {
            ...state.byWorkspace,
            [workspaceId]: { events: [...known.events, ...page.events], hasMore: page.hasMore },
          },
        };
      });
    } catch (error) {
      set({ error: errorMessage(error) });
    } finally {
      set((state) => ({ loading: { ...state.loading, [workspaceId]: false } }));
    }
  },

  async clear(workspaceId) {
    try {
      await ipc.activityClear(workspaceId);
      await get().load(workspaceId);
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  dismissError: () => set({ error: null }),
}));
