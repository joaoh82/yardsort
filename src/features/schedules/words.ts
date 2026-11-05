import type { Schedule, ScheduleRun } from "@/lib/ipc";

/** Where a schedule stands, in a word or two, for its badge. */
export function standingWords(standing: Schedule["standing"]): string {
  switch (standing) {
    case "waiting":
      return "on";
    case "running":
      return "running";
    case "awaiting_approval":
      return "waiting for approval";
    case "refused":
      return "refused";
    case "no_workspace":
      return "no workspace";
    case "off":
      return "off";
    case "done":
      return "done";
  }
}

/** The badge's colours: amber for what needs the user, red for what was refused. */
export function standingColour(standing: Schedule["standing"]): string {
  switch (standing) {
    case "waiting":
      return "bg-green-500/20 text-green-400";
    case "running":
    case "awaiting_approval":
      return "bg-amber-500/20 text-amber-300";
    case "refused":
    case "no_workspace":
      return "bg-red-500/20 text-red-400";
    default:
      return "bg-raised text-ink-faint";
  }
}

/** What became of a time, in a word or two: the run's ending once it has one. */
export function lineWords(run: Pick<ScheduleRun, "outcome" | "result" | "awaitingApproval">) {
  if (run.outcome !== "started") return run.outcome;
  if (run.result) return run.result.replace(/_/g, " ");
  return run.awaitingApproval ? "waiting for approval" : "running";
}

/** A line's colours, by what it came to. */
export function lineColour(run: Pick<ScheduleRun, "outcome" | "result" | "awaitingApproval">) {
  const words = lineWords(run);
  switch (words) {
    case "passed":
    case "succeeded":
      return "bg-green-500/20 text-green-400";
    case "failed":
    case "errored":
    case "refused":
      return "bg-red-500/20 text-red-400";
    case "running":
    case "waiting for approval":
      return "bg-amber-500/20 text-amber-300";
    default:
      return "bg-raised text-ink-faint";
  }
}

/** A moment on this machine's clock: `Fri 6 Nov, 09:00`. Twenty-four hours, as a schedule's
 *  own words are ("Daily at 09:00"), so the two never disagree on one row. */
export function clockTime(at: number): string {
  return new Date(at).toLocaleString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  });
}

/** What a schedule runs, in words: `workflow ping`, `smoke test login`. */
export function targetWords(schedule: Pick<Schedule, "target" | "targetId">): string {
  return `${schedule.target === "test" ? "smoke test" : "workflow"} ${schedule.targetId}`;
}
