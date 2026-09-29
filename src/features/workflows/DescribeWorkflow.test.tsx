import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  workflowWriterStatus: vi.fn(),
  workflowDescribe: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useDraftStore } from "@/stores/draft";
import { NEW_WORKFLOW, useWorkflowStore } from "@/stores/workflows";
import { DescribeWorkflow } from "./DescribeWorkflow";

const canWrite = {
  enabled: true,
  available: true,
  harness: "Claude Code",
  key: false,
  model: "claude-opus-5",
  problem: null,
};

describe("DescribeWorkflow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useDraftStore.setState({ status: null, workflowWriter: null, busy: null, error: null });
    useWorkflowStore.setState({ drafts: {} });
  });

  it("asks the model, and what it wrote becomes the new workflow's unsaved text", async () => {
    core.workflowWriterStatus.mockResolvedValue({ harnessId: null, status: canWrite });
    core.workflowDescribe.mockResolvedValue({
      text: "id: review\nname: Review\n",
      problems: [],
      tries: 1,
      writer: "Claude Code",
    });
    const user = userEvent.setup();
    render(<DescribeWorkflow />);
    const write = screen.getByRole("button", { name: "Write it" });
    await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
    expect(write).toBeDisabled();
    await user.type(screen.getByRole("textbox"), "Review my pull request.");
    await user.click(write);
    expect(core.workflowDescribe).toHaveBeenCalledWith("Review my pull request.");
    await waitFor(() =>
      expect(useWorkflowStore.getState().drafts[NEW_WORKFLOW]).toBe("id: review\nname: Review\n"),
    );
    expect(screen.getByText(/Written by Claude Code\. Look it over/)).toBeInTheDocument();
  });

  it("shows the configured workflow writer independently of workspace drafting", async () => {
    core.workflowWriterStatus.mockResolvedValue({
      harnessId: "codex",
      status: { ...canWrite, harness: "Codex" },
    });
    useDraftStore.setState({ status: canWrite });
    core.workflowDescribe.mockReturnValue(new Promise(() => {}));
    render(<DescribeWorkflow />);
    expect(await screen.findByText(/Written by Codex/)).toBeInTheDocument();
    const user = userEvent.setup();
    await user.type(screen.getByRole("textbox"), "Review it");
    await user.click(screen.getByRole("button", { name: "Write it" }));
    expect(screen.getByRole("button", { name: "Asking Codex…" })).toBeDisabled();
  });

  it("refreshes the displayed writer when settings changes while the workflow is open", async () => {
    core.workflowWriterStatus.mockResolvedValue({ harnessId: null, status: canWrite });
    render(<DescribeWorkflow />);
    expect(await screen.findByText(/Written by Claude Code/)).toBeInTheDocument();
    act(() =>
      useDraftStore.setState({
        workflowWriter: { harnessId: "codex", status: { ...canWrite, harness: "Codex" } },
      }),
    );
    expect(screen.getByText(/Written by Codex/)).toBeInTheDocument();
  });

  it("says how many problems were left, and shows what went wrong", async () => {
    core.workflowWriterStatus.mockResolvedValue({ harnessId: null, status: canWrite });
    core.workflowDescribe.mockResolvedValueOnce({
      text: "id: x\n",
      problems: [{ line: 1, column: 1, message: "Missing field `name`" }],
      tries: 2,
      writer: "Claude Code",
    });
    const user = userEvent.setup();
    render(<DescribeWorkflow />);
    await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
    await user.type(screen.getByRole("textbox"), "Something.");
    await user.click(screen.getByRole("button", { name: "Write it" }));
    expect(await screen.findByText(/with 1 problem left to fix/)).toBeInTheDocument();

    core.workflowDescribe.mockRejectedValueOnce({
      code: "draft_agent_failed",
      message: "Claude Code said: not logged in",
    });
    await user.click(screen.getByRole("button", { name: "Write it" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("not logged in");
  });

  it("a slower answer never replaces a newer one, or a discard", async () => {
    core.workflowWriterStatus.mockResolvedValue({ harnessId: null, status: canWrite });
    let finishA: (v: unknown) => void = () => {};
    let finishB: (v: unknown) => void = () => {};
    core.workflowDescribe
      .mockReturnValueOnce(new Promise((resolve) => (finishA = resolve)))
      .mockReturnValueOnce(new Promise((resolve) => (finishB = resolve)));
    const answer = (id: string) => ({
      text: `id: ${id}\n`,
      problems: [],
      tries: 1,
      writer: "Claude Code",
    });
    const user = userEvent.setup();

    // Ask once, leave the workflow, come back and ask again; B answers first, then A.
    const first = render(<DescribeWorkflow />);
    await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
    await user.type(screen.getByRole("textbox"), "A");
    await user.click(screen.getByRole("button", { name: "Write it" }));
    first.unmount();
    render(<DescribeWorkflow />);
    await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
    await user.type(screen.getByRole("textbox"), "B");
    await user.click(screen.getByRole("button", { name: "Write it" }));
    finishB(answer("b"));
    await waitFor(() => expect(useWorkflowStore.getState().drafts[NEW_WORKFLOW]).toBe("id: b\n"));
    finishA(answer("a"));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(useWorkflowStore.getState().drafts[NEW_WORKFLOW]).toBe("id: b\n");

    // A discard while an answer is on its way sets that answer aside too.
    let finishC: (v: unknown) => void = () => {};
    core.workflowDescribe.mockReturnValueOnce(new Promise((resolve) => (finishC = resolve)));
    await user.type(screen.getByRole("textbox"), "C");
    await user.click(screen.getByRole("button", { name: "Write it" }));
    useWorkflowStore.getState().setDraft(NEW_WORKFLOW, null);
    finishC(answer("c"));
    expect(await screen.findByText(/set aside/)).toBeInTheDocument();
    expect(useWorkflowStore.getState().drafts[NEW_WORKFLOW]).toBeUndefined();
  });

  it("says why nothing can write, and offers nothing to press", async () => {
    core.workflowWriterStatus.mockResolvedValue({
      harnessId: "codex",
      status: {
        ...canWrite,
        available: false,
        harness: null,
        problem: "No agent here can write one, and there is no Anthropic API key.",
      },
    });
    render(<DescribeWorkflow />);
    expect(await screen.findByText(/No agent here can write one/)).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Write it" })).toBeDisabled();
  });
});
