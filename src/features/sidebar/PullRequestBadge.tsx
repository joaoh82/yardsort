import type { PullRequest } from "@/lib/ipc";

/**
 * A workspace's pull request on its row: the number, and what CI made of it.
 *
 * The number is the point — it is what you quote to someone else, and it means the branch has
 * left this machine. The checks colour it rather than adding a second mark, because a sidebar
 * row has room for one glance, not two. Nothing shows without `gh`: see `crate::forge`.
 */
export function PullRequestBadge({ pr }: { pr: PullRequest }) {
  const { text, colour } =
    pr.state === "merged"
      ? { text: "merged", colour: "bg-violet-500/20 text-violet-300" }
      : pr.state === "closed"
        ? { text: "closed", colour: "text-ink-faint line-through" }
        : pr.checks === "failing"
          ? { text: `#${pr.number}`, colour: "bg-red-500/20 text-red-400" }
          : pr.checks === "running"
            ? { text: `#${pr.number}`, colour: "bg-amber-500/20 text-amber-300" }
            : pr.checks === "passing"
              ? { text: `#${pr.number}`, colour: "bg-green-500/20 text-green-400" }
              : { text: `#${pr.number}`, colour: "bg-raised text-ink-faint" };

  const label = [
    pr.draft ? `Draft pull request #${pr.number}` : `Pull request #${pr.number}`,
    pr.state === "merged" ? "merged" : pr.state === "closed" ? "closed" : null,
    pr.state === "open" ? CHECKS[pr.checks] : null,
    pr.title,
  ]
    .filter(Boolean)
    .join(" — ");

  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`shrink-0 rounded px-1 text-[10px] leading-4 font-medium tabular-nums ${
        pr.draft && pr.state === "open" ? "opacity-60" : ""
      } ${colour}`}
    >
      {text}
    </span>
  );
}

const CHECKS: Record<PullRequest["checks"], string> = {
  none: "no checks",
  running: "checks running",
  passing: "checks passing",
  failing: "checks failing",
};
