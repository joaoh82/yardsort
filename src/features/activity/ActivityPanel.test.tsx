import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEvent } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  activityTimeline: vi.fn(),
  activityClear: vi.fn(),
  onActivityChanged: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { useActivityStore } from "@/stores/activity";
import { ActivityPanel } from "./ActivityPanel";
import { describeEvent } from "./describe";

const event = (seq: number, kind: string, payload: object): ActivityEvent => ({
  seq,
  id: `e-${seq}`,
  schemaVersion: 1,
  workspaceId: "ws",
  sessionId: null,
  runId: "run-1",
  occurredAt: 1_790_000_000_000 + seq * 1000,
  receivedAt: 1_790_000_000_000 + seq * 1000,
  kind,
  producer: "yardsort",
  method: "lifecycle",
  fidelity: "observed",
  privacyClass: "metadata",
  payload: JSON.stringify(payload),
});

beforeEach(() => {
  vi.clearAllMocks();
  useActivityStore.setState({ byWorkspace: {}, loading: {}, error: null });
  core.activityClear.mockResolvedValue(undefined);
  core.onActivityChanged.mockResolvedValue(() => {});
});

describe("ActivityPanel", () => {
  it("shows the newest events with their source, and pages back through earlier ones", async () => {
    const user = userEvent.setup();
    core.activityTimeline.mockImplementation(async (_ws: string, before: number | null) =>
      before === null
        ? {
            events: [
              event(3, "process.exited", { exitCode: 2, success: false, via: "spool" }),
              event(2, "process.started", {
                kind: "harness",
                harnessId: "claude",
                model: "opus",
                continuation: "resumed",
                launchedBy: "cli",
              }),
            ],
            hasMore: true,
          }
        : {
            events: [event(1, "process.started", { kind: "shell", continuation: "fresh" })],
            hasMore: false,
          },
    );
    render(<ActivityPanel workspaceId="ws" onClose={() => {}} />);

    const panel = screen.getByRole("region", { name: "Activity" });
    await waitFor(() => expect(within(panel).getAllByRole("listitem")).toHaveLength(2));
    const rows = within(panel).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("exited with 2");
    expect(rows[0]).toHaveTextContent("kept by the background process while Yardsort was closed");
    expect(rows[0]).toHaveTextContent("yardsort/lifecycle");
    expect(rows[1]).toHaveTextContent("claude resumed");
    expect(rows[1]).toHaveTextContent("opus · from ys");
    expect(panel).toHaveTextContent("Each row names its source");
    expect(core.activityTimeline).toHaveBeenLastCalledWith("ws", null, 50);

    await user.click(within(panel).getByRole("button", { name: "Show earlier" }));
    await waitFor(() => expect(within(panel).getAllByRole("listitem")).toHaveLength(3));
    expect(core.activityTimeline).toHaveBeenLastCalledWith("ws", 2, 50);
    expect(within(panel).getAllByRole("listitem")[2]).toHaveTextContent("shell started");
    expect(within(panel).queryByRole("button", { name: "Show earlier" })).toBeNull();
  });

  it("says so when nothing is recorded, and clears what is", async () => {
    const user = userEvent.setup();
    core.activityTimeline
      .mockResolvedValueOnce({
        events: [event(1, "process.spawn_failed", { program: "claude", reason: "not found" })],
        hasMore: false,
      })
      .mockResolvedValue({ events: [], hasMore: false });
    render(<ActivityPanel workspaceId="ws" onClose={() => {}} />);
    const panel = screen.getByRole("region", { name: "Activity" });
    await waitFor(() =>
      expect(within(panel).getByRole("listitem")).toHaveTextContent("could not start"),
    );

    await user.click(within(panel).getByRole("button", { name: "Clear" }));
    expect(core.activityClear).toHaveBeenCalledWith("ws");
    await waitFor(() => expect(panel).toHaveTextContent("Nothing recorded for this workspace yet"));
  });

  it("shows what the agent reported, marked as the agent's word, and reloads when told", async () => {
    let notify: ((ids: string[]) => void) | undefined;
    core.onActivityChanged.mockImplementation((handler: (ids: string[]) => void) => {
      notify = handler;
      return Promise.resolve(() => {});
    });
    const reported = (seq: number, kind: string, payload: object): ActivityEvent => ({
      ...event(seq, kind, payload),
      producer: "claude",
      method: "hook",
      fidelity: "reported",
    });
    core.activityTimeline.mockResolvedValueOnce({
      events: [reported(2, "tool.completed", { tool: "Edit", path: "src/a.rs", durationMs: 12 })],
      hasMore: false,
    });
    render(<ActivityPanel workspaceId="ws" onClose={() => {}} />);
    const panel = screen.getByRole("region", { name: "Activity" });
    await waitFor(() => expect(within(panel).getAllByRole("listitem")).toHaveLength(1));
    expect(within(panel).getByRole("listitem")).toHaveTextContent("Edit done");
    expect(within(panel).getByRole("listitem")).toHaveTextContent("src/a.rs · 12 ms");
    expect(within(panel).getByRole("listitem")).toHaveTextContent("claude/hook");

    core.activityTimeline.mockResolvedValueOnce({
      events: [
        reported(3, "turn.completed", {}),
        reported(2, "tool.completed", { tool: "Edit", path: "src/a.rs", durationMs: 12 }),
      ],
      hasMore: false,
    });
    notify?.(["other-ws"]);
    expect(core.activityTimeline).toHaveBeenCalledTimes(1);
    notify?.(["ws"]);
    await waitFor(() => expect(within(panel).getAllByRole("listitem")).toHaveLength(2));
    expect(within(panel).getAllByRole("listitem")[0]).toHaveTextContent("agent finished its turn");
  });

  it("reports a failure to load rather than showing an empty list", async () => {
    core.activityTimeline.mockRejectedValue({ code: "database", message: "database is locked" });
    render(<ActivityPanel workspaceId="ws" onClose={() => {}} />);
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("database is locked"));
  });
});

