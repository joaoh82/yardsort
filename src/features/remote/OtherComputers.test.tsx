import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OtherComputers, OtherHostView as View } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  otherComputers: vi.fn(),
  otherComputersSignIn: vi.fn(),
  otherComputersCancel: vi.fn(),
  otherComputersSignOut: vi.fn(),
  otherComputersPair: vi.fn(),
  otherComputersForget: vi.fn(),
  otherHost: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { OtherComputersSettings } from "@/features/settings/OtherComputersSettings";
import { OtherComputersList } from "@/features/sidebar/OtherComputersList";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";
import { OtherHostView } from "./OtherHostView";

const signedOut: OtherComputers = {
  client: null,
  signingIn: false,
  hosts: [],
  problem: null,
  signedOut: false,
  suggestedServerUrl: "https://remote.example.com",
  suggestedName: "Workstation",
};
const paired: OtherComputers = {
  ...signedOut,
  client: { serverUrl: "https://remote.example.com", name: "Workstation", since: 1 },
  hosts: [
    {
      grantId: "g-1",
      hostId: "h-1",
      name: "Build box",
      pairing: "paired",
      link: { kind: "online" },
    },
    {
      grantId: "g-2",
      hostId: "h-2",
      name: "Old laptop",
      pairing: "waiting",
      link: null,
    },
    {
      grantId: "g-3",
      hostId: "h-3",
      name: "Refused one",
      pairing: "refused",
      link: null,
    },
  ],
};
const shown: View = {
  hostId: "h-1",
  name: "Build box",
  link: { kind: "online" },
  readAt: 1_790_000_000_000,
  platform: "Linux",
  appVersion: "0.23.0",
  projects: [
    {
      id: "p-1",
      name: "service",
      workspaces: [
        {
          id: "w-1",
          name: "fix-login",
          branch: "ys/fix-login",
          archived: false,
          sessions: [
            {
              id: "s-1",
              title: "Fix the flaky login test",
              harness: "claude",
              state: "busy",
              startedAt: 1,
            },
          ],
          terminals: [{ id: "t-1", program: "sh", purpose: "agent", state: "quiet" }],
        },
      ],
    },
  ],
  activity: [
    {
      id: "e-1",
      kind: "turn.completed",
      occurredAt: 1_790_000_000_000,
      workspace: "fix-login",
    },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  useOtherComputersStore.setState({ status: null, views: {}, error: null });
  useProjectsStore.setState({ remoteHostId: null });
});

describe("the sidebar's other computers", () => {
  it("lists the hosts paired with or waiting, apart from the projects, and opens one", async () => {
    useOtherComputersStore.setState({ status: paired });
    render(<OtherComputersList />);
    const list = screen.getByRole("region", { name: "Other computers" });
    expect(within(list).getByRole("button", { name: /Build box/ })).toHaveTextContent("online");
    expect(within(list).getByRole("button", { name: /Old laptop/ })).toBeDisabled();
    // Refused: in Settings, not here.
    expect(within(list).queryByText("Refused one")).toBeNull();

    await userEvent.click(within(list).getByRole("button", { name: /Build box/ }));
    expect(useProjectsStore.getState().remoteHostId).toBe("h-1");
  });

  it("shows nothing until this computer is signed in and paired", () => {
    useOtherComputersStore.setState({ status: signedOut });
    const { container } = render(<OtherComputersList />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("another computer's page", () => {
  it("names the computer across the top, shows its work, and offers nothing that acts", async () => {
    core.otherHost.mockResolvedValue(shown);
    useOtherComputersStore.setState({ status: paired });
    render(<OtherHostView hostId="h-1" />);
    const banner = await screen.findByRole("banner", { name: "Another computer" });
    expect(banner).toHaveTextContent("Build box · another computer");
    expect(await screen.findByText("Fix the flaky login test")).toBeInTheDocument();
    expect(screen.getByText("ys/fix-login")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Online: Linux, Yardsort 0.23.0.");
    expect(within(screen.getByRole("region", { name: "Activity" })).getByText("turn.completed"));
    // Read-only: the only button closes the page.
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual(["Close"]);
  });

  it("says when what it shows is from before the computer went away", async () => {
    core.otherHost.mockResolvedValue({
      ...shown,
      link: { kind: "offline", since: 1_790_000_000_000, why: "it is not connected" },
    });
    render(<OtherHostView hostId="h-1" />);
    const line = await screen.findByRole("status");
    expect(line).toHaveTextContent(/^Offline since .*: it is not connected\. What it showed at /);
    expect(screen.getByText("Fix the flaky login test")).toBeInTheDocument();
  });
});

describe("Settings → Remote → Other computers", () => {
  it("offers the server this computer is a host on, and signs in as a client there", async () => {
    core.otherComputers.mockResolvedValue(signedOut);
    core.otherComputersSignIn.mockResolvedValue(paired);
    useOtherComputersStore.setState({ status: signedOut });
    render(<OtherComputersSettings />);
    expect(screen.getByRole("textbox", { name: /Server/ })).toHaveValue(
      "https://remote.example.com",
    );
    await userEvent.click(screen.getByRole("button", { name: "Sign in to reach other computers" }));
    expect(core.otherComputersSignIn).toHaveBeenCalledWith(
      "https://remote.example.com",
      "Workstation",
    );
    expect(await screen.findByText(/Signed in as/)).toHaveTextContent(
      "Signed in as Workstation at https://remote.example.com.",
    );
  });

  it("pairs by the other computer's code, and says what the server said when it is wrong", async () => {
    core.otherComputersPair.mockRejectedValueOnce({
      code: "remote_code",
      message: "No computer is showing that code. Check it, or show a new one there.",
    });
    core.otherComputers.mockResolvedValue(paired);
    useOtherComputersStore.setState({ status: paired });
    render(<OtherComputersSettings />);
    await userEvent.type(screen.getByRole("textbox", { name: /Pairing code/ }), "abcd-efgh");
    await userEvent.click(screen.getByRole("button", { name: "Pair" }));
    expect(core.otherComputersPair).toHaveBeenCalledWith("abcd-efgh");
    expect(await screen.findByRole("alert")).toHaveTextContent("No computer is showing that code.");

    core.otherComputersPair.mockResolvedValueOnce(paired);
    await userEvent.click(screen.getByRole("button", { name: "Pair" }));
    expect(screen.getByRole("textbox", { name: /Pairing code/ })).toHaveValue("");
  });

  it("lists every pairing, refused ones too, each of which can be forgotten", async () => {
    core.otherComputers.mockResolvedValue(paired);
    core.otherComputersForget.mockResolvedValue({ ...paired, hosts: paired.hosts.slice(1) });
    useOtherComputersStore.setState({ status: paired });
    render(<OtherComputersSettings />);
    const list = screen.getByRole("list");
    expect(within(list).getByText("waiting for its answer")).toBeInTheDocument();
    expect(within(list).getByText("refused")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Forget Build box" }));
    expect(core.otherComputersForget).toHaveBeenCalledWith("g-1");
    expect(screen.queryByText("Build box")).toBeNull();
  });
});
