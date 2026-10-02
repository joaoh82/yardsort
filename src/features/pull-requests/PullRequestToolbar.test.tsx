import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { PullRequest } from "@/lib/ipc";
import { worktree } from "@/test/fixtures";
const core = vi.hoisted(() => ({
  workspaceMergePullRequest: vi.fn(),
  projectPullRequests: vi.fn(),
  workspaceConflictHelper: vi.fn(),
  workspaceResolveConflicts: vi.fn(),
}));
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
const clipboard = vi.hoisted(() => ({ writeText: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@/lib/native", () => ({ native }));
vi.mock("@tauri-apps/plugin-opener", () => opener);
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);
import { pullRequestFor, pullRequestsFor, usePublishStore } from "@/stores/publish";
import { useTerminalStore } from "@/stores/terminals";
import { PullRequestToolbar } from "./PullRequestToolbar";

const pr: PullRequest = {
  number: 85,
  url: "https://github.com/demo/app/pull/85",
  title: "Fix billing quota",
  branch: "ys/feature",
  state: "open",
  draft: false,
  checks: "passing",
  details: {
    base: "main",
    headOid: "abc123",
    additions: 69,
    deletions: 5,
    review: "APPROVED",
    updatedAt: "2026-09-28T10:00:00Z",
    checks: [
      { name: "Unit tests", state: "passing", workflow: "CI", url: null },
      { name: "Build", state: "passing", workflow: "CI", url: null },
    ],
    mergeable: "mergeable",
    checkCounts: { passed: 2, failed: 0, running: 0 },
    reviewRequests: [],
    reviews: [],
    crossRepository: false,
  },
  author: "ada",
  createdAt: null,
};
const found = (requests = [pr], workspaces: Record<string, number[]> = {}) => ({
  gh: true,
  pullRequests: requests,
  problem: null,
  loggedOut: false,
  workspaces,
  repo: null,
  viewer: null,
  openTotal: null,
  openProblem: null,
});
beforeEach(() => {
  vi.resetAllMocks();
  usePublishStore.setState({ workspaceId: null, state: null, byProject: { alpha: found() } });
  core.projectPullRequests.mockResolvedValue(found());
  core.workspaceMergePullRequest.mockResolvedValue(undefined);
  opener.openUrl.mockResolvedValue(undefined);
  clipboard.writeText.mockResolvedValue(undefined);
});
function setup() {
  render(<PullRequestToolbar workspace={worktree("alpha", "feature")} projectId="alpha" />);
  return userEvent.setup();
}
it("shows shared PR details on focus, expands checks, and dismisses with Escape", async () => {
  const user = setup();
  await user.tab();
  const card = await screen.findByRole("region", { name: "Pull request #85 details" });
  expect(within(card).getByText("Fix billing quota")).toBeVisible();
  expect(within(card).getByText("Approved")).toBeVisible();
  expect(within(card).getByText("ys/feature → main")).toBeVisible();
  await user.keyboard("{ArrowDown}");
  expect(within(card).getByText("Show checks")).toHaveFocus();
  await user.click(within(card).getByText("Show checks"));
  expect(within(card).getByText("Unit tests")).toBeVisible();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("region")).not.toBeInTheDocument();
});
it("opens the hover preview and permits moving the pointer into it", async () => {
  const user = setup();
  await user.hover(screen.getByRole("button", { name: /Open Pull request/ }));
  const card = await screen.findByRole("region");
  await user.hover(card);
  await new Promise((resolve) => setTimeout(resolve, 220));
  expect(card).toBeVisible();
  await user.click(within(card).getByRole("button", { name: /View on GitHub/ }));
  expect(opener.openUrl).toHaveBeenCalledWith(pr.url);
});
it("does not merge when confirmation is cancelled", async () => {
  const user = setup();
  native.confirm.mockResolvedValue(false);
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: "Squash and merge" }));
  await waitFor(() => expect(native.confirm).toHaveBeenCalled());
  expect(core.workspaceMergePullRequest).not.toHaveBeenCalled();
});
it.each([
  ["Squash and merge", "squash"],
  ["Create a merge commit", "merge"],
  ["Rebase and merge", "rebase"],
])("confirms %s with the displayed PR and head, then refreshes", async (label, method) => {
  const user = setup();
  native.confirm.mockResolvedValue(true);
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: label }));
  await waitFor(() =>
    expect(core.workspaceMergePullRequest).toHaveBeenCalledWith(
      "w-alpha-feature",
      85,
      "abc123",
      method,
    ),
  );
  expect(core.projectPullRequests).toHaveBeenCalledWith("alpha", true, false);
  expect(await screen.findByRole("status")).toHaveTextContent("Merge request sent");
});
it("offers copy and refresh, and reports merge errors", async () => {
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: "Copy PR link" }));
  await waitFor(() => expect(clipboard.writeText).toHaveBeenCalledWith(pr.url));
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: "Refresh pull request" }));
  await waitFor(() => expect(core.projectPullRequests).toHaveBeenCalledWith("alpha", true, false));
  native.confirm.mockResolvedValue(true);
  core.workspaceMergePullRequest.mockRejectedValue(new Error("Branch protection requires review"));
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: "Squash and merge" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Branch protection requires review");
});
it("disables merging drafts and closed PRs", async () => {
  const user = setup();
  act(() => usePublishStore.setState({ byProject: { alpha: found([{ ...pr, draft: true }]) } }));
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  expect(screen.getByRole("menuitem", { name: "Squash and merge" })).toBeDisabled();
  fireEvent.keyDown(window, { key: "Escape" });
  act(() =>
    usePublishStore.setState({ byProject: { alpha: found([{ ...pr, state: "merged" }]) } }),
  );
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  expect(screen.getByRole("menuitem", { name: "Rebase and merge" })).toBeDisabled();
});
it("selects an open PR on a reused branch independently of response order", () => {
  const merged = { ...pr, number: 99, state: "merged" as const };
  expect(pullRequestFor(found([merged, pr]), pr.branch)?.number).toBe(85);
  expect(pullRequestFor(found([pr, merged]), pr.branch)?.number).toBe(85);
  expect(pullRequestFor(found([{ ...merged, number: 10 }, merged]), pr.branch)?.number).toBe(99);
});
it("does not replace a new PR with a slower, older project response", async () => {
  let resolveOld!: (value: ReturnType<typeof found>) => void;
  core.projectPullRequests.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        resolveOld = resolve;
      }),
  );
  const old = usePublishStore.getState().loadProject("alpha", true);
  await usePublishStore.getState().loadProject("alpha", true);
  resolveOld(found([{ ...pr, number: 42, state: "merged" }]));
  await old;
  expect(usePublishStore.getState().byProject.alpha?.pullRequests[0]?.number).toBe(85);
});

