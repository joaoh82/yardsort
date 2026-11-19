import { create } from "zustand";
import { errorMessage, hasCore, ipc, type NotificationItem } from "@/lib/ipc";
import { useTerminalStore } from "./terminals";

/** How the view groups its rows. */
export type Grouping = "kind" | "project";
export const GROUPING_KEY = "notifications.grouping";

/** What tells one row from every other: its id is its own computer's, and two computers may
 *  use the same one. */
export function rowKey(item: Pick<NotificationItem, "id" | "hostId">): string {
  return item.hostId === null ? item.id : `${item.hostId}/${item.id}`;
}

interface NotificationsState {
  /** The list as the core gives it: this computer's rows and those of each other computer it
   *  reaches now. What is still in its part, newest first, then Earlier. */
  items: NotificationItem[];
  loaded: boolean;
  error: string | null;
  grouping: Grouping;
  /** The terminal the center panel shows right now, when it shows one. */
  inFront: string | null;
  /** The workflow run the center panel shows right now, when it shows one. */
  runInFront: string | null;
  /** The terminal on another computer the center panel shows right now, when it shows one. */
  thereInFront: { hostId: string; terminalId: string } | null;

  load: () => Promise<void>;
  /** These finished rows were looked at, by `rowKey`. One on another computer is said there. */
  seen: (keys: string[]) => Promise<void>;
  /** Everything finished is seen. */
  seenAll: () => Promise<void>;
  /** The center panel shows this terminal, or none. What its agent finished is seen. */
  looking: (terminalId: string | null) => void;
  /** The center panel shows this workflow run, or none. That it ended is seen. */
  lookingAtRun: (runId: string | null) => void;
  /** The center panel shows this terminal on another computer, or none. What its agent
   *  finished is seen, and that computer is told. */
  lookingThere: (hostId: string | null, terminalId: string | null) => void;
  setGrouping: (grouping: Grouping) => void;
  dismissError: () => void;
}

/** Whether the person can see this computer's terminal a row names right now. */
function inFrontOfSomeone(item: NotificationItem, inFront: string | null): boolean {
  return (
    item.hostId === null &&
    item.terminalId !== null &&
    item.terminalId === inFront &&
    document.hasFocus()
  );
}

/** Whether the person can see the workflow run a row of this computer names right now. */
function runInFrontOfSomeone(item: NotificationItem, runInFront: string | null): boolean {
  return (
    item.hostId === null &&
    item.subjectId !== null &&
    item.subjectId === runInFront &&
    document.hasFocus()
  );
}

/** Whether the person can see another computer's terminal a row names right now. */
function thereInFrontOfSomeone(
  item: NotificationItem,
  there: { hostId: string; terminalId: string } | null,
): boolean {
  return (
    there !== null &&
    item.hostId === there.hostId &&
    item.terminalId === there.terminalId &&
    document.hasFocus()
  );
}

/** Say that these rows were seen where each is kept: here, or on its own computer. */
function sayWhereKept(rows: NotificationItem[]): Promise<unknown> {
  const here = rows.filter((row) => row.hostId === null).map((row) => row.id);
  const there = new Map<string, string[]>();
  for (const row of rows) {
    if (row.hostId === null) continue;
    there.set(row.hostId, [...(there.get(row.hostId) ?? []), row.id]);
  }
  return Promise.all([
    here.length > 0 ? ipc.notificationSeen(here) : null,
    ...[...there].map(([hostId, ids]) => ipc.notificationSeenThere(hostId, ids)),
  ]);
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
  runInFront: null,
  thereInFront: null,

  async load() {
    if (!hasCore()) return;
    try {
      const listed = await ipc.notificationList();
      // A row made while its terminal is in front of someone has been seen already.
      const { inFront, runInFront, thereInFront } = get();
      const watched = listed.filter(
        (item) => item.part === "finished" && inFrontOfSomeone(item, inFront),
      );
      for (const terminalId of new Set(watched.map((item) => item.terminalId)))
        if (terminalId) void ipc.notificationSeenTerminal(terminalId).catch(console.error);
      // And so has a run that ended while it was the one on screen, and what an agent on
      // another computer finished while its terminal there was.
      const watchedElse = listed.filter(
        (item) =>
          item.part === "finished" &&
          (runInFrontOfSomeone(item, runInFront) || thereInFrontOfSomeone(item, thereInFront)),
      );
      if (watchedElse.length > 0) void sayWhereKept(watchedElse).catch(console.error);
      const items = settled(listed, (item) => watched.includes(item) || watchedElse.includes(item));
      set({ items, loaded: true });
      ring(items);
    } catch (error) {
      set({ error: errorMessage(error), loaded: true });
    }
  },

  async seen(keys) {
    const rows = get().items.filter(
      (item) => item.part === "finished" && keys.includes(rowKey(item)),
    );
    if (rows.length === 0) return;
    const items = settled(get().items, (item) => rows.includes(item));
    set({ items });
    ring(items);
    try {
      await sayWhereKept(rows);
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
    if (terminalId === null) return;
    const unseen = get().items.filter(
      (item) => item.part === "finished" && inFrontOfSomeone(item, terminalId),
    );
    if (unseen.length === 0) return;
    const items = settled(get().items, (item) => unseen.includes(item));
    set({ items });
    ring(items);
    void ipc.notificationSeenTerminal(terminalId).catch(console.error);
  },

  lookingAtRun(runId) {
    set({ runInFront: runId });
    const ended = get().items.filter(
      (item) => item.part === "finished" && runInFrontOfSomeone(item, runId),
    );
    if (ended.length > 0) void get().seen(ended.map(rowKey));
  },

  lookingThere(hostId, terminalId) {
    const there = hostId !== null && terminalId !== null ? { hostId, terminalId } : null;
    set({ thereInFront: there });
    const finished = get().items.filter(
      (item) => item.part === "finished" && thereInFrontOfSomeone(item, there),
    );
    if (finished.length > 0) void get().seen(finished.map(rowKey));
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
  // This computer's tabs only: a terminal on another computer is not one of them.
  for (const item of items)
    if (item.part === "finished" && item.hostId === null && item.terminalId)
      unseen.add(item.terminalId);
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

/** How many rows of this computer's `projectId` are still in their part of the list: the number
 *  on a project's row in the sidebar. */
export function listedIn(items: NotificationItem[], projectId: string): number {
  return items.filter(
    (item) => item.hostId === null && item.projectId === projectId && item.part !== "earlier",
  ).length;
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
