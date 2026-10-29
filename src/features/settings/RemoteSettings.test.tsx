import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { RemoteLink, RemoteStatus } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  remoteStatus: vi.fn(),
  remoteEnable: vi.fn(),
  remoteCancel: vi.fn(),
  remoteDisable: vi.fn(),
  remotePair: vi.fn(),
  remoteRequests: vi.fn(),
  remoteAnswer: vi.fn(),
  remoteRevoke: vi.fn(),
  onRemotePairingRequested: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { useProjectsStore } from "@/stores/projects";
import { useRemoteStore } from "@/stores/remote";
import { RemoteSettings } from "./RemoteSettings";

const off: RemoteStatus = {
  available: true,
  host: null,
  devices: [],
  signingIn: false,
  credentials: "in this computer's credential store",
  lastServerUrl: null,
  lastName: null,
};
const on: RemoteStatus = {
  ...off,
  host: {
    serverUrl: "https://remote.example.com",
    name: "Workstation",
    since: 1,
    link: { kind: "connected", devices: 0 },
  },
};
const phone = {
  grantId: "g-1",
  deviceName: "Ana's phone",
  wantsControl: true,
  expiresInMs: 240_000,
};

beforeEach(() => {
  vi.clearAllMocks();
  core.remoteStatus.mockResolvedValue(off);
  core.remoteRequests.mockResolvedValue({ requests: [], status: on });
  core.onRemotePairingRequested.mockResolvedValue(() => {});
  useRemoteStore.setState({ asking: [] });
  useProjectsStore.setState({
    projects: [
      { id: "p-1", name: "api", rootPath: "/repos/api" },
      { id: "p-2", name: "web", rootPath: "/repos/web" },
    ] as never,
  });
});

