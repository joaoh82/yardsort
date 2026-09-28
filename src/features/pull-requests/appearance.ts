import type { PullRequest } from "@/lib/ipc";

/** One status colour for the row, toolbar, menu trigger and preview. */
export function pullRequestColour(pr: PullRequest): string {
  if (pr.state === "merged") return "bg-violet-500/20 text-violet-300";
  if (
    pr.state === "closed" ||
    pr.checks === "failing" ||
    pr.details?.review === "CHANGES_REQUESTED"
  )
    return "bg-red-500/20 text-red-400";
  if (pr.draft || pr.checks === "running" || pr.details?.review === "REVIEW_REQUIRED")
    return "bg-amber-500/20 text-amber-300";
  // An open PR is green even when CI is not configured. The check label still says
  // "no checks", and a success tick is reserved for checks that actually passed.
  return "bg-green-500/20 text-green-400";
}
