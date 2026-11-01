import type { Activity } from "@/features/terminal/activity";
import type { TestRun, TestStepRun } from "@/lib/ipc";

/** The same colours for a run's or a step's state everywhere it shows. */
export function statusColour(status: string): string {
  switch (status) {
    case "passed":
      return "bg-green-500/20 text-green-400";
    case "failed":
    case "errored":
      return "bg-red-500/20 text-red-400";
    case "running":
    case "waiting":
    case "queued":
    case "awaiting_approval":
      return "bg-amber-500/20 text-amber-300";
    default:
      return "bg-raised text-ink-faint";
  }
}

/** A status as a word or two: `awaiting approval`, `passed`. */
export function statusWords(run: Pick<TestRun, "status" | "stopping">): string {
  if (run.status === "running" && run.stopping) return "stopping";
  return run.status.replace(/_/g, " ");
}

/** What a run's state is for the dot on its tab. */
export function runActivity(run: Pick<TestRun, "status">): Activity {
  switch (run.status) {
    case "running":
      return "busy";
    case "queued":
    case "awaiting_approval":
      return "waiting";
    case "failed":
    case "errored":
      return "failed";
    default:
      return "idle";
  }
}

/** Who asked for the run, in words. */
export function requestedByWords(run: Pick<TestRun, "requestedBy">): string {
  switch (run.requestedBy) {
    case "cli":
      return "an agent, with ys test run";
    case "workflow":
      return "a workflow";
    default:
      return "you";
  }
}

/** `1m 12s`, `31s`, `2h 5m`, as `ys test wait` prints it. */
export function durationWords(ms: number): string {
  const secs = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

/** How long a run or a step took, or has taken so far. */
export function tookWords(
  item: { startedAt: number | null; endedAt: number | null },
  now = Date.now(),
): string | null {
  if (item.startedAt === null) return null;
  if (item.endedAt === null) return `${durationWords(now - item.startedAt)} so far`;
  return durationWords(item.endedAt - item.startedAt);
}

export const PHASES = ["setup", "steps", "teardown"] as const;

export const PHASE_WORDS: Record<(typeof PHASES)[number], string> = {
  setup: "Setup",
  steps: "Steps",
  teardown: "Teardown",
};

/** A step's status as the list shows it. */
export function stepStatusWords(step: Pick<TestStepRun, "status">): string {
  return step.status === "waiting" ? "waiting for you" : step.status;
}

/** The outputs a step shows as facts, besides its streams: `exit 0`, `status 200`, `waited 3s`. */
export function outputFacts(step: TestStepRun): [string, string][] {
  const hidden = new Set(["stdout", "stderr", "body", "headers", "prompt", "process"]);
  return Object.entries(step.outputs).filter(([name]) => !hidden.has(name));
}
