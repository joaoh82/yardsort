import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { CoordinatorConfirmation } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  coordinatorConfirmations: vi.fn(),
  coordinatorAnswer: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { FloorManagerConfirmations } from "./Confirmations";

const request = (id: string, command: string): CoordinatorConfirmation => ({
  id,
  command,
  createdUnixMs: 1,
});

let waiting: CoordinatorConfirmation[] = [];

beforeEach(() => {
  vi.clearAllMocks();
  waiting = [];
  core.coordinatorConfirmations.mockImplementation(async () => waiting);
  core.coordinatorAnswer.mockImplementation(async (id: string) => {
    const found = waiting.some((item) => item.id === id);
    waiting = waiting.filter((item) => item.id !== id);
    return found;
  });
});

it("stays out of the way until a command is waiting", async () => {
  render(<FloorManagerConfirmations />);
  await waitFor(() => expect(core.coordinatorConfirmations).toHaveBeenCalled());
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("asks, and Allow is the only thing that lets the command run", async () => {
  waiting = [
    request("one", "ys workspace new demo fix the login"),
    request("two", "ys test run login"),
  ];
  const user = userEvent.setup();
  render(<FloorManagerConfirmations />);
  expect(await screen.findByRole("dialog", { name: /wants to run this/ })).toBeInTheDocument();
  expect(screen.getByText("ys workspace new demo fix the login")).toBeInTheDocument();
  expect(screen.getByText("Another is waiting after this one.")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Allow" }));
  expect(core.coordinatorAnswer).toHaveBeenCalledWith("one", true);
  expect(await screen.findByText("ys test run login")).toBeInTheDocument();
  expect(screen.queryByText("ys workspace new demo fix the login")).not.toBeInTheDocument();
});

it("Don't allow, and Escape, leave the command unrun", async () => {
  waiting = [request("one", "ys workspace new demo fix the login")];
  const user = userEvent.setup();
  const { unmount } = render(<FloorManagerConfirmations />);
  expect(await screen.findByRole("dialog")).toBeInTheDocument();

  await user.keyboard("{Escape}");
  expect(core.coordinatorAnswer).toHaveBeenCalledWith("one", false);
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

  waiting = [request("two", "ys workspace new demo fix")];
  unmount();
  render(<FloorManagerConfirmations />);
  await user.click(await screen.findByRole("button", { name: "Don't allow" }));
  expect(core.coordinatorAnswer).toHaveBeenLastCalledWith("two", false);
});

it("says when the command is no longer waiting", async () => {
  waiting = [request("one", "ys workspace new demo fix the login")];
  core.coordinatorAnswer.mockImplementation(async (id: string) => {
    waiting = waiting.filter((item) => item.id !== id);
    return false;
  });
  const user = userEvent.setup();
  render(<FloorManagerConfirmations />);
  await user.click(await screen.findByRole("button", { name: "Allow" }));
  const notice = await screen.findByRole("alertdialog", { name: /no longer waiting/i });
  expect(notice).toHaveAttribute("aria-modal", "true");
  expect(screen.getByText("It was not run.")).toBeInTheDocument();

  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
});

it("does not cover a newer command when one is no longer waiting", async () => {
  waiting = [
    request("one", "ys workspace new demo fix the login"),
    request("two", "ys test run login"),
  ];
  core.coordinatorAnswer.mockImplementation(async (id: string) => {
    waiting = waiting.filter((item) => item.id !== id);
    return false;
  });
  const user = userEvent.setup();
  render(<FloorManagerConfirmations />);
  await user.click(await screen.findByRole("button", { name: "Allow" }));
  expect(await screen.findByText("ys test run login")).toBeInTheDocument();
  expect(screen.queryByRole("alertdialog", { name: /no longer waiting/i })).not.toBeInTheDocument();
  expect(screen.getByRole("dialog", { name: /wants to run this/ })).toBeInTheDocument();
});