it("turns on with the server and name given, then shows this computer", async () => {
  core.remoteEnable.mockResolvedValue(on);
  render(<RemoteSettings />);
  const button = await screen.findByRole("button", { name: "Turn on and sign in" });
  expect(button).toBeDisabled();
  await userEvent.type(
    screen.getByRole("textbox", { name: "Server" }),
    "https://remote.example.com",
  );
  await userEvent.type(screen.getByRole("textbox", { name: "Name this computer" }), "Workstation");
  await userEvent.click(button);
  expect(core.remoteEnable).toHaveBeenCalledWith("https://remote.example.com", "Workstation");
  expect(await screen.findByText("Workstation")).toBeInTheDocument();
  expect(screen.getByText("https://remote.example.com")).toBeInTheDocument();
  expect(screen.getByText(/kept in this computer's credential store/)).toBeInTheDocument();
});

it("offers the server and name it was last turned on with", async () => {
  core.remoteStatus.mockResolvedValue({
    ...off,
    lastServerUrl: "https://remote.example.com",
    lastName: "Workstation",
  });
  core.remoteEnable.mockResolvedValue(on);
  render(<RemoteSettings />);
  const button = await screen.findByRole("button", { name: "Turn on and sign in" });
  expect(screen.getByRole("textbox", { name: "Server" })).toHaveValue("https://remote.example.com");
  expect(screen.getByRole("textbox", { name: "Name this computer" })).toHaveValue("Workstation");
  await userEvent.click(button);
  expect(core.remoteEnable).toHaveBeenCalledWith("https://remote.example.com", "Workstation");
});

it("says why turning on failed", async () => {
  core.remoteEnable.mockRejectedValue({
    code: "remote_address",
    message: "A server's address starts with https://",
  });
  render(<RemoteSettings />);
  await userEvent.type(await screen.findByRole("textbox", { name: "Server" }), "http://x");
  await userEvent.type(screen.getByRole("textbox", { name: "Name this computer" }), "W");
  await userEvent.click(screen.getByRole("button", { name: "Turn on and sign in" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("starts with https://");
});

it("asks who is waiting as soon as a device asks, and keeps the window's list to it", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    core.remoteStatus.mockResolvedValue(on);
    let told: () => void = () => {};
    core.onRemotePairingRequested.mockImplementation((handler: () => void) => {
      told = handler;
      return Promise.resolve(() => {});
    });
    useRemoteStore.setState({ asking: [{ ...phone, grantId: "g-answered" }] });
    render(<RemoteSettings />);
    await waitFor(() => expect(core.remoteRequests).toHaveBeenCalledTimes(1));
    // What the server says is waiting stands: the request answered elsewhere goes.
    await waitFor(() => expect(useRemoteStore.getState().asking).toEqual([]));

    core.remoteRequests.mockResolvedValue({ requests: [phone], status: on });
    told();
    // Well before the next look.
    expect(await screen.findByText("Ana's phone")).toBeInTheDocument();
    expect(core.remoteRequests).toHaveBeenCalledTimes(2);
    expect(useRemoteStore.getState().asking).toEqual([phone]);
  } finally {
    vi.useRealTimers();
  }
});

it("takes a failed poll's error down once the server answers again", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    core.remoteStatus.mockResolvedValue(on);
    core.remoteRequests
      .mockRejectedValueOnce({ code: "remote_unreachable", message: "Could not reach the server" })
      .mockResolvedValue({ requests: [], status: on });
    render(<RemoteSettings />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not reach the server");
    await vi.advanceTimersByTimeAsync(3000);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  } finally {
    vi.useRealTimers();
  }
});

it("shows a code to pair with until it is done", async () => {
  core.remoteStatus.mockResolvedValue(on);
  core.remotePair.mockResolvedValue({
    code: "7KQ2-M9XD",
    qrSvg: "<svg xmlns='http://www.w3.org/2000/svg'></svg>",
    expiresInMs: 300_000,
  });
  render(<RemoteSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Pair a device" }));
  const pairing = await screen.findByRole("region", { name: "Pair a device" });
  expect(within(pairing).getByLabelText("Pairing code")).toHaveTextContent("7KQ2-M9XD");
  expect(within(pairing).getByRole("img", { name: "QR code for pairing" })).toHaveAttribute(
    "src",
    expect.stringMatching(/^data:image\/svg\+xml;utf8,/),
  );
  expect(within(pairing).getByText(/Good once, for 5:00 more/)).toBeInTheDocument();
  await userEvent.click(within(pairing).getByRole("button", { name: "Done" }));
  expect(screen.queryByRole("region", { name: "Pair a device" })).toBeNull();
});

it("lets a waiting device watch, on all projects", async () => {
  core.remoteStatus.mockResolvedValue(on);
  core.remoteRequests.mockResolvedValue({ requests: [phone], status: on });
  core.remoteAnswer.mockResolvedValue({
    ...on,
    devices: [
      { grantId: "g-1", name: "Ana's phone", control: false, projects: null, approvedAt: 2 },
    ],
  });
  render(<RemoteSettings />);
  const asking = await screen.findByRole("group", { name: "Ana's phone asks to be let in" });
  expect(asking).toHaveTextContent("watch and type into");
  await userEvent.click(within(asking).getByRole("button", { name: "Let it watch" }));
  expect(core.remoteAnswer).toHaveBeenCalledWith("g-1", {
    kind: "allow",
    control: false,
    projects: null,
  });
  const devices = await screen.findByRole("region", { name: "Devices let in" });
  expect(within(devices).getByText("watches, all projects")).toBeInTheDocument();
});

it("lets a device watch and type on the projects chosen, and only once one is", async () => {
  core.remoteStatus.mockResolvedValue(on);
  core.remoteRequests.mockResolvedValue({ requests: [phone], status: on });
  core.remoteAnswer.mockResolvedValue(on);
  render(<RemoteSettings />);
  const asking = await screen.findByRole("group", { name: "Ana's phone asks to be let in" });
  await userEvent.click(within(asking).getByRole("radio", { name: "Only these:" }));
  const typing = within(asking).getByRole("button", { name: "Let it watch and type" });
  expect(typing).toBeDisabled();
  await userEvent.click(within(asking).getByRole("checkbox", { name: "web" }));
  await userEvent.click(typing);
  expect(core.remoteAnswer).toHaveBeenCalledWith("g-1", {
    kind: "allow",
    control: true,
    projects: ["p-2"],
  });
});

it("offers no typing to a device that asked only to watch, and can refuse", async () => {
  core.remoteStatus.mockResolvedValue(on);
  core.remoteRequests.mockResolvedValue({
    requests: [{ ...phone, wantsControl: false }],
    status: on,
  });
  core.remoteAnswer.mockResolvedValue(on);
  render(<RemoteSettings />);
  const asking = await screen.findByRole("group", { name: "Ana's phone asks to be let in" });
  expect(within(asking).queryByRole("button", { name: "Let it watch and type" })).toBeNull();
  await userEvent.click(within(asking).getByRole("button", { name: "Refuse" }));
  expect(core.remoteAnswer).toHaveBeenCalledWith("g-1", { kind: "deny" });
  await waitFor(() =>
    expect(screen.queryByRole("group", { name: "Ana's phone asks to be let in" })).toBeNull(),
  );
});

it("revokes a device let in, and turns off", async () => {
  const withPhone = {
    ...on,
    devices: [
      { grantId: "g-1", name: "Ana's phone", control: true, projects: ["p-1"], approvedAt: 2 },
    ],
  };
  core.remoteStatus.mockResolvedValue(withPhone);
  core.remoteRequests.mockResolvedValue({ requests: [], status: withPhone });
  core.remoteRevoke.mockResolvedValue(on);
  core.remoteDisable.mockResolvedValue({
    ...off,
    lastServerUrl: "https://remote.example.com",
    lastName: "Workstation",
  });
  render(<RemoteSettings />);
  expect(await screen.findByText("watches and types, 1 project")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Revoke Ana's phone" }));
  expect(core.remoteRevoke).toHaveBeenCalledWith("g-1");
  await userEvent.click(screen.getByRole("button", { name: "Turn off" }));
  expect(core.remoteDisable).toHaveBeenCalled();
  expect(await screen.findByRole("button", { name: "Turn on and sign in" })).toBeEnabled();
  expect(screen.getByRole("textbox", { name: "Server" })).toHaveValue("https://remote.example.com");
  expect(screen.getByRole("textbox", { name: "Name this computer" })).toHaveValue("Workstation");
});

it("says where the link to the relay stands, as the poll brings it", async () => {
  const linked = (link: RemoteLink) => ({ ...on, host: { ...on.host!, link } });
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    core.remoteStatus.mockResolvedValue(linked({ kind: "connecting" }));
    core.remoteRequests.mockResolvedValue({
      requests: [],
      status: linked({ kind: "waiting", why: "Could not reach the server.", retryInMs: 4000 }),
    });
    render(<RemoteSettings />);
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Not connected to the relay: Could not reach the server. Trying again in 4 s.",
    );

    core.remoteRequests.mockResolvedValue({
      requests: [],
      status: linked({ kind: "connected", devices: 2 }),
    });
    await vi.advanceTimersByTimeAsync(3000);
    expect(
      await screen.findByText("Connected to the relay. 2 devices are connected now."),
    ).toBeInTheDocument();

    core.remoteRequests.mockResolvedValue({
      requests: [],
      status: linked({ kind: "stopped", why: "This computer was signed out at the server." }),
    });
    await vi.advanceTimersByTimeAsync(3000);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Not connected to the relay: This computer was signed out at the server. Turn Remote off, then on again.",
    );
  } finally {
    vi.useRealTimers();
  }
});
