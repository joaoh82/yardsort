import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useNotificationsStore } from "@/stores/notifications";
import { useProjectsStore } from "@/stores/projects";
import { notification } from "@/test/fixtures";
import { NotificationsRow } from "./NotificationsRow";

const finished = (id: string) => notification(id, { kind: "agent.finished", part: "finished" });

describe("NotificationsRow", () => {
  beforeEach(() => {
    useProjectsStore.setState({ notificationsOpen: false });
    useNotificationsStore.setState({ items: [] });
  });

  it("shows no number when nothing waits, whatever is under Earlier", () => {
    useNotificationsStore.setState({
      items: [notification("old", { part: "earlier", settledAt: 1 })],
    });
    render(<NotificationsRow />);
    expect(screen.getByRole("button", { name: "Notifications" })).toBeInTheDocument();
  });

  it("counts what needs you and what finished unseen apart", () => {
    useNotificationsStore.setState({
      items: [notification("a"), finished("b"), finished("c")],
    });
    render(<NotificationsRow />);
    const row = screen.getByRole("button", {
      name: "Notifications, 1 needs you, 2 finished and not seen",
    });
    expect(row).toHaveTextContent("Notifications12");
  });

  it("opens the Notifications view when clicked, and closes it when clicked again", async () => {
    const user = userEvent.setup();
    useProjectsStore.setState({ tasksOpen: true });
    render(<NotificationsRow />);
    await user.click(screen.getByRole("button", { name: "Notifications" }));
    expect(useProjectsStore.getState().notificationsOpen).toBe(true);
    expect(useProjectsStore.getState().tasksOpen).toBe(false);
    await user.click(screen.getByRole("button", { name: "Notifications" }));
    expect(useProjectsStore.getState().notificationsOpen).toBe(false);
  });
});
