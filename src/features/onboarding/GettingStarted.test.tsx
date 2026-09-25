import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Preflight, YsStatus } from "@/lib/ipc";
import { project } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  preflight: vi.fn(),
  ysInstall: vi.fn(),
  harnessesList: vi.fn(),
  projectsList: vi.fn(),
  uiStateLoad: vi.fn(),
}));
const clipboard = vi.hoisted(() => ({ writeText: vi.fn() }));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);
vi.mock("@tauri-apps/plugin-opener", () => opener);

import { usePreflightStore } from "@/stores/preflight";
import { useProjectsStore } from "@/stores/projects";
import { GettingStarted } from "./GettingStarted";

const agent = (id: string, label: string, path: string | null, enabled = true) => ({
  id,
  label,
  command: id,
  enabled,
  path,
  install: { command: `npm install -g ${id}-package`, url: `https://example.com/${id}` },
});

/** An AppImage whose `ys` has not been installed yet. */
const ysMissing: YsStatus = {
  version: "0.10.0",
  method: "copy",
  bundled: "/tmp/.mount_abc/usr/bin/ys",
  target: "/home/you/.local/bin/ys",
  installed: false,
  targetOnPath: true,
  found: null,
  foundVersion: null,
  foundIsOurs: false,
};
const ysInstalled: YsStatus = {
  ...ysMissing,
  installed: true,
  found: "/home/you/.local/bin/ys",
  foundVersion: "0.10.0",
  foundIsOurs: true,
};

function report(overrides: Partial<Preflight> = {}): Preflight {
  return {
    git: { path: "/usr/bin/git", version: "git version 2.55.0" },
    harnesses: [agent("claude", "Claude Code", "/usr/bin/claude"), agent("codex", "Codex", null)],
    env: { source: "loginShell", shell: "/bin/zsh", pathEntries: 12, warning: null },
    os: "linux",
    ys: ysInstalled,
    ready: true,
    ...overrides,
  };
}

const bare = () =>
  report({
    git: { path: null, version: null },
    harnesses: [agent("claude", "Claude Code", null), agent("codex", "Codex", null)],
    ready: false,
  });

async function show(first: Preflight, projects = 0) {
  core.preflight.mockResolvedValue(first);
  useProjectsStore.setState({
    projects: Array.from({ length: projects }, (_, i) => project(`p${i}`)),
  });
  render(<GettingStarted />);
  await vi.waitFor(() => expect(core.preflight).toHaveBeenCalled());
  return userEvent.setup();
}

describe("GettingStarted", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clipboard.writeText.mockResolvedValue(undefined);
    opener.openUrl.mockResolvedValue(undefined);
    core.harnessesList.mockResolvedValue([]);
    core.projectsList.mockResolvedValue([]);
    core.uiStateLoad.mockResolvedValue({});
    usePreflightStore.setState({ report: null, checking: false, error: null });
  });

  it("on a bare machine says what is missing and how to get it", async () => {
    await show(bare());
    const list = await screen.findByRole("region", { name: "Getting started" });

    expect(list).toHaveTextContent("git is not installed");
    expect(list).toHaveTextContent("sudo pacman -S git");
    expect(list).toHaveTextContent("No coding agent found");
    expect(list).toHaveTextContent("npm install -g claude-package");
    expect(list).toHaveTextContent("npm install -g codex-package");
    // Nothing to open until the machine is ready.
    expect(screen.queryByRole("button", { name: /Open a folder/ })).not.toBeInTheDocument();
  });

  it("gives advice that fits the operating system", async () => {
    await show({ ...bare(), os: "macos" });
    expect(await screen.findByText("xcode-select --install")).toBeInTheDocument();
    expect(screen.queryByText(/pacman/)).not.toBeInTheDocument();
  });

  it("copies a command, and opens links in the browser", async () => {
    const user = await show(bare());
    await user.click(await screen.findByRole("button", { name: /^Copy: sudo pacman/ }));
    expect(clipboard.writeText).toHaveBeenCalledWith("sudo pacman -S git");

    await user.click(screen.getAllByRole("button", { name: /Install instructions/ })[0]!);
    expect(opener.openUrl).toHaveBeenCalledWith("https://example.com/claude");
  });

  it("checks again without a restart, re-reading the environment, and moves on when fixed", async () => {
    const user = await show(bare());
    core.preflight.mockResolvedValue(report());

    await user.click(await screen.findByRole("button", { name: "Check again" }));

    expect(core.preflight).toHaveBeenLastCalledWith(true);
    expect(await screen.findByText("git is installed")).toBeInTheDocument();
    expect(screen.getByText("1 coding agent found")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
    // git appeared, so the project list — which needs git — is loaded again.
    expect(core.projectsList).toHaveBeenCalled();
    expect(core.harnessesList).toHaveBeenCalled();
  });

  it("on a ready machine with no projects, points at the one thing left to do", async () => {
    await show(report());
    const list = await screen.findByRole("region", { name: "Getting started" });
    expect(within(list).getByText("git version 2.55.0")).toBeInTheDocument();
    expect(within(list).getByText("Claude Code")).toBeInTheDocument();
    expect(within(list).getByRole("button", { name: /Open a folder/ })).toBeInTheDocument();
  });

  it("stays out of the way once everything works and there are projects", async () => {
    await show(report(), 2);
    expect(await screen.findByText(/Pick a workspace on the left/)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Getting started" })).not.toBeInTheDocument();
  });

  it("comes back if something breaks later, even with projects", async () => {
    await show(bare(), 2);
    expect(await screen.findByText("git is not installed")).toBeInTheDocument();
  });

  it("does not count a disabled agent, and explains a shell environment that could not be read", async () => {
    await show(
      report({
        harnesses: [agent("claude", "Claude Code", "/usr/bin/claude", false)],
        env: {
          source: "process",
          shell: "/bin/zsh",
          pathEntries: 3,
          warning: "/bin/zsh did not finish within 8s",
        },
        ready: false,
      }),
    );
    expect(await screen.findByText("No coding agent found")).toBeInTheDocument();
    expect(screen.getByText("Your shell environment could not be read")).toBeInTheDocument();
    expect(screen.getByText("/bin/zsh did not finish within 8s")).toBeInTheDocument();
  });
});

