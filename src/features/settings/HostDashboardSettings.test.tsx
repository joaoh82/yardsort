import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { DashboardStatus } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  dashboardStatus: vi.fn(),
  dashboardSetEnabled: vi.fn(),
  dashboardOpen: vi.fn(),
  dashboardSignInLink: vi.fn(),
  dashboardSignEveryoneOut: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
const clipboard = vi.hoisted(() => ({ writeText: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);

import { HostDashboardSettings } from "./HostDashboardSettings";

const off: DashboardStatus = {
  enabled: false,
  address: null,
  tunnel: null,
  signedIn: 0,
  problem: null,
};
const on: DashboardStatus = { ...off, enabled: true, address: "http://127.0.0.1:28680" };

beforeEach(() => {
  vi.clearAllMocks();
  core.dashboardStatus.mockResolvedValue(off);
  clipboard.writeText.mockResolvedValue(undefined);
});

it("is off until turned on, and offers no way in while it is off", async () => {
  render(<HostDashboardSettings />);
  const toggle = await screen.findByRole("checkbox", { name: /Serve the host dashboard/ });
  expect(toggle).not.toBeChecked();
  expect(screen.queryByRole("button", { name: "Open the dashboard" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Copy a sign-in link" })).toBeNull();
  expect(screen.queryByText(/Served at/)).toBeNull();
});

it("turning it on serves it and says where", async () => {
  core.dashboardSetEnabled.mockResolvedValue(on);
  render(<HostDashboardSettings />);
  await userEvent.click(await screen.findByRole("checkbox", { name: /Serve the host dashboard/ }));
  expect(core.dashboardSetEnabled).toHaveBeenCalledWith(true);
  expect(await screen.findByText("http://127.0.0.1:28680")).toBeInTheDocument();
  expect(screen.getByText(/No browser is signed in\./)).toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: /Serve the host dashboard/ })).toBeChecked();
  // Nobody is signed in, so there is nobody to sign out.
  expect(screen.getByRole("button", { name: "Sign every browser out" })).toBeDisabled();
});

it("turning it off stops serving it", async () => {
  core.dashboardStatus.mockResolvedValue(on);
  core.dashboardSetEnabled.mockResolvedValue(off);
  render(<HostDashboardSettings />);
  await userEvent.click(await screen.findByRole("checkbox", { name: /Serve the host dashboard/ }));
  expect(core.dashboardSetEnabled).toHaveBeenCalledWith(false);
  await waitFor(() => expect(screen.queryByText(/Served at/)).toBeNull());
});

it("opens the dashboard in the browser, signed in, from the core", async () => {
  core.dashboardStatus.mockResolvedValue(on);
  core.dashboardOpen.mockResolvedValue(undefined);
  render(<HostDashboardSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Open the dashboard" }));
  expect(core.dashboardOpen).toHaveBeenCalledTimes(1);
  // The link is the core's to open: it is never in the page.
  expect(core.dashboardSignInLink).not.toHaveBeenCalled();
});

it("copies a sign-in link and says it reaches this computer alone", async () => {
  core.dashboardStatus.mockResolvedValue(on);
  core.dashboardSignInLink.mockResolvedValue({
    link: "http://127.0.0.1:28680/#code=AAAA-BBBB-CCCC-DDDD",
    code: "AAAA-BBBB-CCCC-DDDD",
    goodForMinutes: 10,
  });
  render(<HostDashboardSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Copy a sign-in link" }));
  await waitFor(() =>
    expect(clipboard.writeText).toHaveBeenCalledWith(
      "http://127.0.0.1:28680/#code=AAAA-BBBB-CCCC-DDDD",
    ),
  );
  const said = await screen.findByRole("status");
  expect(said).toHaveTextContent("once, within 10 minutes");
  expect(said).toHaveTextContent("forward the port over SSH first");
  // The code is on the clipboard, not on the screen.
  expect(screen.queryByText(/AAAA-BBBB/)).toBeNull();
});

it("a link through the tunnel needs no forwarded port", async () => {
  core.dashboardStatus.mockResolvedValue({
    ...on,
    tunnel: "up at https://desk.example.test",
    signedIn: 2,
  });
  core.dashboardSignInLink.mockResolvedValue({
    link: "https://desk.example.test/#code=AAAA-BBBB-CCCC-DDDD",
    code: "AAAA-BBBB-CCCC-DDDD",
    goodForMinutes: 10,
  });
  render(<HostDashboardSettings />);
  expect(await screen.findByText(/2 browsers are signed in\./)).toHaveTextContent(
    "Its tunnel is up at https://desk.example.test.",
  );
  await userEvent.click(screen.getByRole("button", { name: "Copy a sign-in link" }));
  const said = await screen.findByRole("status");
  expect(said).not.toHaveTextContent("forward the port");
});

it("signs every browser out", async () => {
  core.dashboardStatus.mockResolvedValue({ ...on, signedIn: 1 });
  core.dashboardSignEveryoneOut.mockResolvedValue(on);
  render(<HostDashboardSettings />);
  expect(await screen.findByText(/One browser is signed in\./)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Sign every browser out" }));
  expect(core.dashboardSignEveryoneOut).toHaveBeenCalledTimes(1);
  expect(await screen.findByText(/No browser is signed in\./)).toBeInTheDocument();
});

it("says why it is turned on and not served", async () => {
  core.dashboardStatus.mockResolvedValue({
    ...off,
    enabled: true,
    problem: "None of the ports 28680 to 28689 on 127.0.0.1 could be listened on.",
  });
  render(<HostDashboardSettings />);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "turned on and not served: None of the ports 28680 to 28689",
  );
  expect(screen.queryByRole("button", { name: "Open the dashboard" })).toBeNull();
});

it("says what the core refused", async () => {
  core.dashboardStatus.mockResolvedValue(on);
  core.dashboardOpen.mockRejectedValue({ code: "dashboard_browser", message: "No browser." });
  render(<HostDashboardSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Open the dashboard" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("No browser.");
});
