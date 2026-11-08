import { describe, expect, it } from "vitest";
import { trigger } from "@/test/fixtures";
import {
  defaultQuietMs,
  durationWords,
  sentence,
  standingWords,
  subjectOf,
  type Asking,
} from "./words";

const asking = (overrides: Partial<Asking> = {}): Asking => ({
  events: ["pr.review", "pr.comment"],
  filters: {},
  target: "workflow",
  targetId: "address-review",
  inputs: {},
  quietMs: 120_000,
  cap: 3,
  oneAgent: false,
  ...overrides,
});

describe("what a trigger will do, in words", () => {
  it("says the Address reviews preset as the design does", () => {
    expect(sentence(asking())).toBe(
      "When someone other than you reviews or comments on this workspace's pull request, 2 minutes after the last one, tell the agent to address it. At most 3 times per pull request.",
    );
  });

  it("says whose posts count, which reviews, and that bots are left out", () => {
    expect(
      sentence(
        asking({
          events: ["pr.review"],
          filters: { from: "anyone", states: ["changes_requested"], noBots: true },
          quietMs: 0,
          cap: 1,
        }),
      ),
    ).toBe(
      "When anyone reviews this workspace's pull request (changes requested), leaving out bots, tell the agent to address it. At most once per pull request.",
    );
  });

  it("names the reviewer a review when done asks for, and counts per workspace", () => {
    expect(
      sentence(
        asking({
          events: ["workspace.done"],
          targetId: "review-branch",
          inputs: { reviewer: "codex" },
          quietMs: 0,
        }),
      ),
    ).toBe(
      "When an agent says its work is done, have codex review the branch, and tell the agent what it found. At most 3 times per workspace.",
    );
  });

  it("is about one agent for a session trigger, with its filters", () => {
    expect(
      sentence(
        asking({
          events: ["agent.settled"],
          filters: { newCommits: true, clean: true },
          target: "test",
          targetId: "login",
          quietMs: 180_000,
          oneAgent: true,
        }),
      ),
    ).toBe(
      "When this agent finishes a turn, with new commits and nothing uncommitted, 3 minutes after the last one, run the smoke test login. At most 3 times per agent session.",
    );
  });

  it("says nothing until there is an event and something to run", () => {
    expect(sentence(asking({ events: [] }))).toBe("");
    expect(sentence(asking({ targetId: "" }))).toBe("");
  });
});

describe("the rest of a trigger's words", () => {
  it("knows each event's subject and the quiet time it starts with", () => {
    expect(subjectOf("agent.exited")).toBe("session");
    expect(subjectOf("pr.merged")).toBe("pull request");
    expect(subjectOf("test.finished")).toBe("workspace");
    expect(defaultQuietMs(["pr.checks_failed"])).toBe(0);
    expect(defaultQuietMs(["pr.checks_failed", "pr.comment"])).toBe(120_000);
  });

  it("writes lengths of time the way a person would", () => {
    expect(durationWords(120_000)).toBe("2 minutes");
    expect(durationWords(30_000)).toBe("30 seconds");
    expect(durationWords(3_600_000)).toBe("1 hour");
    expect(durationWords(90_000)).toBe("90 seconds");
  });

  it("calls a trigger whose agent ended ended, not on", () => {
    expect(standingWords(trigger("t"))).toBe("on");
    expect(standingWords(trigger("t", { sessionId: "s1", sessionRunning: false }))).toBe("ended");
    expect(standingWords(trigger("t", { sessionId: "s1", sessionRunning: true }))).toBe("on");
    expect(standingWords(trigger("t", { standing: "capped" }))).toBe("capped");
  });
});
