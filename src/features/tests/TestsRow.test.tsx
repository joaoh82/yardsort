import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
}));

import { useProjectsStore } from "@/stores/projects";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { testRun } from "@/test/fixtures";
import { TestsRow } from "./TestsRow";

describe("TestsRow", () => {
  beforeEach(() => {
    useProjectsStore.setState({ testsOpen: false });
    useSmokeTestsStore.setState({ runs: [] });
  });

  it("shows no badge when nothing waits", () => {
    render(<TestsRow />);
    expect(screen.getByRole("button", { name: "Tests" })).toBeInTheDocument();
  });

  it("counts the runs awaiting approval across workspaces", () => {
    useSmokeTestsStore.setState({
      runs: [
        testRun("r1", { status: "awaiting_approval" }),
        testRun("r2", { status: "awaiting_approval", workspaceId: "w-other" }),
        testRun("r3", { status: "passed" }),
      ],
    });
    render(<TestsRow />);
    expect(
      screen.getByRole("button", { name: "Tests, 2 runs waiting for approval" }),
    ).toBeInTheDocument();
  });

  it("opens the Tests view when clicked", async () => {
    const user = userEvent.setup();
    render(<TestsRow />);
    await user.click(screen.getByRole("button", { name: "Tests" }));
    expect(useProjectsStore.getState().testsOpen).toBe(true);
  });
});
