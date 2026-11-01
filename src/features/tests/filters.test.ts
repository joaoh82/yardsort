import { describe, expect, it } from "vitest";
import type { TestListing, TestRun } from "@/lib/ipc";
import {
  DEFAULT_FILTERS,
  needsSomething,
  readFilters,
  verdictOf,
  visibleRuns,
  visibleTests,
  type Filters,
} from "./filters";

const listing = (
  over: Partial<TestListing> & { id: string; workspaceId: string },
): TestListing => ({
  item: {
    id: over.id,
    name: over.id,
    description: null,
    path: `/w/.yardsort/tests/${over.id}.yaml`,
    text: "",
    problems: [],
    runnable: true,
    usesBrowser: false,
    browserMissing: false,
    inputs: [],
    ...over.item,
  },
  projectId: over.projectId ?? "p1",
  projectName: over.projectName ?? "App",
  workspaceId: over.workspaceId,
  workspaceName: over.workspaceName ?? over.workspaceId,
  envMissing: over.envMissing ?? [],
  lastRun: over.lastRun ?? null,
});

const run = (over: Partial<TestRun> & { id: string }): TestRun =>
  ({
    projectId: "p1",
    workspaceId: "w1",
    workspaceName: "fix",
    testId: "login",
    testName: "Log in",
    definition: "",
    contentHash: "",
    inputs: {},
    status: "passed",
    requestedBy: "app",
    sessionRecordId: null,
    port: null,
    error: null,
    outputDir: null,
    stopping: false,
    createdAt: 1,
    startedAt: null,
    endedAt: null,
    ...over,
  }) as TestRun;

const filters = (over: Partial<Filters> = {}): Filters => ({ ...DEFAULT_FILTERS, ...over });

describe("verdictOf", () => {
  it("maps the last run's status to a verdict, and nothing to never", () => {
    expect(verdictOf(listing({ id: "a", workspaceId: "w1" }))).toBe("never");
    const withRun = (status: string) =>
      verdictOf(
        listing({
          id: "a",
          workspaceId: "w1",
          lastRun: { runId: "r", status, createdAt: 1, endedAt: 2 },
        }),
      );
    expect(withRun("passed")).toBe("passed");
    expect(withRun("awaiting_approval")).toBe("waiting");
    expect(withRun("failed")).toBe("failed");
    expect(withRun("errored")).toBe("failed");
    // A run still going is not a verdict: the test counts as not yet having one.
    expect(withRun("running")).toBe("never");
  });
});

describe("needsSomething", () => {
  it("is true for a missing browser or an unset env name", () => {
    expect(needsSomething(listing({ id: "a", workspaceId: "w1" }))).toBe(false);
    expect(
      needsSomething(
        listing({ id: "a", workspaceId: "w1", item: { browserMissing: true } as never }),
      ),
    ).toBe(true);
    expect(needsSomething(listing({ id: "a", workspaceId: "w1", envMissing: ["PASSWORD"] }))).toBe(
      true,
    );
  });
});

describe("visibleTests", () => {
  const tests = [
    listing({
      id: "login",
      workspaceId: "w1",
      lastRun: { runId: "r1", status: "passed", createdAt: 1, endedAt: 2 },
    }),
    listing({ id: "health", workspaceId: "w2", projectId: "p2", envMissing: ["TOKEN"] }),
  ];
  it("filters by project, workspace, verdict, needs and search", () => {
    expect(visibleTests(tests, filters(), "")).toHaveLength(2);
    expect(visibleTests(tests, filters({ projects: ["p2"] }), "")).toEqual([tests[1]]);
    expect(visibleTests(tests, filters({ workspace: "w1" }), "")).toEqual([tests[0]]);
    expect(visibleTests(tests, filters({ verdict: "passed" }), "")).toEqual([tests[0]]);
    expect(visibleTests(tests, filters({ verdict: "never" }), "")).toEqual([tests[1]]);
    expect(visibleTests(tests, filters({ needs: true }), "")).toEqual([tests[1]]);
    expect(visibleTests(tests, filters(), "health")).toEqual([tests[1]]);
  });
});

describe("visibleRuns", () => {
  const runs = [
    run({ id: "r1", workspaceId: "w1", requestedBy: "app", testName: "Log in" }),
    run({
      id: "r2",
      workspaceId: "w2",
      projectId: "p2",
      requestedBy: "workflow",
      testName: "Health",
    }),
  ];
  it("filters by project, workspace, who asked and search", () => {
    expect(visibleRuns(runs, filters({ tab: "runs" }), "")).toHaveLength(2);
    expect(visibleRuns(runs, filters({ tab: "runs", requestedBy: "workflow" }), "")).toEqual([
      runs[1],
    ]);
    expect(visibleRuns(runs, filters({ tab: "runs", projects: ["p2"] }), "")).toEqual([runs[1]]);
    expect(visibleRuns(runs, filters({ tab: "runs" }), "log in")).toEqual([runs[0]]);
  });
});

describe("readFilters", () => {
  it("drops a project or workspace no longer on show, and keeps the rest", () => {
    const saved = {
      tab: "runs",
      projects: ["p1", "gone"],
      workspace: "missing",
      verdict: "passed",
      needs: true,
      requestedBy: "cli",
    };
    const got = readFilters(saved, ["p1"], ["w1"]);
    expect(got.tab).toBe("runs");
    expect(got.projects).toEqual(["p1"]);
    expect(got.workspace).toBeNull();
    expect(got.verdict).toBe("passed");
    expect(got.needs).toBe(true);
    expect(got.requestedBy).toBe("cli");
  });
  it("falls back to defaults for junk", () => {
    expect(readFilters(null, [], [])).toEqual(DEFAULT_FILTERS);
    expect(readFilters({ verdict: "nonsense", tab: "nope" }, [], [])).toEqual(DEFAULT_FILTERS);
  });
});
