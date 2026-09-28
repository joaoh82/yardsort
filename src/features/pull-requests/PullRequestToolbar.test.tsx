import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { PullRequest } from "@/lib/ipc";
import { worktree } from "@/test/fixtures";
const core = vi.hoisted(() => ({
  workspaceMergePullRequest: vi.fn(),
  projectPullRequests: vi.fn(),
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
import { pullRequestFor, usePublishStore } from "@/stores/publish";
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
      { name: "Unit tests", state: "passing" },
      { name: "Build", state: "passing" },
    ],
  },
};
const found = (requests = [pr]) => ({
  gh: true,
  pullRequests: requests,
  problem: null,
  loggedOut: false,
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
  expect(core.projectPullRequests).toHaveBeenCalledWith("alpha", true);
  expect(await screen.findByRole("status")).toHaveTextContent("Merge request sent");
});
it("offers copy and refresh, and reports merge errors", async () => {
  const user = setup();
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: "Copy PR link" }));
  await waitFor(() => expect(clipboard.writeText).toHaveBeenCalledWith(pr.url));
  await user.click(screen.getByRole("button", { name: /Actions for pull request/ }));
  await user.click(screen.getByRole("menuitem", { name: "Refresh pull request" }));
  await waitFor(() => expect(core.projectPullRequests).toHaveBeenCalledWith("alpha", true));
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
