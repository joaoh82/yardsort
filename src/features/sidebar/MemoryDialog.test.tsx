import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MemoryCheck, MemoryEntry, ProjectMemory } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  memoryGet: vi.fn(),
  memoryWrite: vi.fn(),
  memoryEdit: vi.fn(),
  memoryDecide: vi.fn(),
  memorySettle: vi.fn(),
  memoryShare: vi.fn(),
  conversationsShare: vi.fn(),
  memoryWaiting: vi.fn(),
  memoryCheck: vi.fn(),
  assistStatus: vi.fn(),
  onSyncChanged: vi.fn(),
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
  host: null,
  conflict: null,
  createdAt: 0,
  updatedAt: 0,
  history: [],
  ...extra,
});
const memory = (
  entries: MemoryEntry[],
  shared = false,
  conversationsShared = true,
): ProjectMemory => ({
  projectId: "p-app",
  shared,
  conversationsShared,
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
const torn = entry("conf0001", "Deploy after noon.", {
  conflict: { text: "Deploy before noon.", from: "Build box", at: 0 },
});
const synced = entry("sync0001", "Never edit bindings.ts.", {
  from: "the user, on Build box",
  host: "Build box",
});

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
    core.onSyncChanged.mockResolvedValue(() => {});
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

    // Sharing conversations is its own switch, on until it is turned off.
    const conversations = screen.getByRole("checkbox", {
      name: /Share conversations between this project.s agents/,
    });
    expect(conversations).toBeChecked();
    core.conversationsShare.mockResolvedValue(memory([kept], true, false));
    await user.click(conversations);
    expect(core.conversationsShare).toHaveBeenCalledWith("p-app", false);
    await waitFor(() => expect(conversations).not.toBeChecked());
    expect(
      screen.getByRole("checkbox", { name: /Give this project.s agents its memory/ }),
    ).toBeChecked();

    core.memoryWrite.mockRejectedValue({
      code: "memory_too_long",
      message: "A memory entry is one short paragraph: at most 500 characters, and this is 612.",
    });
    await user.type(screen.getByRole("textbox", { name: "New memory entry" }), "x");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("at most 500 characters");
  });

  it("shows both texts of a conflict apart from the approved ones, and settles it as chosen", async () => {
    core.memoryGet.mockResolvedValue(memory([kept, torn, synced]));
    const user = await open();
    const conflicts = screen.getByRole("region", { name: "Conflicts" });
    expect(conflicts).toHaveTextContent("Stands now · agents get thisDeploy after noon.");
    expect(conflicts).toHaveTextContent("From Build boxDeploy before noon.");
    expect(screen.getByRole("region", { name: "Approved" })).not.toHaveTextContent(
      "Deploy after noon.",
    );

    core.memorySettle.mockResolvedValue(memory([kept, synced]));
    await user.click(within(conflicts).getByRole("button", { name: "Keep this one" }));
    expect(core.memorySettle).toHaveBeenLastCalledWith("conf0001", { keep: "standing" });
    await waitFor(() => expect(screen.queryByRole("region", { name: "Conflicts" })).toBeNull());
  });

  it("keeps the other text, writes a third or revokes it, and the change elsewhere shows", async () => {
    let changed = () => {};
    core.onSyncChanged.mockImplementation(async (handler: () => void) => {
      changed = handler;
      return () => {};
    });
    core.memoryGet.mockResolvedValue(memory([kept, torn, synced]));
    const user = await open();
    // A note from another computer says which.
    expect(screen.getByRole("region", { name: "Approved" })).toHaveTextContent(
      "from the user, on Build box",
    );
    core.memorySettle.mockResolvedValue(memory([kept, torn, synced]));
    const conflicts = () => screen.getByRole("region", { name: "Conflicts" });
    await user.click(within(conflicts()).getByRole("button", { name: "Keep Build box’s" }));
    expect(core.memorySettle).toHaveBeenLastCalledWith("conf0001", { keep: "other" });

    await user.click(within(conflicts()).getByRole("button", { name: "Write another" }));
    const box = screen.getByRole("textbox", { name: "Write another text" });
    await user.clear(box);
    await user.type(box, "Deploy at noon.");
    await user.click(within(conflicts()).getByRole("button", { name: "Save" }));
    expect(core.memorySettle).toHaveBeenLastCalledWith("conf0001", {
      keep: "text",
      text: "Deploy at noon.",
    });

    core.memoryDecide.mockResolvedValue(memory([kept, torn, synced]));
    await user.click(within(conflicts()).getByRole("button", { name: "Revoke" }));
    expect(core.memoryDecide).toHaveBeenLastCalledWith("conf0001", "revoke");

    // Settled on the other computer: a pass says so, and the view reads it again.
    core.memoryGet.mockResolvedValue(memory([kept, synced]));
    changed();
    await waitFor(() => expect(screen.queryByRole("region", { name: "Conflicts" })).toBeNull());
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

  it("asks again when the approved entries change, and drops the old tags meanwhile", async () => {
    core.memoryCheck.mockResolvedValue([{ id: "cand0001", repeats: false, contradicts: true }]);
    core.assistStatus.mockResolvedValue(status(true));
    useAssistStore.setState({ status: status(true) });
    const user = await open();
    expect(await screen.findByText("may contradict an entry")).toBeInTheDocument();
    expect(core.memoryCheck).toHaveBeenCalledTimes(1);

    // Revoking the only approved entry leaves nothing to contradict.
    let answer: (value: MemoryCheck[]) => void = () => {};
    core.memoryCheck.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    core.memoryDecide.mockResolvedValue(memory([{ ...kept, state: "revoked" }, proposal]));
    const approved = screen.getByRole("region", { name: "Approved" });
    await user.click(within(approved).getByRole("button", { name: "Revoke" }));
    await waitFor(() => expect(core.memoryCheck).toHaveBeenCalledTimes(2));
    expect(screen.queryByText("may contradict an entry")).not.toBeInTheDocument();
    answer([{ id: "cand0001", repeats: false, contradicts: false }]);
    await waitFor(() =>
      expect(screen.queryByText("may contradict an entry")).not.toBeInTheDocument(),
    );
  });
});
