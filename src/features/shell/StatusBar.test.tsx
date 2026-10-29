import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import { useLayoutStore } from "@/stores/layout";
import { useRemoteStore } from "@/stores/remote";
import { StatusBar } from "./StatusBar";

const phone = {
  grantId: "g-1",
  deviceName: "Ana's phone",
  wantsControl: true,
  expiresInMs: 60_000,
};

beforeEach(() => {
  useRemoteStore.getState().settled([]);
  useLayoutStore.setState({ settingsOpen: false, settingsSection: "harnesses" });
});

it("says a device asks to pair, and opens Settings → Remote to answer", async () => {
  render(<StatusBar />);
  expect(screen.queryByRole("button", { name: /asks? to pair/ })).toBeNull();

  useRemoteStore.getState().settled([phone]);
  await userEvent.click(await screen.findByRole("button", { name: "Ana's phone asks to pair" }));
  expect(useLayoutStore.getState()).toMatchObject({
    settingsOpen: true,
    settingsSection: "remote",
  });

  useRemoteStore.getState().settled([phone, { ...phone, grantId: "g-2", deviceName: "Tablet" }]);
  expect(await screen.findByRole("button", { name: "2 devices ask to pair" })).toBeInTheDocument();
});
