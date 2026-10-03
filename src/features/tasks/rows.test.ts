import { describe, expect, it } from "vitest";
import { project, task, tasksOf } from "@/test/fixtures";
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
