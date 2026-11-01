import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  testDescribe: vi.fn(),
  testSave: vi.fn(),
  workflowWriterStatus: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useDraftStore } from "@/stores/draft";
import { DescribeTest } from "./DescribeTest";

const workspaces = [
  { id: "w-app-fix", name: "fix" },
  { id: "w-app-main", name: "local" },
];

describe("DescribeTest", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.workflowWriterStatus.mockResolvedValue({
      harnessId: "claude",
      status: {
        enabled: true,
        available: true,
        harness: "Claude",
        key: false,
        model: "",
        problem: null,
      },
    });
    useDraftStore.setState({ workflowWriter: null });
  });

  it("writes a test from a description, then saves it into the chosen workspace", async () => {
    core.testDescribe.mockResolvedValue({
      text: "id: login\nname: Log in\nversion: 1\nsteps:\n  - id: go\n    action: run\n    program: echo\n",
      problems: [],
      tries: 1,
      writer: "Claude",
    });
    core.testSave.mockResolvedValue("/worktrees/app/fix/.yardsort/tests/login.yaml");
    const onSaved = vi.fn();
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<DescribeTest workspaces={workspaces} onClose={onClose} onSaved={onSaved} />);

    const area = screen.getByRole("textbox", { name: "What the smoke test should check" });
    await waitFor(() => expect(area).toBeEnabled());
    await user.type(area, "Check login works");
    await waitFor(() => expect(screen.getByRole("button", { name: "Write it" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Write it" }));

    // The written file appears, editable, with a note naming the writer.
    const file = await screen.findByRole("textbox", { name: "The smoke test file" });
    expect((file as HTMLTextAreaElement).value).toContain("id: login");
    expect(screen.getByText(/Written by Claude/)).toBeInTheDocument();

    // Choose where to save, and save.
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Save in workspace" }),
      "w-app-fix",
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(core.testSave).toHaveBeenCalled());
    expect(core.testSave).toHaveBeenCalledWith("w-app-fix", expect.stringContaining("id: login"));
    expect(onSaved).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("will not write with no model available", async () => {
    core.workflowWriterStatus.mockResolvedValue({
      harnessId: null,
      status: {
        enabled: false,
        available: false,
        harness: null,
        key: false,
        model: "",
        problem: null,
      },
    });
    const user = userEvent.setup();
    render(<DescribeTest workspaces={workspaces} onClose={() => {}} onSaved={() => {}} />);
    await screen.findByText(/switched off in Settings/);
    await user.type(
      screen.getByRole("textbox", { name: "What the smoke test should check" }),
      "anything",
    );
    expect(screen.getByRole("button", { name: "Write it" })).toBeDisabled();
  });
});
