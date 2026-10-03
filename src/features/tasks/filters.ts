import type { Task } from "@/lib/ipc";
import type { Row } from "./rows";

export type StateTab = "open" | "closed" | "all";

export interface Filters {
  state: StateTab;
  /** Project ids. None chosen means every project. */
  projects: string[];
  /** A label's name, or `null` for any. */
  label: string | null;
  /** A login, [`ME`], [`NOBODY`], or `null` for anyone. */
  assignee: string | null;
  /** A login, [`ME`], or `null` for anyone. */
  author: string | null;
  /** Only the ones waiting on a maintainer. */
  needsAnswer: boolean;
}

/** "Whoever `gh` is logged in as", which differs from forge to forge. */
export const ME = "@me";
/** The assignee filter's "assigned to no one". */
export const NOBODY = "@nobody";

export const DEFAULT_FILTERS: Filters = {
  state: "open",
  projects: [],
  label: null,
  assignee: null,
  author: null,
  needsAnswer: false,
};

/** Where the filters are remembered, in the core's `ui_state`. Search is not: it is of the moment. */
export const FILTERS_KEY = "tasks.filters";

export const STATE_TABS: { id: StateTab; label: string }[] = [
  { id: "open", label: "Open" },
  { id: "closed", label: "Closed" },
  { id: "all", label: "All" },
];

const same = (a: string | null | undefined, b: string | null | undefined) =>
  !!a && !!b && a.toLowerCase() === b.toLowerCase();

/** `login` is who the filter `wanted` means, where `@me` is the row's own viewer. */
const is = (login: string | null, wanted: string, viewer: string | null) =>
  wanted === ME ? same(login, viewer) : same(login, wanted);

function searchMatches(task: Task, search: string): boolean {
  const query = search.trim().toLowerCase();
  if (query === "") return true;
  // `#12` and `12` both mean the number; a number can also be part of a title.
  const digits = /^#?(\d+)$/.exec(query)?.[1];
  if (digits && task.key.replace(/^#/, "").startsWith(digits)) return true;
  return task.title.toLowerCase().includes(query);
}

/** The rows the filters and the search leave, in the order they came. */
export function visible(rows: Row[], filters: Filters, search: string): Row[] {
  return rows.filter(({ task, viewer, project }) => {
    if (filters.state !== "all" && task.state !== filters.state) return false;
    if (filters.projects.length > 0 && !filters.projects.includes(project.id)) return false;
    if (filters.needsAnswer && !task.needsAnswer) return false;
    if (filters.label !== null && !task.labels.some((label) => same(label.name, filters.label))) {
      return false;
    }
    if (filters.assignee === NOBODY) {
      if (task.assignees.length > 0) return false;
    } else if (
      filters.assignee !== null &&
      !task.assignees.some((login) => is(login, filters.assignee!, viewer))
    ) {
      return false;
    }
    if (filters.author !== null && !is(task.author, filters.author, viewer)) return false;
    return searchMatches(task, search);
  });
}

/** Sorted, each once, whatever its capitals. */
function distinct(names: (string | null)[]): string[] {
  const seen = new Map<string, string>();
  for (const name of names) {
    if (name && !seen.has(name.toLowerCase())) seen.set(name.toLowerCase(), name);
  }
  return [...seen.values()].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
}

/** Everyone who opened one of these, for the author filter. */
export const authorsOf = (rows: Row[]): string[] => distinct(rows.map((row) => row.task.author));
/** Everyone one of these is assigned to, for the assignee filter. */
export const assigneesOf = (rows: Row[]): string[] =>
  distinct(rows.flatMap((row) => row.task.assignees));
/** Every label on one of these, for the label filter. */
export const labelsOf = (rows: Row[]): string[] =>
  distinct(rows.flatMap((row) => row.task.labels.map((label) => label.name)));

/** Whether anything narrows the list: what "Clear filters" is offered on. */
export const filtering = (filters: Filters, search: string): boolean =>
  filters.projects.length > 0 ||
  filters.label !== null ||
  filters.assignee !== null ||
  filters.author !== null ||
  filters.needsAnswer ||
  search.trim() !== "";

/**
 * Remembered filters, made safe to use: anything unreadable falls back to its default, and a
 * project that is no longer open is dropped — a filter on something that is not there would hide
 * every row with nothing on screen to say why.
 */
export function readFilters(saved: unknown, projectIds: string[]): Filters {
  if (!saved || typeof saved !== "object") return DEFAULT_FILTERS;
  const raw = saved as Record<string, unknown>;
  const text = (value: unknown) => (typeof value === "string" && value !== "" ? value : null);
  return {
    state: STATE_TABS.find((tab) => tab.id === raw.state)?.id ?? DEFAULT_FILTERS.state,
    projects: Array.isArray(raw.projects)
      ? raw.projects.filter((id): id is string => typeof id === "string" && projectIds.includes(id))
      : [],
    label: text(raw.label),
    assignee: text(raw.assignee),
    author: text(raw.author),
    needsAnswer: raw.needsAnswer === true,
  };
}
