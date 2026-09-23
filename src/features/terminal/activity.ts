import type { ExitInfo } from "@/lib/ipc";

export type Activity = "busy" | "waiting" | "idle" | "failed";

/** Roll several terminals up into one dot: busy beats waiting beats idle. */
export function summarise(tabs: { busy: boolean; exit: unknown }[]): Activity {
  const live = tabs.filter((tab) => !tab.exit);
  if (live.some((tab) => tab.busy)) return "busy";
  return live.length > 0 ? "waiting" : "idle";
}

/**
 * What a workspace's *harnesses* have got to, for the badge on its row in the sidebar. Shells are
 * left out: a prompt sitting there is not news, and a workspace whose agent has ended should not
 * read as busy because a shell is still open next to it.
 */
export type HarnessState =
  | { kind: "none" }
  | { kind: "working" }
  /** `count` of them are running but quiet — your turn. */
  | { kind: "waiting"; count: number }
  | { kind: "done" }
  | { kind: "failed" };

/**
 * Waiting outranks working: if one of two agents wants you, that is the thing to say. Among the
 * ones that have ended, a failure outranks a clean finish for the same reason.
 */
export function harnessState(
  tabs: { busy: boolean; exit: ExitInfo | null; recordId: string | null }[],
): HarnessState {
  const harnesses = tabs.filter((tab) => tab.recordId !== null);
  const live = harnesses.filter((tab) => !tab.exit);
  const waiting = live.filter((tab) => !tab.busy).length;
  if (waiting > 0) return { kind: "waiting", count: waiting };
  if (live.length > 0) return { kind: "working" };
  if (harnesses.some((tab) => tab.exit && !tab.exit.success)) return { kind: "failed" };
  return harnesses.length > 0 ? { kind: "done" } : { kind: "none" };
}
