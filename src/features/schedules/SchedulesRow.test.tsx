import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
}));

import { useProjectsStore } from "@/stores/projects";
import { useSchedulesStore } from "@/stores/schedules";
import { schedule } from "@/test/fixtures";
import { SchedulesRow } from "./SchedulesRow";

describe("SchedulesRow", () => {
  beforeEach(() => {
    useProjectsStore.setState({ schedulesOpen: false });
    useSchedulesStore.setState({ items: [] });
  });

  it("shows no badge when every schedule can fire", () => {
    useSchedulesStore.setState({ items: [schedule("s1"), schedule("s2", { standing: "off" })] });
    render(<SchedulesRow />);
    expect(screen.getByRole("button", { name: "Schedules" })).toBeInTheDocument();
  });

  it("counts the schedules that need the user", () => {
    useSchedulesStore.setState({
      items: [
        schedule("s1", { standing: "awaiting_approval" }),
        schedule("s2", { standing: "refused" }),
        schedule("s3"),
      ],
    });
    render(<SchedulesRow />);
    expect(
      screen.getByRole("button", { name: "Schedules, 2 need your attention" }),
    ).toBeInTheDocument();
  });

  it("opens the Schedules view when clicked, and closes it when clicked again", async () => {
    const user = userEvent.setup();
    render(<SchedulesRow />);
    await user.click(screen.getByRole("button", { name: "Schedules" }));
    expect(useProjectsStore.getState().schedulesOpen).toBe(true);
    await user.click(screen.getByRole("button", { name: "Schedules" }));
    expect(useProjectsStore.getState().schedulesOpen).toBe(false);
  });
});
