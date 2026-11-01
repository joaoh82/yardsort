import type { TestListing, TestRun } from "@/lib/ipc";

/** The two tabs of the Tests view: the tests themselves, or every run of them. */
export type Tab = "tests" | "runs";

/** The verdict column's values, for the Tests tab's filter. */
export type Verdict = "passed" | "failed" | "waiting" | "never";

export interface Filters {
  tab: Tab;
  /** Project ids. None chosen means every project. */
  projects: string[];
  /** A workspace id, or `null` for any. */
  workspace: string | null;
  /** The last run's verdict (Tests tab), or `null` for any. */
  verdict: Verdict | null;
  /** Only tests that need something the machine lacks: a browser, or an unset environment name. */
  needs: boolean;
  /** Who asked for the run (Runs tab): `app` (you), `cli` (an agent), `workflow`, or `null`. */
  requestedBy: string | null;
}

export const DEFAULT_FILTERS: Filters = {
  tab: "tests",
  projects: [],
  workspace: null,
  verdict: null,
  needs: false,
  requestedBy: null,
};

/** Where the filters are remembered, in the core's `ui_state`. Search is of the moment, not kept. */
export const FILTERS_KEY = "tests.filters";

export const TABS: { id: Tab; label: string }[] = [
  { id: "tests", label: "Tests" },
  { id: "runs", label: "Runs" },
];

export const VERDICTS: { id: Verdict; label: string }[] = [
  { id: "passed", label: "Passed" },
  { id: "failed", label: "Failed" },
  { id: "waiting", label: "Waiting for approval" },
  { id: "never", label: "Never run" },
];

export const REQUESTED_BY: { id: string; label: string }[] = [
  { id: "app", label: "You" },
  { id: "cli", label: "An agent" },
  { id: "workflow", label: "A workflow" },
];

/** The verdict of a test's last run, as the filter and the column name it. */
export function verdictOf(listing: TestListing): Verdict {
  const status = listing.lastRun?.status;
  if (!status) return "never";
  if (status === "passed") return "passed";
  if (status === "awaiting_approval") return "waiting";
  if (status === "queued" || status === "running") return "never";
  // failed, refused, cancelled, errored all read as "failed" for the filter's purpose: it did
  // not pass. The column shows the exact word.
  return "failed";
}

/** Whether a test needs something the machine lacks: a browser, or an unset environment name. */
export function needsSomething(listing: TestListing): boolean {
  return listing.item.browserMissing || listing.envMissing.length > 0;
}

function matchesSearch(text: string, search: string): boolean {
  const query = search.trim().toLowerCase();
  return query === "" || text.toLowerCase().includes(query);
}

/** The test listings the filters and search leave. */
export function visibleTests(
  listings: TestListing[],
  filters: Filters,
  search: string,
): TestListing[] {
  return listings.filter((listing) => {
    if (filters.projects.length > 0 && !filters.projects.includes(listing.projectId)) return false;
    if (filters.workspace !== null && listing.workspaceId !== filters.workspace) return false;
    if (filters.verdict !== null && verdictOf(listing) !== filters.verdict) return false;
    if (filters.needs && !needsSomething(listing)) return false;
    return matchesSearch(
      `${listing.item.name} ${listing.item.id} ${listing.workspaceName}`,
      search,
    );
  });
}

/** The runs the filters and search leave. */
export function visibleRuns(runs: TestRun[], filters: Filters, search: string): TestRun[] {
  return runs.filter((run) => {
    if (filters.projects.length > 0 && !(run.projectId && filters.projects.includes(run.projectId)))
      return false;
    if (filters.workspace !== null && run.workspaceId !== filters.workspace) return false;
    if (filters.requestedBy !== null && run.requestedBy !== filters.requestedBy) return false;
    return matchesSearch(`${run.testName} ${run.testId} ${run.workspaceName}`, search);
  });
}

/** Whether anything narrows the list: what "Clear filters" is offered on. The tab is where you
 *  are, not a filter. */
export function filtering(filters: Filters, search: string): boolean {
  return (
    filters.projects.length > 0 ||
    filters.workspace !== null ||
    filters.verdict !== null ||
    filters.needs ||
    filters.requestedBy !== null ||
    search.trim() !== ""
  );
}

/**
 * Remembered filters, made safe: anything unreadable falls back to its default, and a project
 * or workspace no longer on show is dropped — a filter on something gone would hide every row
 * with nothing on screen to say why.
 */
export function readFilters(saved: unknown, projectIds: string[], workspaceIds: string[]): Filters {
  if (!saved || typeof saved !== "object") return DEFAULT_FILTERS;
  const raw = saved as Record<string, unknown>;
  const workspace =
    typeof raw.workspace === "string" && workspaceIds.includes(raw.workspace)
      ? raw.workspace
      : null;
  const verdict = VERDICTS.find((v) => v.id === raw.verdict)?.id ?? null;
  const requestedBy = REQUESTED_BY.find((r) => r.id === raw.requestedBy)?.id ?? null;
  return {
    tab: TABS.find((tab) => tab.id === raw.tab)?.id ?? DEFAULT_FILTERS.tab,
    projects: Array.isArray(raw.projects)
      ? raw.projects.filter((id): id is string => typeof id === "string" && projectIds.includes(id))
      : [],
    workspace,
    verdict,
    needs: raw.needs === true,
    requestedBy,
  };
}