describe("status bar environment indicator", () => {
  it("re-reads the environment from anywhere in the app", async () => {
    const { StatusBar } = await import("@/features/shell/StatusBar");
    const { useAppStore } = await import("@/stores/app");
    vi.clearAllMocks();
    core.harnessesList.mockResolvedValue([]);
    usePreflightStore.setState({ report: null, checking: false, error: null });
    useAppStore.setState({
      env: { source: "loginShell", shell: "/bin/zsh", pathEntries: 12, warning: null },
    });
    core.preflight.mockResolvedValue(
      report({ env: { source: "loginShell", shell: "/bin/zsh", pathEntries: 14, warning: null } }),
    );
    render(<StatusBar />);

    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: /env: login shell · 12 PATH/ }));

    expect(core.preflight).toHaveBeenCalledWith(true);
    expect(await screen.findByRole("button", { name: /14 PATH/ })).toBeInTheDocument();
  });

  describe("the ys command", () => {
    it("is installed from here, and then says so", async () => {
      core.ysInstall.mockResolvedValue(ysInstalled);
      const user = await show(report({ ys: ysMissing }));
      const list = await screen.findByRole("region", { name: "Getting started" });
      expect(list).toHaveTextContent("The ys command is not installed");
      expect(list).toHaveTextContent("/home/you/.local/bin/ys");

      await user.click(screen.getByRole("button", { name: "Install ys" }));

      expect(core.ysInstall).toHaveBeenCalledWith(false);
      expect(await screen.findByText("The ys command is installed")).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Install ys" })).not.toBeInTheDocument();
    });

    it("offers an update when the one on PATH is Yardsort's but older", async () => {
      core.ysInstall.mockResolvedValue(ysInstalled);
      const user = await show(report({ ys: { ...ysInstalled, foundVersion: "0.9.0" } }));
      expect(await screen.findByText("The ys command is out of date")).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Update to 0.10.0" }));
      expect(core.ysInstall).toHaveBeenCalledWith(false);
      expect(await screen.findByText("The ys command is installed")).toBeInTheDocument();
    });

    it("replaces a file that is not ys only after a second yes", async () => {
      const inTheWay = {
        code: "ys_exists",
        message: "There is already a file at /home/you/.local/bin/ys and it is not Yardsort's ys.",
      };
      core.ysInstall
        .mockRejectedValueOnce(inTheWay)
        .mockRejectedValueOnce(inTheWay)
        .mockResolvedValueOnce(ysInstalled);
      const user = await show(report({ ys: { ...ysMissing, installed: true } }));

      await user.click(await screen.findByRole("button", { name: "Install ys" }));
      const ask = await screen.findByRole("alertdialog", { name: "Replace the file?" });
      expect(ask).toHaveTextContent("it is not Yardsort's ys. Replace it?");

      await user.click(within(ask).getByRole("button", { name: "Cancel" }));
      expect(core.ysInstall).toHaveBeenCalledTimes(1);
      expect(core.ysInstall).not.toHaveBeenCalledWith(true);
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Install ys" }));
      await user.click(await screen.findByRole("button", { name: "Replace it" }));
      expect(core.ysInstall).toHaveBeenLastCalledWith(true);
      expect(await screen.findByText("The ys command is installed")).toBeInTheDocument();
    });

    it("is optional: a machine without it is still ready", async () => {
      await show(report({ ys: ysMissing }));
      const list = await screen.findByRole("region", { name: "Getting started" });
      expect(list).toHaveTextContent("optional");
      expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
    });
  });
});