describe("describeEvent", () => {
  it("reads every stage-1 kind, and leaves an unknown one recognisable", () => {
    const words = (kind: string, payload: object) => describeEvent(event(1, kind, payload));
    expect(words("process.started", { kind: "shell", continuation: "fresh" }).title).toBe(
      "shell started",
    );
    expect(words("process.started", { harnessId: "pi", continuation: "forked" }).title).toBe(
      "pi forked",
    );
    expect(words("process.exited", { exitCode: 0, success: true, via: "live" })).toEqual({
      title: "exited",
      detail: "seen as it happened",
      tone: "ok",
    });
    expect(
      words("process.exited", { exitCode: null, reason: "interrupted", via: "reconcile" }),
    ).toEqual({
      title: "ended without an exit status",
      detail: "interrupted · process gone when Yardsort started",
      tone: "bad",
    });
    expect(words("session.forked", { fromSessionId: "abcdef0123" }).detail).toBe("from abcdef01");
    expect(
      words("process.started", { harnessId: "claude", continuation: "fresh", capture: "hook" })
        .detail,
    ).toBe("reporting through hooks");
    expect(words("tool.started", { tool: "Bash" })).toEqual({
      title: "Bash started",
      detail: "",
      tone: "plain",
    });
    expect(words("tool.started", { tool: "Agent", subagentType: "Explore" }).title).toBe(
      "Agent (Explore) started",
    );
    expect(words("tool.failed", { tool: "Read", pathOutsideWorkspace: true })).toEqual({
      title: "Read failed",
      detail: "a file outside the workspace",
      tone: "bad",
    });
    expect(words("approval.resolved", { tool: "Bash", decision: "denied" }).tone).toBe("bad");
    expect(words("session.started", { source: "resume", contextTokens: 29241 })).toEqual({
      title: "agent resumed its session",
      detail: "29,241 tokens of context",
      tone: "plain",
    });
    expect(words("prompt.submitted", { chars: 147 }).detail).toBe("147 characters");
    expect(words("agent.notified", { type: "permission_prompt" }).detail).toBe("permission_prompt");
    expect(words("turn.failed", { errorType: "rate_limit" }).tone).toBe("bad");
    expect(words("usage.reported", { tokens: 1 }).title).toBe("usage.reported");
    expect(describeEvent({ ...event(1, "process.exited", {}), payload: "{ not json" }).title).toBe(
      "ended without an exit status",
    );
  });
});
