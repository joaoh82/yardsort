import type { PullRequest } from "@/lib/ipc";
import type { Row } from "./rows";

export type StateTab = "all" | "open" | "merged" | "closed";
export type ReviewFilter =
  "any" | "none" | "required" | "approved" | "changes" | "byYou" | "notByYou" | "awaitingYou";

export interface Filters {
  state: StateTab;
  /** Project ids. None chosen means every project. */
  projects: string[];
  /** A login, [`ME`], or `null` for anyone. */
  author: string | null;
  reviews: ReviewFilter;
}

/** The author filter's "whoever `gh` is logged in as", which differs from forge to forge. */
export const ME = "@me";

export const DEFAULT_FILTERS: Filters = {
  state: "open",
  projects: [],
  author: null,
  reviews: "any",
};

/** Where the filters are remembered, in the core's `ui_state`. Search is not: it is of the moment. */
export const FILTERS_KEY = "pullRequests.filters";

export const STATE_TABS: { id: StateTab; label: string }[] = [
  { id: "all", label: "All" },
  { id: "open", label: "Open" },
  { id: "merged", label: "Merged" },
  { id: "closed", label: "Closed" },
];

/** `you` marks the ones that need to know who you are. */
export const REVIEW_FILTERS: { id: ReviewFilter; label: string; you?: true }[] = [
  { id: "any", label: "Any review status" },
  { id: "none", label: "No reviews" },
  { id: "required", label: "Review required" },
  { id: "approved", label: "Approved" },
  { id: "changes", label: "Changes requested" },
  { id: "byYou", label: "Reviewed by you", you: true },
  { id: "notByYou", label: "Not reviewed by you", you: true },
  { id: "awaitingYou", label: "Awaiting review from you", you: true },
];

const same = (a: string | null | undefined, b: string | null | undefined) =>
  !!a && !!b && a.toLowerCase() === b.toLowerCase();

/**
 * Where a pull request stands with its reviewers.
 *
 * The forge's own decision when it has one. A repository with no review rule has none, and then
 * the latest reviews decide: any request for changes outranks any approval.
 */
export function reviewDecision(pr: PullRequest): "approved" | "changes" | "required" | null {
  const details = pr.details;
  if (!details) return null;
  if (details.review === "APPROVED") return "approved";
  if (details.review === "CHANGES_REQUESTED") return "changes";
  if (details.review === "REVIEW_REQUIRED") return "required";
  if (details.reviews.some((review) => review.state === "changesRequested")) return "changes";
  if (details.reviews.some((review) => review.state === "approved")) return "approved";
  return null;
}

function reviewMatches(row: Row, filter: ReviewFilter): boolean {
  if (filter === "any") return true;
  const details = row.pr.details;
  if (!details) return false;
  const you = row.viewer;
  const reviewedByYou = details.reviews.some((review) => same(review.login, you));
  switch (filter) {
    case "none":
      return details.reviews.length === 0;
    case "required":
      return reviewDecision(row.pr) === "required";
    case "approved":
      return reviewDecision(row.pr) === "approved";
    case "changes":
      return reviewDecision(row.pr) === "changes";
    case "byYou":
      return reviewedByYou;
    case "notByYou":
      // Your own pull request is not one you have failed to review.
      return !!you && !reviewedByYou && !same(row.pr.author, you);
    case "awaitingYou":
      return details.reviewRequests.some((request) => !request.team && same(request.name, you));
  }
}

function searchMatches(pr: PullRequest, search: string): boolean {
  const query = search.trim().toLowerCase();
  if (query === "") return true;
  // `#12` and `12` both mean the number; a number can also be part of a title.
  const digits = /^#?(\d+)$/.exec(query)?.[1];
  if (digits && String(pr.number).startsWith(digits)) return true;
  return pr.title.toLowerCase().includes(query) || pr.branch.toLowerCase().includes(query);
}

/** The rows the filters and the search leave, in the order they came. */
export function visible(rows: Row[], filters: Filters, search: string): Row[] {
  return rows.filter((row) => {
    if (filters.state !== "all" && row.pr.state !== filters.state) return false;
    if (filters.projects.length > 0 && !filters.projects.includes(row.project.id)) return false;
    if (filters.author === ME) {
      if (!same(row.pr.author, row.viewer)) return false;
    } else if (filters.author !== null && !same(row.pr.author, filters.author)) return false;
    return reviewMatches(row, filters.reviews) && searchMatches(row.pr, search);
  });
}

/** Everyone who opened one of these, for the author filter: sorted, each once. */
export function authorsOf(rows: Row[]): string[] {
  const seen = new Map<string, string>();
  for (const row of rows) {
    const author = row.pr.author;
    if (author && !seen.has(author.toLowerCase())) seen.set(author.toLowerCase(), author);
  }
  return [...seen.values()].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
}

/** Whether anything narrows the list: what "Clear filters" is offered on. */
export const filtering = (filters: Filters, search: string): boolean =>
  filters.projects.length > 0 ||
  filters.author !== null ||
  filters.reviews !== "any" ||
  search.trim() !== "";

/**
 * Remembered filters, made safe to use: anything unreadable falls back to its default, and a
 * project that is no longer open is dropped — a filter on something that is not there would hide
 * every row with nothing on screen to say why.
 */
export function readFilters(saved: unknown, projectIds: string[]): Filters {
  if (!saved || typeof saved !== "object") return DEFAULT_FILTERS;
  const raw = saved as Record<string, unknown>;
  const state = STATE_TABS.find((tab) => tab.id === raw.state)?.id ?? DEFAULT_FILTERS.state;
  const reviews =
    REVIEW_FILTERS.find((filter) => filter.id === raw.reviews)?.id ?? DEFAULT_FILTERS.reviews;
  const projects = Array.isArray(raw.projects)
    ? raw.projects.filter((id): id is string => typeof id === "string" && projectIds.includes(id))
    : [];
  const author = typeof raw.author === "string" && raw.author !== "" ? raw.author : null;
  return { state, projects, author, reviews };
}
