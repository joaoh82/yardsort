import { describe, expect, it } from "vitest";
import { project, task, tasksOf, worktree } from "@/test/fixtures";
import { at, moreOpenThanListed, needingAnswer, openIn, rowsOf, stateLabel } from "./rows";

const alpha = project("alpha");
const beta = project("beta");

describe("rowsOf", () => {
  it("is every project's tasks as one list, most recently changed first", () => {
    const rows = rowsOf([alpha, beta], {
      [alpha.id]: tasksOf([
        task(1, { updatedAt: "2026-09-01T00:00:00Z" }),
        task(3, { updatedAt: "2026-09-03T00:00:00Z" }),
      ]),
      [beta.id]: tasksOf([task(1, { updatedAt: "2026-09-02T00:00:00Z" })], { viewer: "linus" }),
    });
    expect(rows.map((row) => `${row.project.name}${row.task.key}`)).toEqual([
      "alpha#3",
      "beta#1",
      "alpha#1",
    ]);
    expect(new Set(rows.map((row) => row.key)).size, "two #1s, two keys").toBe(3);
    expect(rows[1]!.viewer).toBe("linus");
  });

  it("leaves out a project whose folder is gone, and one nothing is known about yet", () => {
    const gone = { ...beta, missing: true };
    const rows = rowsOf([alpha, gone, project("gamma")], {
      [alpha.id]: tasksOf([task(1)]),
      [gone.id]: tasksOf([task(2)]),
    });
    expect(rows.map((row) => row.task.key)).toEqual(["#1"]);
  });

  it("orders by number when the times are the same or unreadable", () => {
    const rows = rowsOf([alpha], {
      [alpha.id]: tasksOf([task(2, { updatedAt: "" }), task(5, { updatedAt: "" })]),
    });
    expect(rows.map((row) => row.task.key)).toEqual(["#5", "#2"]);
    expect(at("")).toBeNull();
    expect(at("2026-09-01T00:00:00Z")).toBe(Date.parse("2026-09-01T00:00:00Z"));
  });
});

describe("the workspaces started from a task", () => {
  const from = (number: number) => ({
    source: "github" as const,
    repo: "github.com/demo/app",
    key: `#${number}`,
    url: `https://github.com/demo/app/issues/${number}`,
    title: `Task ${number}`,
  });

  it("are on its row, by the task's link and not by its key alone", () => {
    const first = { ...worktree("alpha", "one"), tasks: [from(7)] };
    const second = { ...worktree("alpha", "two"), tasks: [from(7)] };
    const other = { ...worktree("alpha", "three"), tasks: [from(8)] };
    const put_away = { ...worktree("alpha", "four"), tasks: [from(7)], archived: true };
    const gone = { ...worktree("alpha", "five"), tasks: [from(7)], missing: true };
    // Another project's #7 is another task.
    const elsewhere = {
      ...worktree("beta", "six"),
      tasks: [{ ...from(7), url: "https://github.com/demo/site/issues/7" }],
    };
    const rows = rowsOf(
      [
        { ...alpha, workspaces: [...alpha.workspaces, first, second, other, put_away, gone] },
        { ...beta, workspaces: [...beta.workspaces, elsewhere] },
      ],
      {
        [alpha.id]: tasksOf([task(7), task(9)]),
        [beta.id]: tasksOf([task(7)]),
      },
    );
    const of = (name: string, key: string) =>
      rows
        .find((row) => row.project.name === name && row.task.key === key)!
        .workspaces.map((workspace) => workspace.name);
    expect(of("alpha", "#7"), "a folder that is gone is not somewhere to go").toEqual([
      "one",
      "two",
    ]);
    expect(of("alpha", "#9")).toEqual([]);
    expect(of("beta", "#7"), "its own repository's issue is a different link").toEqual([]);
  });
});

describe("counts", () => {
  const found = tasksOf(
    [task(1, { needsAnswer: true }), task(2), task(3, { state: "closed", closedAs: "completed" })],
    { openTotal: 2 },
  );

  it("counts the open ones, and the ones waiting on a maintainer", () => {
    expect(openIn(found)).toBe(2);
    expect(needingAnswer(found)).toBe(1);
    expect(openIn(undefined)).toBe(0);
    expect(needingAnswer(undefined)).toBe(0);
  });

  it("knows when the source has more open than the list holds", () => {
    expect(moreOpenThanListed(found)).toBe(false);
    expect(moreOpenThanListed({ ...found, openTotal: 1039 })).toBe(true);
    expect(moreOpenThanListed({ ...found, openTotal: null })).toBe(false);
    expect(moreOpenThanListed(undefined)).toBe(false);
  });
});

describe("stateLabel", () => {
  it("says why a closed task was closed when that is not that it was done", () => {
    expect(stateLabel(task(1))).toBe("Open");
    expect(stateLabel(task(1, { state: "closed", closedAs: "completed" }))).toBe("Closed");
    expect(stateLabel(task(1, { state: "closed", closedAs: null }))).toBe("Closed");
    expect(stateLabel(task(1, { state: "closed", closedAs: "notPlanned" }))).toBe("Not planned");
    expect(stateLabel(task(1, { state: "closed", closedAs: "duplicate" }))).toBe("Duplicate");
  });
});