/** A second pull request the workspace opened, from another branch its agent checked out. */
const second: PullRequest = {
  ...pr,
  number: 90,
  title: "Split out the quota migration",
  branch: "ys/quota-migration",
  details: { ...pr.details!, headOid: "def456" },
};

it("lists every pull request the workspace opened and acts on the one chosen", async () => {
  usePublishStore.setState({
    byProject: { alpha: found([second, pr], { "w-alpha-feature": [90, 85] }) },
  });
  const user = setup();
  // The checked-out branch's own comes first; the other is counted on the menu button.
  expect(screen.getByRole("button", { name: /Open Pull request #85/ })).toBeInTheDocument();
  await user.click(
    screen.getByRole("button", {
      name: "Actions for pull request #85 — 1 more from this workspace",
    }),
  );
  expect(
    screen.getByRole("menuitemradio", { name: /#85 open — Fix billing quota/ }),
  ).toHaveAttribute("aria-checked", "true");
  await user.click(screen.getByRole("menuitemradio", { name: /#90 open — Split out/ }));
  expect(screen.getByRole("button", { name: /Open Pull request #90/ })).toBeInTheDocument();

  native.confirm.mockResolvedValue(true);
  await user.click(screen.getByRole("button", { name: /Actions for pull request #90/ }));
  await user.click(screen.getByRole("menuitem", { name: "Squash and merge" }));
  await waitFor(() =>
    expect(core.workspaceMergePullRequest).toHaveBeenCalledWith(
      "w-alpha-feature",
      90,
      "def456",
      "squash",
    ),
  );
});

it("puts the branch's own pull request first, then the core's matches newest first", () => {
  const workspace = worktree("alpha", "feature");
  const third = { ...second, number: 70, state: "merged" as const };
  const all = found([second, pr, third], { "w-alpha-feature": [90, 85, 70] });
  expect(pullRequestsFor(all, workspace).map((p) => p.number)).toEqual([85, 90, 70]);
  // Not matched by the core, not on the branch: not this workspace's.
  expect(pullRequestsFor(found([second, pr]), workspace).map((p) => p.number)).toEqual([85]);
  // Detached: only what the core matched.
  expect(
    pullRequestsFor(all, {
      ...workspace,
      head: { label: "abc", detached: true, unborn: false },
    }).map((p) => p.number),
  ).toEqual([90, 85, 70]);
});

const conflicted: PullRequest = {
  ...pr,
  details: { ...pr.details!, mergeable: "conflicting" },
};

it("offers to resolve conflicts only when GitHub reports them", async () => {
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  expect(screen.queryByRole("menuitem", { name: /resolve conflicts/ })).not.toBeInTheDocument();
  fireEvent.keyDown(window, { key: "Escape" });
  act(() => usePublishStore.setState({ byProject: { alpha: found([conflicted]) } }));
  expect(screen.getByRole("button", { name: /merge conflicts/ })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  expect(screen.getByRole("menuitem", { name: /resolve conflicts/ })).toBeEnabled();
});

it("names the agent before asking it, and a no sends nothing", async () => {
  usePublishStore.setState({ byProject: { alpha: found([conflicted]) } });
  core.workspaceConflictHelper.mockResolvedValue({
    sessionId: "r1",
    harnessLabel: "Claude Code",
    title: "Fix the billing quota",
    reach: "resume",
  });
  native.confirm.mockResolvedValue(false);
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: /resolve conflicts/ }));
  await waitFor(() => expect(native.confirm).toHaveBeenCalled());
  const [message, options] = native.confirm.mock.calls[0]!;
  expect(message).toContain("Ask Claude Code to resolve the conflicts in pull request #85");
  expect(message).toContain("“Fix the billing quota” has ended. It is resumed in a new tab");
  expect(message).toContain("merge main into ys/feature");
  expect(options).toMatchObject({ okLabel: "Ask Claude Code" });
  expect(core.workspaceResolveConflicts).not.toHaveBeenCalled();
});

it("asks the agent and shows the tab it is in", async () => {
  usePublishStore.setState({ byProject: { alpha: found([conflicted]) } });
  useTerminalStore.setState({ tabs: [], active: {} });
  core.workspaceConflictHelper.mockResolvedValue({
    sessionId: "r1",
    harnessLabel: "Claude Code",
    title: "Fix the billing quota",
    reach: "resume",
  });
  core.workspaceResolveConflicts.mockResolvedValue({
    reach: "resume",
    session: {
      id: "pty-7",
      program: "claude",
      args: [],
      cwd: null,
      pid: 1,
      size: { cols: 80, rows: 24 },
      labels: { workspace: "w-alpha-feature", harness: "claude", record: "r1" },
      state: { status: "running" },
      hasOutput: false,
      busy: false,
      idleMs: 0,
    },
  });
  native.confirm.mockResolvedValue(true);
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: /resolve conflicts/ }));
  await waitFor(() =>
    expect(core.workspaceResolveConflicts).toHaveBeenCalledWith("w-alpha-feature", 85, "r1", {
      cols: 80,
      rows: 24,
    }),
  );
  expect(await screen.findByRole("status")).toHaveTextContent(
    "Asked Claude Code to resolve the conflicts in #85.",
  );
  expect(useTerminalStore.getState().active["w-alpha-feature"]).toBe("pty-7");
  expect(core.projectPullRequests).toHaveBeenCalledWith("alpha", true, false);
});

it("says why when the agent cannot be asked", async () => {
  usePublishStore.setState({ byProject: { alpha: found([conflicted]) } });
  core.workspaceConflictHelper.mockRejectedValue(
    new Error("Claude Code is working right now. Ask again once it is quiet."),
  );
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: /resolve conflicts/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Claude Code is working right now");
  expect(native.confirm).not.toHaveBeenCalled();
});

it("warns that a half-typed message goes with a request typed into a running agent", async () => {
  usePublishStore.setState({ byProject: { alpha: found([conflicted]) } });
  core.workspaceConflictHelper.mockResolvedValue({
    sessionId: "r1",
    harnessLabel: "Claude Code",
    title: "Fix the billing quota",
    reach: "type",
  });
  native.confirm.mockResolvedValue(false);
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: /resolve conflicts/ }));
  await waitFor(() => expect(native.confirm).toHaveBeenCalled());
  expect(native.confirm.mock.calls[0]![0]).toContain(
    "Anything you had typed there and not sent goes with it.",
  );
});
