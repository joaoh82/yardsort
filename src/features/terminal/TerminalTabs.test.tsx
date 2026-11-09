import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => false,
}));

import { useProjectsStore } from "@/stores/projects";
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
});
