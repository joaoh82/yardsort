import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TestListing, TestRun } from "@/lib/ipc";
import { testRun } from "@/test/fixtures";
import { TestDetail } from "./TestDetail";

const listing = (over: Partial<TestListing> = {}): TestListing => ({
  item: {
    id: "health",
    name: "The server answers",
    description: null,
    path: "/worktrees/app/fix-login/.yardsort/tests/health.yaml",
    text: "id: health\n",
    problems: [],
    runnable: true,
    usesBrowser: false,
    browserMissing: false,
    inputs: [],
    ...over.item,
  },
  projectId: "p-app",
  projectName: "app",
  workspaceId: "w-app-fix-login",
  workspaceName: "fix-login",
  envMissing: [],
  lastRun: null,
  ...over,
});

describe("TestDetail", () => {
  it("shows only this project's runs, not another project's same test id", () => {
    const runs: TestRun[] = [
      testRun("mine", { projectId: "p-app", testId: "health", workspaceName: "fix-login" }),
      testRun("theirs", {
        projectId: "p-other",
        testId: "health",
        workspaceName: "other-ws",
        status: "failed",
      }),
    ];
    render(
      <TestDetail
        listing={listing()}
        catalog={[listing()]}
        runs={runs}
        now={Date.now()}
        onOpenRun={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    const history = screen.getByRole("list", { name: "Run history" });
    expect(within(history).getByText("fix-login")).toBeInTheDocument();
    // The other project's `health` run must not leak into this one's history.
    expect(within(history).queryByText("other-ws")).toBeNull();
  });
});
