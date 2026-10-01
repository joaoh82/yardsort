import { openUrl } from "@tauri-apps/plugin-opener";
import type { PullRequest } from "@/lib/ipc";
import { conflicting, pullRequestColour } from "@/features/pull-requests/appearance";

/**
 * A workspace's pull request on its row: the number, and what CI made of it. Pressing it opens
 * the pull request in the browser.
 *
 * The number is the point — it is what you quote to someone else, and it means the branch has
 * left this machine. Its status colours it rather than adding a second mark, because a sidebar
 * row has room for one glance, not two. Nothing shows without `gh`: see `crate::forge`.
 *
 * It sits *beside* the row's own button rather than inside it, because a button inside a button
 * is not a thing: the row opens the workspace, and this opens the pull request.
 */
export function PullRequestBadge({ pr, toolbar = false }: { pr: PullRequest; toolbar?: boolean }) {
  const text =
    pr.state === "merged" ? "merged" : pr.state === "closed" ? "closed" : `#${pr.number}`;
  const colour = pullRequestColour(pr);

  const label = [
    pr.draft ? `Draft pull request #${pr.number}` : `Pull request #${pr.number}`,
    pr.state === "merged" ? "merged" : pr.state === "closed" ? "closed" : null,
    pr.state === "open" ? CHECKS[pr.checks] : null,
    conflicting(pr) ? "merge conflicts" : null,
    pr.title,
  ]
    .filter(Boolean)
    .join(" — ");

  return (
    <button
      type="button"
      aria-label={`Open ${label}`}
      onClick={() => void openUrl(pr.url).catch(console.error)}
      className={`shrink-0 rounded ${toolbar ? "px-2 py-1 text-xs" : "px-1 text-[10px]"} leading-4 font-medium tabular-nums hover:brightness-125 ${pr.state === "closed" ? "line-through" : ""} ${colour}`}
    >
      {toolbar && (
        <svg
          aria-hidden
          viewBox="0 0 16 16"
          className="mr-1 inline size-3.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
        >
          <circle cx="4" cy="3" r="2" />
          <circle cx="12" cy="13" r="2" />
          <path d="M4 5v9M9 2h1a2 2 0 0 1 2 2v7M8 4l2-2-2-2" />
        </svg>
      )}
      {toolbar ? `#${pr.number}` : text}
      {toolbar && conflicting(pr) && (
        <span aria-hidden className="ml-1" title="Merge conflicts">
          ⚠
        </span>
      )}
      {toolbar && (
        <span aria-hidden className="ml-1">
          {pr.checks === "passing"
            ? "✓"
            : pr.checks === "failing"
              ? "✗"
              : pr.checks === "running"
                ? "◷"
                : ""}
        </span>
      )}
    </button>
  );
}

const CHECKS: Record<PullRequest["checks"], string> = {
  none: "no checks",
  running: "checks running",
  passing: "checks passing",
  failing: "checks failing",
};
