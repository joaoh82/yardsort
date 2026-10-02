import type { Checks, PullRequest } from "@/lib/ipc";

/** One status colour for the row, toolbar, menu trigger and preview. */
export function pullRequestColour(pr: PullRequest): string {
  if (pr.state === "merged") return "bg-violet-500/20 text-violet-300";
  if (
    pr.state === "closed" ||
    pr.checks === "failing" ||
    pr.details?.review === "CHANGES_REQUESTED" ||
    conflicting(pr)
  )
    return "bg-red-500/20 text-red-400";
  if (pr.draft || pr.checks === "running" || pr.details?.review === "REVIEW_REQUIRED")
    return "bg-amber-500/20 text-amber-300";
  // An open PR is green even when CI is not configured. The check label still says
  // "no checks", and a success tick is reserved for checks that actually passed.
  return "bg-green-500/20 text-green-400";
}

/** Open, and GitHub says it cannot be merged without resolving conflicts first. */
export function conflicting(pr: PullRequest): boolean {
  return pr.state === "open" && pr.details?.mergeable === "conflicting";
}

/** Draft, Open, Merged or Closed: the word that goes with [`pullRequestColour`]. */
export function pullRequestStateLabel(pr: PullRequest): string {
  if (pr.state === "merged") return "Merged";
  if (pr.state === "closed") return "Closed";
  return pr.draft ? "Draft" : "Open";
}

/** The colour of a check verdict, wherever one is written out. */
export const checksColour: Record<Checks, string> = {
  none: "text-ink-faint",
  running: "text-amber-300",
  passing: "text-green-400",
  failing: "text-red-400",
};

/** Why this pull request cannot be merged right now, in the words the button's tooltip uses. */
export function mergeBlocked(pr: PullRequest): string | null {
  if (pr.state !== "open") return "It is not open.";
  if (pr.draft) return "It is a draft.";
  if (conflicting(pr)) return `It conflicts with ${pr.details?.base || "its base"}.`;
  if (!pr.details?.headOid) return "Its head commit is not known yet. Refresh.";
  return null;
}
