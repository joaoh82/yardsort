import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MachineReport, SettingsInfo, UsageReport } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  usageTokens: vi.fn(),
  usageMachine: vi.fn(),
  settingsSaveUsage: vi.fn(),
  uiStateSave: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { runCommand } from "@/features/keyboard/commands";
import { useAppStore } from "@/stores/app";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { UsageView } from "./UsageView";

const day = (date: string, cost: number[], tokens: number[]) => ({ date, cost, tokens });

const tokens = (overrides: Partial<UsageReport> = {}): UsageReport => ({
  days: 30,
  from: "2026-09-02",
  to: "2026-10-01",
  cost: 125.5,
  unpricedModels: ["grok-4.7-build"],
  totals: {
    processed: 6_800_000_000,
    cacheRead: 6_500_000_000,
    cacheWrite: 200_000_000,
    uncachedInput: 81_400_000,
    output: 17_800_000,
    cacheSavings: 3864.7,
  },
  agents: [
    { agent: "claude", tokens: 5e9, cost: 100, location: "~/.claude", files: 12 },
    { agent: "codex", tokens: 1.7e9, cost: 25.5, location: "~/.codex", files: 3 },
    { agent: "grok", tokens: 1e8, cost: 0, location: "~/.grok", files: 1 },
  ],
  daily: [
    day("2026-09-30", [60, 20, 0], [3e9, 1e9, 0]),
    day("2026-10-01", [40, 5.5, 0], [2e9, 7e8, 1e8]),
  ],
  models: [
    { model: "claude-opus-5-5", agent: "claude", tokens: 5e9, knownCost: 100 },
    { model: "gpt-6-sol", agent: "codex", tokens: 1.7e9, knownCost: 25.5 },
    { model: "grok-4.7-build", agent: "grok", tokens: 1e8, knownCost: null },
  ],
  places: [
    {
      label: "fix-login",
      project: "app",
      workspaceId: "w-fix",
      folder: null,
      tokens: 4e9,
      cost: 90,
    },
    {
      label: "scripts",
      project: null,
      workspaceId: null,
      folder: "~/scripts",
      tokens: 1e9,
      cost: 10,
    },
  ],
  limits: [
    {
      agent: "codex",
      plan: "pro",
      observedAt: Date.now() - 60_000,
      windows: [
        { minutes: 10080, usedPercent: 14, resetsAt: Date.now() + 2 * 86_400_000 + 3_600_000 },
      ],
    },
  ],
  ...overrides,
});

const load = (cpu: number, memory: number, processes = 1) => ({ cpu, memory, processes });
const MB = 1024 ** 2;

const machine = (): MachineReport => ({
  sampledAt: Date.now(),
  yardsort: load(6.6, 1500 * MB, 9),
  system: {
    totalMemory: 32 * 1024 * MB,
    usedMemory: 16 * 1024 * MB,
    availableMemory: 16 * 1024 * MB,
    cpuCount: 10,
    cpu: 20,
    loadOne: 1.25,
  },
  app: [
    { part: "main", load: load(0.5, 200 * MB) },
    { part: "webview", load: load(0.7, 300 * MB, 3) },
    { part: "host", load: load(0.1, 20 * MB) },
  ],
  projects: [
    {
      projectId: "p-app",
      name: "app",
      load: load(5.3, 980 * MB, 4),
      workspaces: [
        {
          workspaceId: "w-fix",
          name: "fix-login",
          load: load(5.3, 980 * MB, 4),
          terminals: [
            {
              sessionId: "s1",
              label: "Fix the login form",
              harness: "claude",
              run: false,
              load: load(5.0, 900 * MB, 3),
            },
            { sessionId: "s2", label: null, harness: null, run: false, load: load(0.3, 80 * MB) },
          ],
        },
      ],
    },
  ],
  loose: [],
  history: [
    { at: Date.now() - 4000, cpu: 5, memory: 1400 * MB },
    { at: Date.now(), cpu: 6.6, memory: 1500 * MB },
  ],
});

