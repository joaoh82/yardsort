import { openUrl } from "@tauri-apps/plugin-opener";
import type { PullRequest } from "@/lib/ipc";
import { checksColour, conflicting, pullRequestColour, pullRequestStateLabel } from "./appearance";
import { checksSummary } from "./rows";

const checksLabel = {
  none: "No checks reported",
  running: "Checks running",
  passing: "All checks passed",
  failing: "Checks failing",
};
const reviewLabel: Record<string, string> = {
  APPROVED: "Approved",
  CHANGES_REQUESTED: "Changes requested",
  REVIEW_REQUIRED: "Review required",
};

/**
 * A pull request at a glance. `link` is the "View on GitHub" foot, which the Pull requests view
 * leaves off: it has that among its own actions.
 */
export function PullRequestDetails({ pr, link = true }: { pr: PullRequest; link?: boolean }) {
  const details = pr.details;
  const updated = details?.updatedAt ? new Date(details.updatedAt) : null;
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-ink-muted">#{pr.number}</span>
        <span className={`rounded-full px-2 py-0.5 ${pullRequestColour(pr)}`}>
          {pullRequestStateLabel(pr)}
        </span>
        {details?.review && (
          <span className="text-ink-muted">{reviewLabel[details.review] ?? details.review}</span>
        )}
        {details && (
          <span className="ml-auto font-mono">
            <span className="text-green-400">+{details.additions}</span>{" "}
            <span className="text-red-400">−{details.deletions}</span>
          </span>
        )}
      </div>
      <p className="text-sm font-medium break-words">{pr.title}</p>
      <p className="font-mono text-[11px] break-all text-ink-muted">
        {pr.branch}
        {details?.base ? ` → ${details.base}` : ""}
      </p>
      {conflicting(pr) && (
        <p className="text-red-400">
          Conflicts with {details?.base || "its base"}: they must be resolved before it can merge.
        </p>
      )}
      <div className="border-t border-line pt-3">
        <p className={checksColour[pr.checks]}>
          {checksLabel[pr.checks]}
          {/* Counted by the core, which has the numbers even when it has no names to list. */}
          {checksSummary(pr) && ` · ${checksSummary(pr)}`}
        </p>
        {!!details?.checks.length && (
          <details className="mt-2">
            <summary className="cursor-pointer text-ink-muted">Show checks</summary>
            <ul className="mt-2 space-y-1.5">
              {details.checks.map((check, index) => (
                <li key={`${check.name}-${index}`} className="flex justify-between gap-3">
                  <span className="min-w-0 break-words">{check.name}</span>
                  <span className={`shrink-0 ${checksColour[check.state]}`}>{check.state}</span>
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
      {updated && !Number.isNaN(updated.getTime()) && (
        <p className="text-[11px] text-ink-faint">Updated {updated.toLocaleString()}</p>
      )}
      {link && (
        <button
          type="button"
          onClick={() => void openUrl(pr.url).catch(console.error)}
          className="flex w-full items-center justify-between border-t border-line pt-3 text-ink-muted hover:text-ink"
        >
          View on GitHub <span aria-hidden>↗</span>
        </button>
      )}
    </div>
  );
}
