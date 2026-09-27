import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Attempt, ProjectOutcomes } from "@/lib/ipc";

const core = vi.hoisted(() => ({ outcomesGet: vi.fn(), outcomeLabel: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { project } from "@/test/fixtures";
import { OutcomesDialog } from "./OutcomesDialog";

const attempt = (id: string, extra: Partial<Attempt> = {}): Attempt => ({
  id,
  workspaceId: id,
  workspaceName: id,
  branch: `ys/${id}`,
  baseBranch: "main",
  task: `Do ${id}`,
  harnesses: ["claude"],
  label: null,
  outcome: null,
  outcomeSource: null,
  evidence: { ahead: false, mergedIntoBase: false, prNumber: null, prState: null, ended: null },
  createdAt: 0,
  ...extra,
});

const outcomes = (attempts: Attempt[]): ProjectOutcomes => ({
  projectId: "p-app",
  attempts,
  agents: [
    {
      harness: "claude",
      attempts: 6,
      kept: 3,
      keptByMerge: 1,
      partly: 1,
      discarded: 1,
      known: 5,
      enough: true,
    },
    {
      harness: "codex",
      attempts: 2,
      kept: 1,
      keptByMerge: 0,
      partly: 0,
      discarded: 0,
      known: 1,
      enough: false,
    },
  ],
  minSample: 5,
});

describe("OutcomesDialog", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shows each agent's history without overstating a small sample, and each attempt's evidence", async () => {
    core.outcomesGet.mockResolvedValue(
      outcomes([
        attempt("merged-one", {
          outcome: "kept",
          outcomeSource: "merge",
          evidence: {
            ahead: true,
            mergedIntoBase: false,
            prNumber: 8,
            prState: "merged",
            ended: "deleted",
          },
        }),
        attempt("labelled", { label: "discarded", outcome: "discarded", outcomeSource: "you" }),
        attempt("open", {
          evidence: {
            ahead: true,
            mergedIntoBase: false,
            prNumber: null,
            prState: null,
            ended: null,
          },
        }),
      ]),
    );
    render(<OutcomesDialog project={project("app")} onClose={() => {}} />);
    const agents = await screen.findByRole("region", { name: "Agents" });
    expect(agents).toHaveTextContent("kept 3 of 5 (1 by merge) · partly 1 · discarded 1");
    expect(agents).toHaveTextContent("too few to say yet — 1 outcome so far");

    const merged = screen.getByRole("listitem", { name: "merged-one" });
    expect(merged).toHaveTextContent("kept — merged, not labelled");
    expect(merged).toHaveTextContent("PR #8 merged · ahead of main · deleted");
    expect(screen.getByRole("listitem", { name: "labelled" })).toHaveTextContent("discarded");
    const open = screen.getByRole("listitem", { name: "open" });
    expect(open).toHaveTextContent("no outcome yet");
    expect(open).toHaveTextContent("ahead of main");
  });

  it("labels an attempt, and takes a label back when it is chosen again", async () => {
    const user = userEvent.setup();
    core.outcomesGet.mockResolvedValue(
      outcomes([
        attempt("a"),
        attempt("b", { label: "kept", outcome: "kept", outcomeSource: "you" }),
      ]),
    );
    core.outcomeLabel.mockResolvedValue(outcomes([attempt("a"), attempt("b")]));
    render(<OutcomesDialog project={project("app")} onClose={() => {}} />);
    const b = await screen.findByRole("listitem", { name: "b" });
    expect(within(b).getByRole("button", { name: "Kept" })).toHaveAttribute("aria-pressed", "true");
    await user.click(within(b).getByRole("button", { name: "Kept" }));
    expect(core.outcomeLabel).toHaveBeenLastCalledWith("b", null);
    const a = screen.getByRole("listitem", { name: "a" });
    await user.click(within(a).getByRole("button", { name: "Kept" }));
    expect(core.outcomeLabel).toHaveBeenLastCalledWith("a", "kept");
  });
});
