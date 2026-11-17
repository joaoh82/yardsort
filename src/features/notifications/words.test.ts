import { describe, expect, it } from "vitest";
import { notification, standingNotification } from "@/test/fixtures";
import {
  about,
  chip,
  detailName,
  leadName,
  opens,
  sections,
  wentBadly,
  what,
  workedWords,
} from "./words";

const asks = notification("asks", { command: "git push --force-with-lease", at: 400 });
const finished = notification("finished", {
  kind: "agent.finished",
  part: "finished",
  workedMs: 12 * 60_000 + 20_000,
  at: 300,
});
const stopped = notification("stopped", {
  kind: "agent.stopped",
  part: "finished",
  harnessId: "codex",
  exitCode: 1,
  projectId: "p-site",
  projectName: "site",
  workspaceName: "cache-forecasts",
  at: 500,
});
const answered = notification("answered", {
  part: "earlier",
  settledAt: 200,
  command: null,
  at: 100,
});
const seen = notification("seen", {
  kind: "agent.failed",
  part: "earlier",
  settledAt: 250,
  at: 90,
});
const all = [asks, finished, stopped, answered, seen];

describe("a notification's words", () => {
  it("says what happened, and who", () => {
    expect(what(asks)).toBe("claude asks for a permission");
    expect(what(answered)).toBe("claude asked for a permission");
    expect(what(finished)).toBe("claude finished after 12 min");
    expect(what({ ...finished, workedMs: 20_000 })).toBe("claude finished");
    expect(what({ ...finished, workedMs: null })).toBe("claude finished");
    expect(what(stopped)).toBe("codex stopped with an error");
    expect(what(seen)).toBe("claude's turn failed");
    expect(what({ ...finished, workedMs: null, harnessId: null })).toBe("The agent finished");
  });

  it("says how long to the minute", () => {
    expect(workedWords(59_000)).toBeNull();
    expect(workedWords(60_000)).toBe("1 min");
    expect(workedWords(60 * 60_000)).toBe("1 h");
    expect(workedWords(65 * 60_000)).toBe("1 h 5 min");
  });

  it("shows the command while the question is open, and the exit code of one that stopped", () => {
    expect(chip(asks)).toBe("git push --force-with-lease");
    expect(chip({ ...answered, command: "npm test" })).toBeNull();
    expect(chip(stopped)).toBe("exit 1");
    expect(chip(finished)).toBeNull();
  });
});

describe("the view's sections", () => {
  const shape = (list: ReturnType<typeof sections>) =>
    list.map((section) => [section.title, section.items.map((item) => item.id)]);

  it("groups by what a row is: needs you, finished, then earlier by when it left", () => {
    expect(shape(sections(all, "kind", "all"))).toEqual([
      ["Needs you", ["asks"]],
      ["Finished, not seen", ["stopped", "finished"]],
      ["Earlier", ["seen", "answered"]],
    ]);
  });

  it("groups by project, what needs an answer first, and keeps Earlier apart", () => {
    const grouped = sections(all, "project", "all");
    expect(shape(grouped)).toEqual([
      ["app", ["asks", "finished"]],
      ["site", ["stopped"]],
      ["Earlier", ["seen", "answered"]],
    ]);
    expect(grouped.map((section) => section.note)).toEqual([
      "1 needs you · 1 finished",
      "1 finished",
      "Kept for a week",
    ]);
    // A row under its project's name does not repeat it; one under Earlier still says it.
    expect(grouped.map((section) => section.namesProject)).toEqual([false, false, true]);
  });

  it("shows one part when asked, and leaves out a section with nothing in it", () => {
    expect(shape(sections(all, "kind", "needs"))).toEqual([["Needs you", ["asks"]]]);
    expect(shape(sections(all, "project", "finished"))).toEqual([
      ["app", ["finished"]],
      ["site", ["stopped"]],
    ]);
    expect(shape(sections(all, "kind", "earlier"))).toEqual([["Earlier", ["seen", "answered"]]]);
    expect(sections([], "kind", "all")).toEqual([]);
  });
});

describe("a row that is not about an agent", () => {
  const approval = standingNotification("approval");
  const question = standingNotification("question", { kind: "workflow.question" });
  const failed = standingNotification("failed", { kind: "workflow.failed", part: "finished" });
  const test = standingNotification("test", { kind: "test.waits", title: "Half-hour zones" });
  const refused = standingNotification("refused", {
    kind: "schedule.refused",
    title: "Nightly review",
  });
  const lost = standingNotification("lost", {
    kind: "trigger.noWorkspace",
    title: "Review when done",
    workspaceId: null,
    branch: null,
  });

  it("says what waits, and what it was once it has left", () => {
    expect(what(approval)).toBe("Workflow waits for your approval");
    expect(what({ ...approval, part: "earlier" })).toBe("Workflow waited for an approval");
    expect(what(question)).toBe("Workflow asks you a question");
    expect(what(failed)).toBe("Workflow failed");
    expect(what({ ...failed, kind: "workflow.succeeded" })).toBe("Workflow finished");
    expect(what(test)).toBe("Smoke test waits for you to approve its file");
    expect(what(refused)).toBe("Schedule could not start its run");
    expect(what({ ...refused, kind: "schedule.noWorkspace" })).toBe(
      "Schedule has no workspace to run in",
    );
    expect(what({ ...lost, kind: "trigger.capped" })).toBe("Trigger reached its limit of runs");
    expect(what({ ...lost, kind: "trigger.refused", part: "earlier" })).toBe(
      "Trigger could not start a run",
    );
  });

  it("knows what each is about, and what its row opens", () => {
    expect([approval, test, refused, lost].map(about)).toEqual([
      "workflow",
      "test",
      "schedule",
      "trigger",
    ]);
    expect(about(notification("agent"))).toBe("agent");
    expect(opens(approval)).toBe("the run");
    expect(opens(test)).toBe("its workspace, where the file is approved");
    expect(opens(refused)).toBe("Schedules");
    expect(opens(lost)).toBe("Triggers");
    expect(opens(notification("agent"))).toBe("the agent's terminal");
    expect(opens(notification("gone", { terminalId: null }))).toBe("its workspace");
  });

  it("draws a failed run red, and nothing that only waits", () => {
    expect(wentBadly(failed)).toBe(true);
    expect([approval, question, test, refused, lost].some(wentBadly)).toBe(false);
  });

  it("leads with the workspace, or with its own name when it has none", () => {
    expect([leadName(approval), detailName(approval)]).toEqual(["fix-login", "Review"]);
    // A trigger whose workspace is gone keeps what it was called.
    expect([leadName(lost), detailName(lost)]).toEqual(["Review when done", "was in fix-login"]);
    // A schedule that makes its own workspaces never had one to name.
    const own = { ...refused, workspaceId: null, workspaceName: "" };
    expect([leadName(own), detailName(own)]).toEqual(["Nightly review", null]);
  });
});
