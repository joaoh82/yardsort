import { describe, expect, it } from "vitest";
import { project, task, tasksOf } from "@/test/fixtures";
import {
  assigneesOf,
  authorsOf,
  DEFAULT_FILTERS,
  filtering,
  labelsOf,
  ME,
  NOBODY,
  readFilters,
  visible,
  type Filters,
} from "./filters";
import { rowsOf } from "./rows";

const alpha = project("alpha");
const beta = project("beta");
const bug = { name: "bug", color: "d73a4a" };
const question = { name: "Question", color: "d876e3" };

const rows = rowsOf([alpha, beta], {
  [alpha.id]: tasksOf([
    task(12, {
      title: "Worktrees on a network drive",
      author: "grace",
      labels: [bug],
      assignees: ["ada"],
      needsAnswer: true,
      updatedAt: "2026-09-30T10:00:00Z",
    }),
    task(9, {
      title: "Document the daemon",
      author: "ada",
      labels: [question],
      updatedAt: "2026-09-29T10:00:00Z",
    }),
    task(4, {
      title: "Crash on an empty repository",
      author: "ken",
      state: "closed",
      closedAs: "completed",
      labels: [bug],
      updatedAt: "2026-09-20T10:00:00Z",
    }),
  ]),
  [beta.id]: tasksOf(
    [
      task(12, {
        title: "Dark mode",
        author: "Grace",
        assignees: ["linus"],
        updatedAt: "2026-09-28T10:00:00Z",
      }),
    ],
    { viewer: "linus" },
  ),
});

const keys = (filters: Partial<Filters>, search = "") =>
  visible(rows, { ...DEFAULT_FILTERS, ...filters }, search).map(
    (row) => `${row.project.name}${row.task.key}`,
  );

describe("visible", () => {
  it("shows open tasks by default, and the others by tab", () => {
    expect(keys({})).toEqual(["alpha#12", "alpha#9", "beta#12"]);
    expect(keys({ state: "closed" })).toEqual(["alpha#4"]);
    expect(keys({ state: "all" })).toHaveLength(4);
  });

  it("narrows by project, label, assignee, author and whether an answer is owed", () => {
    expect(keys({ projects: [beta.id] })).toEqual(["beta#12"]);
    expect(keys({ label: "BUG" }), "whatever its capitals").toEqual(["alpha#12"]);
    expect(keys({ label: "bug", state: "all" })).toEqual(["alpha#12", "alpha#4"]);
    expect(keys({ assignee: "ada" })).toEqual(["alpha#12"]);
    expect(keys({ assignee: NOBODY })).toEqual(["alpha#9"]);
    expect(keys({ author: "grace" })).toEqual(["alpha#12", "beta#12"]);
    expect(keys({ needsAnswer: true })).toEqual(["alpha#12"]);
  });

  it("means each project's own login by Me", () => {
    // `gh` is ada for alpha and linus for beta.
    expect(keys({ assignee: ME })).toEqual(["alpha#12", "beta#12"]);
    expect(keys({ author: ME })).toEqual(["alpha#9"]);
  });

  it("puts filters together, and a search on top of them", () => {
    expect(keys({ author: "grace", label: "bug" })).toEqual(["alpha#12"]);
    expect(keys({ author: "grace", label: "question" })).toEqual([]);
    expect(keys({}, "DAEMON")).toEqual(["alpha#9"]);
    expect(keys({}, "#12")).toEqual(["alpha#12", "beta#12"]);
    expect(keys({}, "1")).toEqual(["alpha#12", "beta#12"]);
    expect(keys({}, "2"), "the start of a number, not its middle").toEqual([]);
    expect(keys({ projects: [alpha.id] }, "12")).toEqual(["alpha#12"]);
    expect(keys({}, "   ")).toHaveLength(3);
  });
});

describe("what the pickers offer", () => {
  it("lists each label, assignee and author once, sorted", () => {
    expect(labelsOf(rows)).toEqual(["bug", "Question"]);
    expect(assigneesOf(rows)).toEqual(["ada", "linus"]);
    expect(authorsOf(rows)).toEqual(["ada", "grace", "ken"]);
  });
});

describe("remembered filters", () => {
  it("knows when something is narrowing the list", () => {
    expect(filtering(DEFAULT_FILTERS, "")).toBe(false);
    expect(filtering({ ...DEFAULT_FILTERS, state: "closed" }, ""), "a tab is not a filter").toBe(
      false,
    );
    expect(filtering({ ...DEFAULT_FILTERS, needsAnswer: true }, "")).toBe(true);
    expect(filtering({ ...DEFAULT_FILTERS, label: "bug" }, "")).toBe(true);
    expect(filtering(DEFAULT_FILTERS, " x ")).toBe(true);
  });

  it("come back as they were saved", () => {
    const saved: Filters = {
      state: "all",
      projects: [alpha.id],
      label: "bug",
      assignee: ME,
      author: "grace",
      needsAnswer: true,
    };
    expect(readFilters(JSON.parse(JSON.stringify(saved)), [alpha.id, beta.id])).toEqual(saved);
  });

  it("fall back to the defaults for anything unreadable, and drop a project that is gone", () => {
    expect(readFilters(null, [])).toEqual(DEFAULT_FILTERS);
    expect(readFilters("open", [])).toEqual(DEFAULT_FILTERS);
    expect(
      readFilters(
        { state: "merged", projects: ["gone", alpha.id, 7], label: "", needsAnswer: "yes" },
        [alpha.id],
      ),
    ).toEqual({ ...DEFAULT_FILTERS, projects: [alpha.id] });
  });
});
