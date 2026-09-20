import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AssistStatus, Review } from "@/lib/ipc";

const core = vi.hoisted(() => ({ assistStatus: vi.fn(), assistReview: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { REVIEW_DELAY_MS, useAssistStore } from "./assist";

const status = (extra: Partial<AssistStatus> = {}): AssistStatus => ({
  keySource: "keychain",
  keyHint: "…1234",
  problem: null,
  reviewChanges: true,
  suggestInComposer: true,
  thresholds: {
    flagAtPercent: 70,
    offTaskAtPercent: 60,
    suggestAtPercent: 50,
    defaults: [70, 60, 50],
    range: [5, 95],
  },
  model: "jev-1.13.0",
  ...extra,
});

const review = (extra: Partial<Review> = {}): Review => ({
  files: [{ path: "a.ts", scope: "uncommitted", relevance: "direct", flags: [], notChecked: null }],
  task: "Fix the login bug",
  model: "jev-1.13.0",
  problem: null,
  ...extra,
});

const store = () => useAssistStore.getState();

describe("assist store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    core.assistStatus.mockResolvedValue(status());
    core.assistReview.mockResolvedValue(review());
    useAssistStore.setState({
      status: status(),
      workspaceId: null,
      review: null,
      reviewing: false,
      error: null,
    });
  });
  afterEach(() => vi.useRealTimers());

  it("checks a workspace's changes once the writing has settled, not on every signal", async () => {
    store().follow("w1");
    store().reviewSoon();
    store().reviewSoon();
    expect(core.assistReview).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    expect(core.assistReview).toHaveBeenCalledExactlyOnceWith("w1");
    expect(store().review?.files[0]!.relevance).toBe("direct");
  });

  it("asks nothing at all while Assist is off or has no key", async () => {
    for (const off of [status({ reviewChanges: false }), status({ keySource: "none" })]) {
      useAssistStore.setState({ status: off, workspaceId: null, review: null });
      store().follow("w1");
      await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    }
    expect(core.assistReview).not.toHaveBeenCalled();
  });

  it("drops an answer that arrives after the panel moved on", async () => {
    store().follow("w1");
    await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    expect(store().review).not.toBeNull();

    core.assistReview.mockImplementation(
      () => new Promise((resolve) => setTimeout(() => resolve(review()), 50)),
    );
    store().follow("w2");
    expect(store().review).toBeNull();
    await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    store().follow("w3");
    await vi.advanceTimersByTimeAsync(100);
    expect(store().review).toBeNull();
  });

  it("shows a failure but stays quiet about being switched off", async () => {
    core.assistReview.mockRejectedValue({ code: "assist_rate_limited", message: "Too many." });
    store().follow("w1");
    await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    expect(store().error).toBe("Too many.");

    core.assistReview.mockRejectedValue({ code: "assist_off", message: "Switched off." });
    store().follow("w2");
    await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    expect(store().error).toBeNull();
  });

  it("carries a partial review's problem through, and forgets the review when switched off", async () => {
    core.assistReview.mockResolvedValue(review({ problem: "TypeSafe is rate limiting." }));
    store().follow("w1");
    await vi.advanceTimersByTimeAsync(REVIEW_DELAY_MS);
    expect(store().error).toBe("TypeSafe is rate limiting.");
    expect(store().review?.files).toHaveLength(1);

    store().adopt(status({ reviewChanges: false }));
    expect(store().review).toBeNull();
    expect(store().error).toBeNull();
  });
});
