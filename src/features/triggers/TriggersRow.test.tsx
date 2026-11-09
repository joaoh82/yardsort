import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
}));

import { useProjectsStore } from "@/stores/projects";
import { useTriggersStore } from "@/stores/triggers";
import { trigger } from "@/test/fixtures";
import { TriggersRow } from "./TriggersRow";

describe("TriggersRow", () => {
  beforeEach(() => {
    useProjectsStore.setState({ triggersOpen: false });
    useTriggersStore.setState({ items: [] });
  });

  it("shows no badge when every trigger can fire", () => {
    useTriggersStore.setState({ items: [trigger("s1"), trigger("s2", { standing: "off" })] });
    render(<TriggersRow />);
    expect(screen.getByRole("button", { name: "Triggers" })).toBeInTheDocument();
  });

  it("counts the triggers that need the user", () => {
    useTriggersStore.setState({
      items: [
        trigger("s1", { standing: "awaiting_approval" }),
        trigger("s2", { standing: "capped" }),
        trigger("s3"),
      ],
    });
    render(<TriggersRow />);
    expect(
      screen.getByRole("button", { name: "Triggers, 2 need your attention" }),
    ).toBeInTheDocument();
  });

  it("opens the Triggers view when clicked, and closes it when clicked again", async () => {
    const user = userEvent.setup();
    render(<TriggersRow />);
    await user.click(screen.getByRole("button", { name: "Triggers" }));
    expect(useProjectsStore.getState().triggersOpen).toBe(true);
    await user.click(screen.getByRole("button", { name: "Triggers" }));
    expect(useProjectsStore.getState().triggersOpen).toBe(false);
  });
});
