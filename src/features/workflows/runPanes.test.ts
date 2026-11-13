import { describe, expect, it } from "vitest";
import type { WorkflowRun, WorkflowStepRun } from "@/lib/ipc";
import { lasted, openingPick, railRows, SUMMARY } from "./WorkflowRuns";

function step(stepId: string, status: string): WorkflowStepRun {
  return {
    stepId,
    action: "notify",
    status,
    startedAt: null,
    endedAt: null,
    outputs: {},
    note: null,
    outcome: null,
    artifacts: [],
    consumed: [],
    approvalCode: null,
    approvalLocked: false,
    clarificationPrompt: null,
    clarificationQuestions: [],
  };
}

const run = (status: string) => ({ status }) as WorkflowRun;

describe("the step rail", () => {
  const long = [
    ...["a", "b", "c", "d"].map((id) => step(id, "succeeded")),
    step("verify", "failed"),
    ...["e", "f"].map((id) => step(id, "skipped")),
  ];

  it("folds a long run's succeeded steps into one row", () => {
    const rows = railRows(long, "verify", false);
    expect(rows[0]).toMatchObject({ kind: "folded" });
    expect(rows[0]!.kind === "folded" && rows[0]!.steps.map((s) => s.stepId)).toEqual([
      "a",
      "b",
      "c",
      "d",
    ]);
    expect(rows.slice(1).map((r) => r.kind === "step" && r.step.stepId)).toEqual([
      "verify",
      "e",
      "f",
    ]);
  });

  it("never folds away the step picked, nor fewer than three in a row", () => {
    const rows = railRows(long, "c", false);
    // a, b on one side of the pick and d on the other are too few to fold.
    expect(rows.every((r) => r.kind === "step")).toBe(true);
  });

  it("lists every step of a short run, or one unfolded", () => {
    expect(railRows(long.slice(0, 6), SUMMARY, false).every((r) => r.kind === "step")).toBe(true);
    expect(railRows(long, SUMMARY, true)).toHaveLength(long.length);
  });
});

describe("what a run opens on", () => {
  it("is the step waiting on the person, then the failed step, then the summary", () => {
    expect(openingPick(run("running"), [step("a", "succeeded"), step("gate", "waiting")])).toBe(
      "gate",
    );
    expect(openingPick(run("failed"), [step("a", "succeeded"), step("verify", "failed")])).toBe(
      "verify",
    );
    expect(openingPick(run("succeeded"), [step("a", "succeeded")])).toBe(SUMMARY);
    expect(openingPick(run("failed"), undefined)).toBe(SUMMARY);
  });
});

describe("how long something took", () => {
  it("reads in seconds, minutes, then hours and minutes", () => {
    expect(lasted(45_000)).toBe("45 s");
    expect(lasted(12 * 60_000)).toBe("12 min");
    expect(lasted(89 * 60_000)).toBe("1 h 29 min");
    expect(lasted(120 * 60_000)).toBe("2 h");
  });
});