describe("Usage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.usageTokens.mockResolvedValue(tokens());
    core.usageMachine.mockResolvedValue(machine());
    core.uiStateSave.mockResolvedValue(undefined);
    useAppStore.setState({ showUsageInSidebar: true });
    useHarnessStore.setState({ harnesses: [] });
    useProjectsStore.setState({ usageOpen: true, selectedWorkspaceId: null, workflowId: null });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows what the agents spent, priced, by agent, model and workspace", async () => {
    const user = userEvent.setup();
    render(<UsageView />);

    expect(await screen.findByText("$126*")).toBeInTheDocument();
    expect(core.usageTokens).toHaveBeenCalledWith(30);
    // A subscription is not billed per token, and that is said beside the figure.
    expect(screen.getByText(/if billed per token at API rates/)).toHaveTextContent(
      "Not priced: grok-4.7-build.",
    );

    const agents = screen.getByRole("list", { name: "By agent" });
    expect(within(agents).getByText("Claude Code")).toBeInTheDocument();
    expect(within(agents).getByText("80%")).toBeInTheDocument();

    const models = screen.getByRole("table", { name: "By model" });
    const grok = within(models).getByText("grok-4.7-build").closest("tr")!;
    expect(within(grok).getByTitle("No known price for this model")).toHaveTextContent("—");

    expect(screen.getByText("Cache savings").nextSibling).toHaveTextContent("$3,865");
    // A folder Yardsort does not know is listed by name, with where it is.
    expect(screen.getByText("scripts")).toHaveAttribute("title", "~/scripts");

    // Codex's plan limit, as it last heard it.
    const limits = screen.getByRole("region", { name: "Plan limits" });
    expect(within(limits).getByRole("heading", { name: "Plan limits" })).toBeInTheDocument();
    expect(within(limits).getByRole("meter", { name: "Weekly limit used" })).toHaveAttribute(
      "aria-valuenow",
      "14",
    );
    expect(within(limits).getByText(/2d 1h/)).toBeInTheDocument();

    // A Yardsort workspace opens from its row.
    await user.click(screen.getByRole("button", { name: "app · fix-login" }));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-fix");
    expect(useProjectsStore.getState().usageOpen).toBe(false);
  });

  it("asks again for another range, and shows tokens instead of cost", async () => {
    const user = userEvent.setup();
    render(<UsageView />);
    await screen.findByText("$126*");

    core.usageTokens.mockResolvedValue(tokens({ days: 7, cost: 12 }));
    await user.click(screen.getByRole("button", { name: "7d" }));
    expect(core.usageTokens).toHaveBeenLastCalledWith(7);
    expect(await screen.findByText("$12.00*")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Tokens" }));
    expect(screen.getByText("tokens processed").previousSibling).toHaveTextContent("6.8B");
  });

  it("measures every workspace in the unit the tab shows, priced or not", async () => {
    const user = userEvent.setup();
    core.usageTokens.mockResolvedValue(
      tokens({
        places: [
          {
            label: "app",
            project: "app",
            workspaceId: "w-app",
            folder: null,
            tokens: 1e6,
            cost: 50,
          },
          // Grok is unpriced: no cost, many tokens.
          {
            label: "tool",
            project: null,
            workspaceId: null,
            folder: "~/tool",
            tokens: 2e6,
            cost: 0,
          },
        ],
      }),
    );
    render(<UsageView />);
    const places = await screen.findByRole("region", { name: "By workspace" });
    const bar = (label: string) =>
      within(places).getByText(label).closest("li")!.querySelector<HTMLElement>("[style]")!;

    expect(bar("app · app").style.width).toBe("100%");
    expect(bar("tool").style.width).toBe("0%");

    await user.click(screen.getByRole("button", { name: "Tokens" }));
    expect(bar("tool").style.width).toBe("100%");
    expect(bar("app · app").style.width).toBe("50%");
    expect(within(places).getAllByRole("listitem")[0]).toHaveTextContent("tool");
  });

  it("keeps the range last chosen when an earlier one answers after it", async () => {
    const user = userEvent.setup();
    render(<UsageView />);
    await screen.findByText("$126*");

    let finish90: (report: UsageReport) => void = () => {};
    core.usageTokens.mockImplementation((days: number) =>
      days === 90
        ? new Promise<UsageReport>((resolve) => (finish90 = resolve))
        : Promise.resolve(tokens({ days, cost: 7 })),
    );
    await user.click(screen.getByRole("button", { name: "90d" }));
    await user.click(screen.getByRole("button", { name: "7d" }));
    expect(await screen.findByText("$7.00*")).toBeInTheDocument();

    await act(async () => finish90(tokens({ days: 90, cost: 900 })));
    expect(screen.getByText("$7.00*")).toBeInTheDocument();
    expect(screen.queryByText("$900*")).not.toBeInTheDocument();
    expect(screen.getByText("From session logs on this machine")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Read the logs again" })).toBeEnabled();
  });

  it("puts the sidebar switch back and says so when it cannot be saved", async () => {
    const user = userEvent.setup();
    core.settingsSaveUsage.mockRejectedValue(new Error("Could not save settings: read-only"));
    render(<UsageView />);
    const box = screen.getByRole("checkbox", { name: "Show Usage in the sidebar" });
    await user.click(box);

    expect(await screen.findByRole("alert")).toHaveTextContent("read-only");
    expect(box).toBeChecked();
    expect(useAppStore.getState().showUsageInSidebar).toBe(true);
  });

  it("says where it looked when there are no logs at all", async () => {
    core.usageTokens.mockResolvedValue(
      tokens({ agents: [], daily: [], models: [], places: [], limits: [], cost: 0 }),
    );
    render(<UsageView />);
    expect(await screen.findByText(/No session logs found/)).toHaveTextContent("~/.codex");
  });

  it("breaks the machine down by project, workspace and terminal, and samples every two seconds", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<UsageView />);
    await user.click(screen.getByRole("tab", { name: "Machine resources" }));

    expect(await screen.findByText("Yardsort memory")).toBeInTheDocument();
    expect(screen.getByText("Yardsort memory").nextSibling).toHaveTextContent("1.46 GB");
    expect(screen.getByText("Load (1 m)").nextSibling).toHaveTextContent("1.25");
    const table = screen.getByRole("table");
    expect(within(table).getByText("Terminal host")).toBeInTheDocument();
    expect(within(table).getByText("Fix the login form")).toBeInTheDocument();
    // A plain shell is named for what it is.
    expect(within(table).getByText("Shell")).toBeInTheDocument();

    await user.click(within(table).getByRole("button", { name: "Collapse fix-login" }));
    expect(within(table).queryByText("Fix the login form")).not.toBeInTheDocument();
    await user.click(within(table).getByRole("button", { name: "Collapse app" }));
    expect(within(table).queryByText("fix-login")).not.toBeInTheDocument();

    const before = core.usageMachine.mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    await waitFor(() => expect(core.usageMachine.mock.calls.length).toBe(before + 1));
  });

  it("stops sampling once it is closed", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const { unmount } = render(<UsageView />);
    await user.click(screen.getByRole("tab", { name: "Machine resources" }));
    await screen.findByText("Yardsort memory");
    unmount();
    const before = core.usageMachine.mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(core.usageMachine.mock.calls.length).toBe(before);
  });

  it("can take itself off the sidebar, and the palette still opens it", async () => {
    const user = userEvent.setup();
    core.settingsSaveUsage.mockResolvedValue({ showUsageInSidebar: false } as SettingsInfo);
    render(<UsageView />);
    await user.click(screen.getByRole("checkbox", { name: "Show Usage in the sidebar" }));
    expect(core.settingsSaveUsage).toHaveBeenCalledWith(false);
    await waitFor(() => expect(useAppStore.getState().showUsageInSidebar).toBe(false));

    await user.click(screen.getByRole("button", { name: "Close usage" }));
    expect(useProjectsStore.getState().usageOpen).toBe(false);
    runCommand("usage");
    expect(useProjectsStore.getState().usageOpen).toBe(true);
  });
});
