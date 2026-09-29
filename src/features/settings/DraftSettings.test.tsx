import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { HarnessInfo } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  draftStatus: vi.fn(),
  workflowWriterStatus: vi.fn(),
  workflowSaveWriter: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { useDraftStore } from "@/stores/draft";
import { useHarnessStore } from "@/stores/harnesses";
import { DraftSettings } from "./DraftSettings";

const status = {
  enabled: true,
  available: true,
  harness: "Claude Code",
  key: false,
  model: "claude-opus-5",
  problem: null,
};
const harness = (id: string, extra: Partial<HarnessInfo> = {}): HarnessInfo => ({
  id,
  label: id,
  command: id,
  baseArgs: [],
  promptArgs: [],
  resumeArgs: [],
  forkArgs: [],
  modelArgs: [],
  effortArgs: [],
  sessionArgs: [],
  writeArgs: ["{prompt}"],
  models: [],
  efforts: [],
  promptTransport: "argv",
  sessionIdMode: "assigned",
  stdinReadyMs: 0,
  enabled: true,
  resolvedPath: `/bin/${id}`,
  builtin: false,
  modified: false,
  ...extra,
});

beforeEach(() => {
  vi.clearAllMocks();
  useDraftStore.setState({ status: null, workflowWriter: null });
  core.draftStatus.mockResolvedValue(status);
  core.workflowWriterStatus.mockResolvedValue({ harnessId: "codex", status });
  core.workflowSaveWriter.mockImplementation(async (harnessId) => ({ harnessId, status }));
  useHarnessStore.setState({
    loaded: true,
    harnesses: [
      harness("claude"),
      harness("codex"),
      harness("custom"),
      harness("missing", { resolvedPath: null }),
      harness("disabled", { enabled: false }),
      harness("interactive", { writeArgs: [] }),
    ],
  });
});

it("loads the saved workflow writer, saves a custom harness, and restores automatic selection", async () => {
  const user = userEvent.setup();
  const view = render(<DraftSettings />);
  const select = await screen.findByRole("combobox", { name: "Workflow writer" });
  expect(select).toHaveValue("codex");
  for (const name of [
    "missing (not installed)",
    "disabled (disabled)",
    "interactive (needs Write args)",
  ]) {
    expect(screen.getByRole("option", { name })).toBeDisabled();
  }
  await user.selectOptions(select, "custom");
  await waitFor(() => expect(select).toHaveValue("custom"));
  expect(core.workflowSaveWriter).toHaveBeenCalledWith("custom");
  view.unmount();
  core.workflowWriterStatus.mockResolvedValue({ harnessId: "custom", status });
  render(<DraftSettings />);
  const reopened = await screen.findByRole("combobox", { name: "Workflow writer" });
  await waitFor(() => expect(reopened).toHaveValue("custom"));
  await user.selectOptions(reopened, "");
  await waitFor(() => expect(reopened).toHaveValue(""));
  expect(core.workflowSaveWriter).toHaveBeenLastCalledWith(null);
});

it("keeps a removed writer visible and preserves the saved choice when a save fails", async () => {
  core.workflowWriterStatus.mockResolvedValue({
    harnessId: "removed",
    status: { ...status, problem: "Selected writer unavailable." },
  });
  core.workflowSaveWriter.mockRejectedValue({
    code: "failed",
    message: "Could not save settings.",
  });
  render(<DraftSettings />);
  const select = await screen.findByRole("combobox", { name: "Workflow writer" });
  expect(select).toHaveValue("removed");
  expect(screen.getByRole("option", { name: "removed (unavailable)" })).toBeDisabled();
  await userEvent.setup().selectOptions(select, "codex");
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not save settings.");
  expect(select).toHaveValue("removed");
});
