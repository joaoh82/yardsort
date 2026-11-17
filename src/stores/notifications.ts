import { create } from "zustand";
import { errorMessage, hasCore, ipc, type NotificationItem } from "@/lib/ipc";
import { useTerminalStore } from "./terminals";

/** How the view groups its rows. */
export type Grouping = "kind" | "project";
export const GROUPING_KEY = "notifications.grouping";

interface NotificationsState {
  /** The list as the core keeps it: what is still in its part, newest first, then Earlier. */
  items: NotificationItem[];
  loaded: boolean;
  error: string | null;
  grouping: Grouping;
  /** The terminal the center panel shows right now, when it shows one. */
  inFront: string | null;

  load: () => Promise<void>;
  /** These finished rows were looked at. */
  seen: (ids: string[]) => Promise<void>;
  /** Everything finished is seen. */
  seenAll: () => Promise<void>;
  /** The center panel shows this terminal, or none. What its agent finished is seen. */
  looking: (terminalId: string | null) => void;
  setGrouping: (grouping: Grouping) => void;
  dismissError: () => void;
}

/** Whether the person can see `terminalId` right now. */
function inFrontOfSomeone(terminalId: string | null, inFront: string | null): boolean {
  return terminalId !== null && terminalId === inFront && document.hasFocus();
}

/** Rows that have left their part, as the core will say they have once it is asked. */
function settled(items: NotificationItem[], gone: (item: NotificationItem) => boolean) {
  const now = Date.now();
  return items.map((item) =>
    item.part === "finished" && gone(item)
      ? { ...item, part: "earlier" as const, settledAt: now }
      : item,
  );
}

export const useNotificationsStore = create<NotificationsState>((set, get) => ({
  items: [],
  loaded: false,
  error: null,
  grouping: "kind",
  inFront: null,

  async load() {
    if (!hasCore()) return;
    try {
      const listed = await ipc.notificationList();
      // A row made while its terminal is in front of someone has been seen already.
      const { inFront } = get();
      const watched = listed.filter(
        (item) => item.part === "finished" && inFrontOfSomeone(item.terminalId, inFront),
      );
      for (const terminalId of new Set(watched.map((item) => item.terminalId)))
        if (terminalId) void ipc.notificationSeenTerminal(terminalId).catch(console.error);
      const items = settled(listed, (item) => watched.includes(item));
      set({ items, loaded: true });
      ring(items);
    } catch (error) {
      set({ error: errorMessage(error), loaded: true });
    }
  },

  async seen(ids) {
    if (ids.length === 0) return;
    const items = settled(get().items, (item) => ids.includes(item.id));
    set({ items });
    ring(items);
    try {
      await ipc.notificationSeen(ids);
    } catch (error) {
      set({ error: errorMessage(error) });
      void get().load();
    }
  },

  async seenAll() {
    const items = settled(get().items, () => true);
    set({ items });
    ring(items);
    try {
      await ipc.notificationSeenAll();
    } catch (error) {
      set({ error: errorMessage(error) });
      void get().load();
    }
  },

  looking(terminalId) {
    set({ inFront: terminalId });
    if (!inFrontOfSomeone(terminalId, terminalId)) return;
    const unseen = get().items.some(
      (item) => item.part === "finished" && item.terminalId === terminalId,
    );
    if (!unseen || terminalId === null) return;
    const items = settled(get().items, (item) => item.terminalId === terminalId);
    set({ items });
    ring(items);
    void ipc.notificationSeenTerminal(terminalId).catch(console.error);
  },

  setGrouping(grouping) {
    set({ grouping });
    if (hasCore()) void ipc.uiStateSave(GROUPING_KEY, grouping).catch(console.error);
  },

  dismissError: () => set({ error: null }),
}));

/** Draw the "finished, not seen yet" ring on the tabs the list says are unseen, and on no
 *  other: the list is what knows, and it outlives the window. */
function ring(items: NotificationItem[]) {
  const unseen = new Set<string>();
  for (const item of items)
    if (item.part === "finished" && item.terminalId) unseen.add(item.terminalId);
  useTerminalStore.getState().ringOnly(unseen);
}

/** How many rows need an answer, and how many finished and were not seen: the two numbers on
 *  the sidebar's row. */
export function counts(items: NotificationItem[]): { needs: number; finished: number } {
  let needs = 0;
  let finished = 0;
  for (const item of items) {
    if (item.part === "needs") needs += 1;
    else if (item.part === "finished") finished += 1;
  }
  return { needs, finished };
}

/** How many rows of `projectId` are still in their part of the list: the number on a project's
 *  row in the sidebar. */
export function listedIn(items: NotificationItem[], projectId: string): number {
  return items.filter((item) => item.projectId === projectId && item.part !== "earlier").length;
}

/** Keep the list current as the core changes it. Mount once. */
export function listenForNotifications(): Promise<() => void> {
  if (!hasCore()) return Promise.resolve(() => {});
  const store = useNotificationsStore;
  void ipc
    .uiStateLoad()
    .then((ui) => {
      if (ui[GROUPING_KEY] === "project") store.setState({ grouping: "project" });
    })
    .catch(console.error);
  void store.getState().load();
  // A tab that appears after the list was read — the window reloaded, an agent was resumed —
  // gets its ring from what is already known.
  const stopTabs = useTerminalStore.subscribe((state, before) => {
    if (state.tabs.length !== before.tabs.length && store.getState().loaded)
      ring(store.getState().items);
  });
  return ipc
    .onNotificationsChanged(() => void store.getState().load())
    .then((stop) => () => {
      stopTabs();
      stop();
    });
}
