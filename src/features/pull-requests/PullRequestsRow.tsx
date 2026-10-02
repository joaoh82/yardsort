import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { moreOpenThanListed, openIn } from "./rows";

/**
 * The top of the sidebar: one row that opens every project's pull requests in the center panel.
 *
 * A row and not a section, because the list wants the width of the center panel; what the
 * sidebar has room for is the one number worth a glance — how many are open. Nothing is counted
 * without `gh`, as nowhere else in the app shows a pull request without it either.
 */
export function PullRequestsRow() {
  const open = useProjectsStore((s) => s.pullRequestsOpen);
  const projects = useProjectsStore((s) => s.projects);
  // The whole record, counted outside the selector: one that built a number from a filter would
  // be fine, but one that built the list to count would not.
  const byProject = usePublishStore((s) => s.byProject);

  const found = projects.filter((p) => !p.missing).map((p) => byProject[p.id]);
  const count = found.reduce((sum, answer) => sum + openIn(answer), 0);
  const more = found.some(moreOpenThanListed);
  const said = count === 0 ? null : `${count}${more ? "+" : ""}`;

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={said ? `Pull requests, ${said} open` : "Pull requests"}
      title="Every project's pull requests"
      onClick={() => useProjectsStore.getState().openPullRequests(!open)}
      className={`flex h-9 w-full shrink-0 items-center gap-2 border-b border-line px-3 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <PullRequestIcon />
      <span className="min-w-0 flex-1 truncate">Pull requests</span>
      {said && (
        <span
          aria-hidden
          className="shrink-0 rounded-full border border-line px-1.5 text-[10px] leading-4 tabular-nums"
        >
          {said}
        </span>
      )}
    </button>
  );
}

export function PullRequestIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-3.5 shrink-0"
    >
      <circle cx="4" cy="3.5" r="1.6" />
      <circle cx="4" cy="12.5" r="1.6" />
      <circle cx="12" cy="12.5" r="1.6" />
      <path d="M4 5.1v5.8M12 10.9V6.5a2 2 0 0 0-2-2H8.2M9.8 2.9 8.2 4.5l1.6 1.6" />
    </svg>
  );
}
