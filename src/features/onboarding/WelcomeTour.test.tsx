import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { useLayoutStore } from "@/stores/layout";
import { usePreferencesStore } from "@/stores/preferences";
import { WelcomeTour } from "./WelcomeTour";
const core = vi.hoisted(() => ({ uiStateSave: vi.fn(), uiStateLoad: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
beforeEach(() => {
  core.uiStateSave.mockReset().mockResolvedValue(undefined);
  usePreferencesStore.setState({ loaded: false, welcomeSeen: false, error: null, saving: false });
  useLayoutStore.setState({ tourOpen: false, collapsed: { left: true, right: true } });
});
it("waits for persisted preferences, offers a choice and remembers declining across reloads", async () => {
  const user = userEvent.setup();
  render(<WelcomeTour />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  core.uiStateLoad.mockResolvedValue({});
  await act(() => usePreferencesStore.getState().load());
  expect(screen.getByRole("heading", { name: "Take a quick look around?" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Not now" }));
  expect(core.uiStateSave).toHaveBeenCalledWith("onboarding.welcomeSeen", "true");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  core.uiStateLoad.mockResolvedValue({ "onboarding.welcomeSeen": "true" });
  await act(() => usePreferencesStore.getState().load());
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("tours an empty app using the keyboard, traps focus, restores panels and can replay", async () => {
  usePreferencesStore.setState({ loaded: true });
  const user = userEvent.setup();
  render(
    <>
      <button>Outside</button>
      <WelcomeTour />
    </>,
  );
  await user.click(screen.getByRole("button", { name: "Take the tour" }));
  expect(screen.getByRole("heading", { name: "Your projects live here" })).toBeInTheDocument();
  screen.getByRole("button", { name: "Next" }).focus();
  await user.tab();
  expect(screen.getByRole("button", { name: "Skip tour" })).toHaveFocus();
  await user.tab({ shift: true });
  expect(screen.getByRole("button", { name: "Next" })).toHaveFocus();
  await user.keyboard("{ArrowRight}{ArrowRight}");
  expect(screen.getByRole("heading", { name: "Review the work" })).toBeInTheDocument();
  expect(useLayoutStore.getState().collapsed).toEqual({ left: false, right: false });
  await user.keyboard("{ArrowLeft}");
  expect(screen.getByRole("heading", { name: "Give an agent a task" })).toBeInTheDocument();
  await user.keyboard("{ArrowRight}{ArrowRight}{ArrowRight}");
  await user.click(screen.getByRole("button", { name: "Finish" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(useLayoutStore.getState().collapsed).toEqual({ left: true, right: true });
  act(() => useLayoutStore.getState().setTourOpen(true));
  expect(screen.getByRole("heading", { name: "Your projects live here" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("shows persistence failures and lets the user retry", async () => {
  usePreferencesStore.setState({ loaded: true });
  core.uiStateSave.mockRejectedValueOnce(new Error("Disk full"));
  render(<WelcomeTour />);
  fireEvent.click(screen.getByRole("button", { name: "Not now" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
  expect(usePreferencesStore.getState().welcomeSeen).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Not now" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});
