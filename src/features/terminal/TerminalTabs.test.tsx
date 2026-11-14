import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  sessionSetLabel: vi.fn(),
  sessionsList: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => false,
  ipc: core,
}));

import { record } from "@/test/fixtures";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { TerminalTabs } from "./TerminalTabs";

const tab = (id: string, recordId: string | null): TerminalTab => ({
  id,
  workspaceId: "w1",
  title: recordId ? "Claude" : "shell",
  exit: null,
  recordId,
  busy: false,
  attention: false,
  toldAbout: null,
  toldAt: null,
  busySince: null,
});

describe("TerminalTabs", () => {
  beforeEach(() => {
    useTerminalStore.setState({ tabs: [tab("pty-1", "rec-1"), tab("pty-2", null)], active: {} });
    useProjectsStore.setState({ triggersOpen: false, triggersFocus: null });
    useSessionsStore.setState({ byWorkspace: { w1: [record("rec-1", { workspaceId: "w1" })] } });
    core.sessionSetLabel.mockReset();
    core.sessionsList.mockReset();
  });

  it("opens an agent tab's own triggers from its menu, and offers none on a shell's", async () => {
    const user = userEvent.setup();
    render(<TerminalTabs workspaceId="w1" />);
    fireEvent.contextMenu(screen.getByRole("tab", { name: /shell/ }));
    expect(screen.queryByRole("menuitem", { name: "Triggers…" })).toBeNull();
    await user.keyboard("{Escape}");

    fireEvent.contextMenu(screen.getByRole("tab", { name: /Claude/ }));
    await user.click(screen.getByRole("menuitem", { name: "Triggers…" }));
    expect(useProjectsStore.getState().triggersOpen).toBe(true);
    expect(useProjectsStore.getState().triggersFocus).toEqual({
      workspaceId: "w1",
      sessionId: "rec-1",
    });
  });

  it("labels an agent's tab from its menu, keeps it in the record, and shows the agent beside it", async () => {
    core.sessionSetLabel.mockResolvedValue("reviewer");
    core.sessionsList.mockResolvedValue([
      record("rec-1", { workspaceId: "w1", label: "reviewer" }),
    ]);
    const user = userEvent.setup();
    render(<TerminalTabs workspaceId="w1" />);
    fireEvent.contextMenu(screen.getByRole("tab", { name: /Claude/ }));
    await user.click(screen.getByRole("menuitem", { name: "Label…" }));
    const input = screen.getByRole("textbox", { name: "Label" });
    expect(input).toHaveFocus();
    await user.type(input, "  reviewer {Enter}");

    expect(core.sessionSetLabel).toHaveBeenCalledWith("rec-1", "reviewer");
    const labelled = await screen.findByRole("tab", { name: "reviewer (Claude Code)" });
    expect(labelled).toHaveTextContent("reviewer");
    expect(screen.queryByRole("textbox", { name: "Label" })).toBeNull();
  });

  it("starts labelling on a double-click, and Escape keeps what was there", async () => {
    const user = userEvent.setup();
    render(<TerminalTabs workspaceId="w1" />);
    await user.dblClick(screen.getByRole("tab", { name: /Claude/ }));
    await user.type(screen.getByRole("textbox", { name: "Label" }), "oops{Escape}");
    expect(core.sessionSetLabel).not.toHaveBeenCalled();
    expect(screen.getByRole("tab", { name: /Claude/ })).toBeInTheDocument();
  });

  it("clears a label when it is emptied, and offers no label on a shell", async () => {
    useSessionsStore.setState({
      byWorkspace: { w1: [record("rec-1", { workspaceId: "w1", label: "reviewer" })] },
    });
    core.sessionSetLabel.mockResolvedValue("");
    core.sessionsList.mockResolvedValue([record("rec-1", { workspaceId: "w1" })]);
    const user = userEvent.setup();
    render(<TerminalTabs workspaceId="w1" />);

    fireEvent.contextMenu(screen.getByRole("tab", { name: /shell/ }));
    expect(screen.queryByRole("menuitem", { name: /Label/ })).toBeNull();
    await user.keyboard("{Escape}");

    fireEvent.contextMenu(screen.getByRole("tab", { name: /reviewer/ }));
    await user.click(screen.getByRole("menuitem", { name: "Rename label…" }));
    await user.clear(screen.getByRole("textbox", { name: "Label" }));
    await user.keyboard("{Enter}");
    expect(core.sessionSetLabel).toHaveBeenCalledWith("rec-1", "");
    expect(await screen.findByRole("tab", { name: /Claude/ })).toBeInTheDocument();
  });
});
