import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MemoryEntry, ProjectMemory } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  memoryGet: vi.fn(),
  memoryWrite: vi.fn(),
  memoryEdit: vi.fn(),
  memoryDecide: vi.fn(),
  memoryShare: vi.fn(),
  memoryWaiting: vi.fn(),
  memoryCheck: vi.fn(),
  assistStatus: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useAssistStore } from "@/stores/assist";
import { project } from "@/test/fixtures";
import { MemoryDialog } from "./MemoryDialog";

const entry = (id: string, text: string, extra: Partial<MemoryEntry> = {}): MemoryEntry => ({
  id,
  shortId: id.slice(0, 8),
  text,
  state: "approved",
  author: "user",
  from: "the user",
  createdAt: 0,
  updatedAt: 0,
  history: [],
  ...extra,
});
const memory = (entries: MemoryEntry[], shared = false): ProjectMemory => ({
  projectId: "p-app",
  shared,
  preview: null,
  entries,
});
const proposal = entry("cand0001", "Run just check before committing.", {
  state: "candidate",
  author: "agent",
  from: "claude in fix-login",
});
const kept = entry("appr0001", "The tests need TZ=UTC.");
const gone = entry("revk0001", "Use npm.", { state: "revoked" });

function status(checkMemory: boolean) {
  return {
    keySource: "keychain" as const,
    keyHint: "…1234",
    problem: null,
    reviewChanges: false,
    suggestInComposer: false,
    sendProvenance: false,
    checkMemory,
    thresholds: {
      flagAtPercent: 70,
      offTaskAtPercent: 60,
      suggestAtPercent: 50,
      defaults: [70, 60, 50] as [number, number, number],
      range: [5, 95] as [number, number],
    },
    model: "jev-1.13.0",
  };
}

async function open() {
  render(<MemoryDialog project={project("app")} onClose={() => {}} />);
  await screen.findByRole("dialog", { name: "Memory — app" });
  await screen.findByText("The tests need TZ=UTC.");
  return userEvent.setup();
}

describe("MemoryDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.memoryGet.mockResolvedValue(memory([kept, proposal, gone]));
    core.memoryWaiting.mockResolvedValue([]);
    core.memoryCheck.mockResolvedValue([]);
    core.assistStatus.mockResolvedValue(status(false));
    useAssistStore.setState({ status: status(false) });
  });

  it("sorts entries into waiting, approved and set aside, each with where it came from", async () => {
    await open();
    const waiting = screen.getByRole("region", { name: "Waiting for you" });
    expect(waiting).toHaveTextContent("Run just check before committing.");
    expect(waiting).toHaveTextContent("from claude in fix-login · cand0001");
    const approved = screen.getByRole("region", { name: "Approved" });
    expect(approved).toHaveTextContent("The tests need TZ=UTC.");
    expect(approved).toHaveTextContent("from the user");
    expect(approved).not.toHaveTextContent("Use npm.");
    expect(screen.getByText(/Rejected and revoked · 1/)).toBeInTheDocument();
  });

  it("approves and rejects proposals, revokes and restores entries, and says so to the core", async () => {
    const user = await open();
    core.memoryDecide.mockResolvedValue(memory([kept, proposal, gone]));
    const waiting = screen.getByRole("region", { name: "Waiting for you" });
    await user.click(within(waiting).getByRole("button", { name: "Approve" }));
    expect(core.memoryDecide).toHaveBeenLastCalledWith("cand0001", "approve");
    await user.click(within(waiting).getByRole("button", { name: "Reject" }));
    expect(core.memoryDecide).toHaveBeenLastCalledWith("cand0001", "reject");
    const approved = screen.getByRole("region", { name: "Approved" });
    await user.click(within(approved).getByRole("button", { name: "Revoke" }));
    expect(core.memoryDecide).toHaveBeenLastCalledWith("appr0001", "revoke");
    await user.click(screen.getByText(/Rejected and revoked/));
    await user.click(screen.getByRole("button", { name: "Restore" }));
    expect(core.memoryDecide).toHaveBeenLastCalledWith("revk0001", "restore");
    await waitFor(() => expect(core.memoryWaiting).toHaveBeenCalled());
  });

  it("writes, edits and shares, and shows the core's refusal where the user is looking", async () => {
    const user = await open();
    core.memoryWrite.mockResolvedValue(memory([kept]));
    await user.type(screen.getByRole("textbox", { name: "New memory entry" }), "Use bun.");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(core.memoryWrite).toHaveBeenCalledWith("p-app", "Use bun.");

    core.memoryEdit.mockResolvedValue(memory([kept]));
    const approved = screen.getByRole("region", { name: "Approved" });
    await user.click(within(approved).getByRole("button", { name: "Edit" }));
    const box = screen.getByRole("textbox", { name: "Edit memory entry" });
    await user.clear(box);
    await user.type(box, "The tests need TZ=UTC set.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(core.memoryEdit).toHaveBeenCalledWith("appr0001", "The tests need TZ=UTC set.");

    core.memoryShare.mockResolvedValue(memory([kept], true));
    await user.click(
      screen.getByRole("checkbox", { name: /Give this project.s agents its memory/ }),
    );
    expect(core.memoryShare).toHaveBeenCalledWith("p-app", true);
    await waitFor(() =>
      expect(
        screen.getByRole("checkbox", { name: /Give this project.s agents its memory/ }),
      ).toBeChecked(),
    );

    core.memoryWrite.mockRejectedValue({
      code: "memory_too_long",
      message: "A memory entry is one short paragraph: at most 500 characters, and this is 612.",
    });
    await user.type(screen.getByRole("textbox", { name: "New memory entry" }), "x");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("at most 500 characters");
  });

  it("marks a proposal Jev reads as a repeat or a contradiction, only with the switch on", async () => {
    core.memoryCheck.mockResolvedValue([{ id: "cand0001", repeats: false, contradicts: true }]);
    await open();
    expect(core.memoryCheck).not.toHaveBeenCalled();
    expect(screen.queryByText("may contradict an entry")).not.toBeInTheDocument();

    useAssistStore.setState({ status: status(true) });
    expect(await screen.findByText("may contradict an entry")).toBeInTheDocument();
    expect(core.memoryCheck).toHaveBeenCalledWith("p-app");
    expect(screen.queryByText("repeats an entry")).not.toBeInTheDocument();
  });
});
