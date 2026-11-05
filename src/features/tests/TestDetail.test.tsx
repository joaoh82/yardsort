import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { TestListing, TestRun } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { project, testRun, worktree } from "@/test/fixtures";
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

  it("offers to schedule the test, in the workspace chosen to run it in", async () => {
    useProjectsStore.setState({
      projects: [
        {
          ...project("app"),
          workspaces: [...project("app").workspaces, worktree("app", "fix-login")],
        },
      ],
    });
    const user = userEvent.setup();
    render(
      <TestDetail
        listing={listing()}
        catalog={[listing()]}
        runs={[]}
        now={Date.now()}
        onOpenRun={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Schedule…" }));
    const dialog = await screen.findByRole("dialog", { name: "New schedule" });
    expect(within(dialog).getByRole("combobox", { name: "What to run" })).toHaveValue("test");
    expect(within(dialog).getByRole("combobox", { name: "Workspace" })).toHaveValue(
      "w-app-fix-login",
    );
  });

  it("does not offer to schedule a test that cannot run", () => {
    render(
      <TestDetail
        listing={listing({ item: { ...listing().item, runnable: false } })}
        catalog={[]}
        runs={[]}
        now={Date.now()}
        onOpenRun={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Schedule…" })).toBeDisabled();
  });
});
