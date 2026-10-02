import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectPullRequests } from "@/lib/ipc";
import {
  project,
  pullRequest,
  pullRequestsOf,
  pullRequestSummary,
  worktree,
} from "@/test/fixtures";

const core = vi.hoisted(() => ({
  uiStateSave: vi.fn(),
  projectPullRequests: vi.fn(),
  pullRequestSummary: vi.fn(),
  pullRequestMerge: vi.fn(),
  pullRequestClose: vi.fn(),
  pullRequestReopen: vi.fn(),
  pullRequestPrepareBranch: vi.fn(),
  sessionsList: vi.fn(),
  ptySpawn: vi.fn(),
}));
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
const clipboard = vi.hoisted(() => ({ writeText: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
vi.mock("@/lib/native", () => ({ native }));
vi.mock("@tauri-apps/plugin-opener", () => opener);
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);

import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { usePullRequestsStore } from "@/stores/pullRequests";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";
import { FILTERS_KEY } from "./filters";
import { PullRequestsView } from "./PullRequestsView";

const base = project("alpha");
const feature = worktree("alpha", "feature");
const alpha = { ...base, workspaces: [...base.workspaces, feature] };
const beta = project("beta");

const login = pullRequest(7, {
  title: "Fix the login redirect",
  author: "ada",
  branch: "ys/feature",
  details: { updatedAt: "2026-09-30T10:00:00Z", headOid: "abc123def456789", additions: 69 },
});
const retry = pullRequest(8, {
  title: "Add a retry to the uploader",
  author: "grace",
  branch: "grace/retry",
  details: {
    updatedAt: "2026-09-29T10:00:00Z",
    review: "REVIEW_REQUIRED",
    reviewRequests: [{ name: "ada", team: false }],
    reviews: [{ login: "linus", state: "commented" }],
    headOid: "0123456789abcdef",
  },
});
const parser = pullRequest(5, {
  title: "Bump the parser",
  author: "grace",
  state: "merged",
  details: { updatedAt: "2026-09-20T10:00:00Z" },
});
const flag = pullRequest(4, {
  title: "Drop the old flag",
  author: "ken",
  state: "closed",
  details: { updatedAt: "2026-09-19T10:00:00Z" },
});
const paint = pullRequest(3, {
  title: "Speed up the first paint",
  author: "ken",
  checks: "failing",
  details: {
    updatedAt: "2026-09-28T10:00:00Z",
    checkCounts: { passed: 9, failed: 1, running: 0 },
  },
});

const answers = (): Record<string, ProjectPullRequests> => ({
  [alpha.id]: pullRequestsOf([login, retry, parser, flag]),
  [beta.id]: pullRequestsOf([paint], {
    repo: { host: "github.com", owner: "demo", name: "site", kind: "github" },
  }),
});

function show(byProject = answers(), ui: Record<string, string> = {}) {
  usePublishStore.setState({ byProject });
  useProjectsStore.setState({ ui });
  render(<PullRequestsView />);
  return userEvent.setup();
}

const row = (title: RegExp) => screen.getByRole("button", { name: title });
const titles = () =>
  within(screen.getByRole("list", { name: "Pull requests" }))
    .getAllByRole("listitem")
    .map((item) => item.querySelector("[data-pr-row] > span:nth-child(2)")?.textContent);
/** What the filters were last saved as, in the core's `ui_state`. */
const saved = () => {
  const calls = core.uiStateSave.mock.calls.filter((call) => call[0] === FILTERS_KEY);
  const last = calls[calls.length - 1];
  return last ? JSON.parse(last[1] as string) : null;
};

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  core.uiStateSave.mockResolvedValue(undefined);
  core.projectPullRequests.mockImplementation(async (id: string) => answers()[id]);
  core.pullRequestSummary.mockImplementation(async (id: string, number: number) =>
    pullRequestSummary(answers()[id]!.pullRequests.find((pr) => pr.number === number)!),
  );
  core.sessionsList.mockResolvedValue([]);
  core.ptySpawn.mockResolvedValue({ id: "s1", labels: { workspace: feature.id } });
  opener.openUrl.mockResolvedValue(undefined);
  clipboard.writeText.mockResolvedValue(undefined);
  for (const fn of [core.pullRequestMerge, core.pullRequestClose, core.pullRequestReopen]) {
    fn.mockResolvedValue(undefined);
  }
  useProjectsStore.setState({
    projects: [alpha, beta],
    loaded: true,
    ui: {},
    collapsed: [],
    selectedWorkspaceId: null,
    composingProjectId: null,
    composingBranch: null,
    workflowId: null,
    usageOpen: false,
    pullRequestsOpen: true,
    error: null,
  });
  usePublishStore.setState({ byProject: {}, workspaceId: null, state: null, busy: null });
  usePullRequestsStore.setState({
    selected: null,
    summaries: {},
    busy: null,
    error: null,
    notice: null,
  });
  useSessionsStore.setState({ byWorkspace: {}, error: null });
  useTerminalStore.setState({ tabs: [], active: {}, error: null });
});

describe("the list", () => {
  it("shows every project's open pull requests, most recently changed first", () => {
    show();
    expect(titles()).toEqual([
      "Fix the login redirect",
      "Add a retry to the uploader",
      "Speed up the first paint",
    ]);
    const first = row(/Fix the login redirect/);
    expect(first).toHaveTextContent("alpha");
    expect(first).toHaveTextContent("#7");
    expect(first).toHaveTextContent("Open");
    expect(first).toHaveTextContent("ada");
    expect(first).toHaveTextContent("+69");
    expect(within(first).getByLabelText("3 of 3 checks passed")).toHaveTextContent("3/3");
    // A failure is said, not only coloured.
    expect(
      within(row(/Speed up the first paint/)).getByLabelText("9 of 10 checks passed, 1 failed"),
    ).toHaveTextContent("9/10");
  });

  it("names the workspace a pull request has, and goes there", async () => {
    const user = show();
    await user.click(screen.getByRole("button", { name: "feature" }));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe(feature.id);
    expect(useProjectsStore.getState().pullRequestsOpen, "the view gives way").toBe(false);
  });

  it("moves between rows with Up, Down, Home and End", async () => {
    const user = show();
    row(/Fix the login redirect/).focus();
    await user.keyboard("{ArrowDown}");
    expect(row(/Add a retry/)).toHaveFocus();
    await user.keyboard("{End}");
    expect(row(/Speed up the first paint/)).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(row(/Speed up the first paint/), "stops at the end").toHaveFocus();
    await user.keyboard("{Home}{ArrowUp}");
    expect(row(/Fix the login redirect/)).toHaveFocus();
  });
});

describe("filters", () => {
  it("switches state with the tabs and remembers it", async () => {
    const user = show();
    expect(screen.getByRole("tab", { name: "Open" })).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getByRole("tab", { name: "Merged" }));
    expect(titles()).toEqual(["Bump the parser"]);
    expect(saved()).toMatchObject({ state: "merged" });
    await user.click(screen.getByRole("tab", { name: "Closed" }));
    expect(titles()).toEqual(["Drop the old flag"]);
    await user.click(screen.getByRole("tab", { name: "All" }));
    expect(titles()).toHaveLength(5);
  });

  it("narrows by project, author and review status, together, and remembers each", async () => {
    const user = show();
    await user.click(screen.getByRole("button", { name: "Projects: All projects" }));
    await user.click(screen.getByRole("menuitemradio", { name: "alpha" }));
    expect(titles()).toEqual(["Fix the login redirect", "Add a retry to the uploader"]);
    expect(screen.getByRole("button", { name: "Projects: alpha" })).toBeVisible();

    await user.selectOptions(screen.getByRole("combobox", { name: "Author" }), "grace");
    expect(titles()).toEqual(["Add a retry to the uploader"]);
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Reviews" }),
      "Awaiting review from you",
    );
    expect(titles()).toEqual(["Add a retry to the uploader"]);
    expect(saved()).toEqual({
      state: "open",
      projects: [alpha.id],
      author: "grace",
      reviews: "awaitingYou",
    });

    await user.selectOptions(screen.getByRole("combobox", { name: "Reviews" }), "Approved");
    expect(screen.getByRole("status")).toHaveTextContent("No pull requests match.");
  });

  it("offers Me by name, and filters to your own", async () => {
    const user = show();
    await user.selectOptions(screen.getByRole("combobox", { name: "Author" }), "Me (ada)");
    expect(titles()).toEqual(["Fix the login redirect"]);
  });

  it("searches by title and by number, and is not remembered", async () => {
    const user = show();
    const search = screen.getByRole("searchbox", { name: "Search pull requests" });
    await user.type(search, "retry");
    expect(titles()).toEqual(["Add a retry to the uploader"]);
    await user.clear(search);
    await user.type(search, "#3");
    expect(titles()).toEqual(["Speed up the first paint"]);
    expect(core.uiStateSave).not.toHaveBeenCalled();
    // Escape empties the box before it does anything else.
    await user.keyboard("{Escape}");
    expect(search).toHaveValue("");
    expect(titles()).toHaveLength(3);
  });

  it("clears what narrows the list and keeps the tab", async () => {
    const user = show(answers(), {
      [FILTERS_KEY]: JSON.stringify({
        state: "all",
        projects: [],
        author: "nobody-here",
        reviews: "any",
      }),
    });
    // A remembered author nobody matches is shown, so it can be seen and undone.
    expect(screen.getByRole("combobox", { name: "Author" })).toHaveValue("nobody-here");
    expect(screen.getByRole("status")).toHaveTextContent("No pull requests match.");
    await user.click(
      within(screen.getByRole("status")).getByRole("button", { name: "Clear filters" }),
    );
    expect(titles()).toHaveLength(5);
    expect(screen.getByRole("tab", { name: "All" })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("button", { name: "Clear filters" })).not.toBeInTheDocument();
  });

  it("comes back as it was left, without a project that has gone since", () => {
    show(answers(), {
      [FILTERS_KEY]: JSON.stringify({
        state: "merged",
        projects: ["p-removed"],
        author: null,
        reviews: "any",
      }),
    });
    expect(screen.getByRole("tab", { name: "Merged" })).toHaveAttribute("aria-selected", "true");
    // Filtering on a project that is not there would hide the one merged pull request.
    expect(titles()).toEqual(["Bump the parser"]);
    expect(screen.getByRole("button", { name: "Projects: All projects" })).toBeVisible();
  });

  it("does not offer the filters about you until gh has said who you are", () => {
    const anonymous = answers();
    for (const id of Object.keys(anonymous)) anonymous[id] = { ...anonymous[id]!, viewer: null };
    show(anonymous);
    expect(screen.getByRole("option", { name: "Awaiting review from you" })).toBeDisabled();
    expect(screen.getByRole("option", { name: "Reviewed by you" })).toBeDisabled();
    expect(screen.getByRole("option", { name: "Approved" })).toBeEnabled();
    expect(screen.queryByRole("option", { name: /^Me/ })).not.toBeInTheDocument();
  });
});

describe("when there is nothing to show, or not everything", () => {
  const only = (answer: Partial<ProjectPullRequests>) => {
    useProjectsStore.setState({ projects: [alpha] });
    return show({ [alpha.id]: pullRequestsOf([], answer) });
  };

  it("says gh is missing, once, and how to get it", async () => {
    const user = show({
      [alpha.id]: pullRequestsOf([], { gh: false }),
      [beta.id]: pullRequestsOf([], { gh: false }),
    });
    const notes = screen.getAllByRole("note");
    expect(notes).toHaveLength(1);
    expect(notes[0]).toHaveTextContent("gh is not installed");
    await user.click(within(notes[0]!).getByRole("button", { name: /Get it/ }));
    expect(opener.openUrl).toHaveBeenCalledWith("https://cli.github.com");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says when nobody is logged in, with the fix", () => {
    only({ loggedOut: true, problem: "gh: To get started, run: gh auth login" });
    expect(screen.getByRole("note")).toHaveTextContent("Run gh auth login");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("names a project that is on another forge, and one with no remote", () => {
    show({
      [alpha.id]: pullRequestsOf([], {
        repo: { host: "gitlab.com", owner: "o", name: "r", kind: "gitlab" },
      }),
      [beta.id]: pullRequestsOf([paint], { repo: null }),
    });
    const notes = screen.getAllByRole("note").map((note) => note.textContent);
    expect(notes).toEqual([
      "alpha is on GitLab. Pull requests are listed for GitHub only.",
      "beta has no remote on a forge, so there is nowhere to ask about pull requests.",
    ]);
  });

  it("says what gh said when it failed, and asks again on Retry", async () => {
    const user = only({ problem: "`gh pr list` failed: HTTP 504" });
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("alpha: `gh pr list` failed: HTTP 504");
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true);
    expect(core.projectPullRequests).toHaveBeenCalledTimes(1);
  });

  it("keeps the rows it has when the open ones could not all be read", () => {
    show({ ...answers(), [alpha.id]: pullRequestsOf([login], { openProblem: "HTTP 502" }) });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "alpha: not every open pull request could be read. HTTP 502",
    );
    expect(titles()).toContain("Fix the login redirect");
  });

  it("says when the forge has more open than the list holds, and where the rest are", async () => {
    const user = show({
      ...answers(),
      [alpha.id]: pullRequestsOf([login, retry], { openTotal: 1394 }),
    });
    const note = screen.getByRole("note");
    expect(note).toHaveTextContent("Showing the 2 most recently updated of 1394 open in alpha.");
    await user.click(within(note).getByRole("button", { name: /See all on GitHub/ }));
    expect(opener.openUrl).toHaveBeenCalledWith("https://github.com/demo/app/pulls");
  });

  it("says it is loading before any project has answered, and what is empty after", () => {
    show({});
    expect(screen.getByRole("status")).toHaveTextContent("Loading pull requests…");
    act(() => usePublishStore.setState({ byProject: { [alpha.id]: pullRequestsOf([parser]) } }));
    expect(screen.getByRole("status")).toHaveTextContent("No open pull requests.");
  });

  it("asks for every project again on Refresh", async () => {
    const user = show();
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(core.projectPullRequests).toHaveBeenCalledTimes(2));
    expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true);
    expect(core.projectPullRequests).toHaveBeenCalledWith(beta.id, true, true);
  });
});

describe("the details", () => {
  it("opens beside the list, and Escape closes them and goes back to the row", async () => {
    const user = show();
    await user.click(row(/Add a retry/));
    const pane = screen.getByRole("region", { name: "Pull request #8" });
    expect(row(/Add a retry/)).toHaveAttribute("aria-current", "true");
    expect(within(pane).getByText("grace/retry → main")).toBeVisible();
    expect(within(pane).getByText("Review required")).toBeVisible();
    expect(
      await within(pane).findByText("What Add a retry to the uploader is about."),
    ).toBeVisible();
    expect(core.pullRequestSummary).toHaveBeenCalledWith(alpha.id, 8, false);
    const reviewers = within(pane).getByRole("region", { name: "Reviewers" });
    expect(reviewers).toHaveTextContent("linuscommented");
    expect(reviewers).toHaveTextContent("adareview requested");

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: "Pull request #8" })).not.toBeInTheDocument();
    await waitFor(() => expect(row(/Add a retry/)).toHaveFocus());
  });

  it("can have the list out of the way, and back", async () => {
    const user = show();
    await user.click(row(/Add a retry/));
    await user.click(screen.getByRole("button", { name: "Hide the list" }));
    expect(screen.queryByRole("list", { name: "Pull requests" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Pull request #8" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Show the list" }));
    expect(screen.getByRole("list", { name: "Pull requests" })).toBeVisible();
  });

  it("opens it on the forge and copies its link", async () => {
    const user = show();
    await user.click(row(/Add a retry/));
    await user.click(screen.getByRole("button", { name: /Open on GitHub/ }));
    expect(opener.openUrl).toHaveBeenCalledWith(retry.url);
    await user.click(screen.getByRole("button", { name: "Copy link" }));
    expect(clipboard.writeText).toHaveBeenCalledWith(retry.url);
    expect(await screen.findByRole("button", { name: "Copied" })).toBeVisible();
  });

  it("closes when the pull request leaves the list", async () => {
    const user = show();
    await user.click(row(/Add a retry/));
    act(() => usePublishStore.setState({ byProject: { [alpha.id]: pullRequestsOf([login]) } }));
    expect(screen.queryByRole("region", { name: /Pull request #/ })).not.toBeInTheDocument();
  });
});

describe("merging, closing and reopening", () => {
  const merge = async (user: ReturnType<typeof userEvent.setup>, label: string) => {
    await user.click(screen.getByRole("button", { name: /^Merge/ }));
    await user.click(screen.getByRole("menuitem", { name: label }));
  };

  it("does not merge when the confirmation is cancelled", async () => {
    const user = show();
    native.confirm.mockResolvedValue(false);
    await user.click(row(/Add a retry/));
    await merge(user, "Squash and merge");
    await waitFor(() => expect(native.confirm).toHaveBeenCalled());
    expect(core.pullRequestMerge).not.toHaveBeenCalled();
    expect(core.projectPullRequests).not.toHaveBeenCalled();
  });

  it.each([
    ["Squash and merge", "squash"],
    ["Create a merge commit", "merge"],
    ["Rebase and merge", "rebase"],
  ])(
    "confirms %s naming whose it is, merges that head, then asks the forge again",
    async (label, method) => {
      const user = show();
      native.confirm.mockResolvedValue(true);
      await user.click(row(/Add a retry/));
      await merge(user, label);
      await waitFor(() =>
        expect(core.pullRequestMerge).toHaveBeenCalledWith(alpha.id, 8, "0123456789abcdef", method),
      );
      const [message, options] = native.confirm.mock.calls[0]!;
      expect(message).toContain(`${label} pull request #8: Add a retry to the uploader`);
      expect(message).toContain("grace opened it, not you.");
      expect(message).toContain("grace/retry → main");
      expect(message).toContain("Commit 0123456789ab");
      expect(message).toContain("No branch is deleted.");
      expect(options).toEqual({ title: "Merge pull request", okLabel: label });
      await waitFor(() =>
        expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true),
      );
      expect(await screen.findByRole("status")).toHaveTextContent("Merge request sent.");
    },
  );

  it("says a pull request is yours when it is", async () => {
    const user = show();
    native.confirm.mockResolvedValue(false);
    await user.click(row(/Fix the login redirect/));
    await merge(user, "Squash and merge");
    await waitFor(() => expect(native.confirm).toHaveBeenCalled());
    expect(native.confirm.mock.calls[0]![0]).toContain("You opened it.");
  });

  it("will not merge a draft or one that conflicts, and says why", async () => {
    const draft = pullRequest(20, { title: "Half done", draft: true });
    const clash = pullRequest(21, {
      title: "Clashing change",
      details: { mergeable: "conflicting" },
    });
    const user = show({ [alpha.id]: pullRequestsOf([draft, clash]) });
    await user.click(row(/Half done/));
    expect(screen.getByRole("button", { name: /^Merge/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Merge/ })).toHaveAttribute(
      "title",
      "It is a draft.",
    );
    await user.click(row(/Clashing change/));
    expect(screen.getByRole("button", { name: /^Merge/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Merge/ })).toHaveAttribute(
      "title",
      "It conflicts with main.",
    );
  });

  it("does not close when the confirmation is cancelled", async () => {
    const user = show();
    native.confirm.mockResolvedValue(false);
    await user.click(row(/Add a retry/));
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(native.confirm).toHaveBeenCalled());
    expect(core.pullRequestClose).not.toHaveBeenCalled();
  });

  it("closes after a confirmation that says the branch is kept", async () => {
    const user = show();
    native.confirm.mockResolvedValue(true);
    await user.click(row(/Add a retry/));
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(core.pullRequestClose).toHaveBeenCalledWith(alpha.id, 8));
    // What the forge says about it now is asked for too, not taken from before the close.
    await waitFor(() =>
      expect(core.pullRequestSummary).toHaveBeenLastCalledWith(alpha.id, 8, true),
    );
    const [message, options] = native.confirm.mock.calls[0]!;
    expect(message).toContain("Close pull request #8 without merging it?");
    expect(message).toContain("grace opened it, not you.");
    expect(message).toContain("Its branch is kept");
    expect(options).toEqual({ title: "Close pull request", okLabel: "Close pull request" });
    await waitFor(() =>
      expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true),
    );
  });

  it("offers Reopen on a closed one only, and does nothing on a no", async () => {
    const user = show();
    await user.click(screen.getByRole("tab", { name: "All" }));
    await user.click(row(/Bump the parser/));
    // Merged: nothing left to do to it but read it.
    for (const name of [/^Merge/, "Close", "Reopen", "Start workspace"]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument();
    }

    await user.click(row(/Drop the old flag/));
    expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
    native.confirm.mockResolvedValue(false);
    await user.click(screen.getByRole("button", { name: "Reopen" }));
    await waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));
    expect(core.pullRequestReopen).not.toHaveBeenCalled();

    native.confirm.mockResolvedValue(true);
    await user.click(screen.getByRole("button", { name: "Reopen" }));
    await waitFor(() => expect(core.pullRequestReopen).toHaveBeenCalledWith(alpha.id, 4));
    expect(native.confirm.mock.calls[1]![0]).toContain("Its reviewers are told");
  });

  it("shows the forge's refusal, and asks it again all the same", async () => {
    const user = show();
    native.confirm.mockResolvedValue(true);
    core.pullRequestMerge.mockRejectedValue({
      code: "gh_failed",
      message: "`gh pr merge 8` failed: Pull request is not mergeable: required checks",
    });
    await user.click(row(/Add a retry/));
    await merge(user, "Squash and merge");
    expect(await screen.findByRole("alert")).toHaveTextContent("required checks");
    expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true);
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("workspaces", () => {
  it("goes to the workspace a pull request already has, instead of starting one", async () => {
    const user = show();
    await user.click(row(/Fix the login redirect/));
    const pane = screen.getByRole("region", { name: "Pull request #7" });
    expect(within(pane).queryByRole("button", { name: "Start workspace" })).not.toBeInTheDocument();
    await user.click(within(pane).getByRole("button", { name: "Go to workspace" }));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe(feature.id);
    expect(useProjectsStore.getState().pullRequestsOpen).toBe(false);
    expect(core.pullRequestPrepareBranch).not.toHaveBeenCalled();
  });

  it("gets the branch ready and hands over to the composer", async () => {
    const user = show();
    const prepared = { branch: "grace/retry", behind: 0, fork: false };
    core.pullRequestPrepareBranch.mockResolvedValue(prepared);
    await user.click(row(/Add a retry/));
    await user.click(screen.getByRole("button", { name: "Start workspace" }));
    await waitFor(() => expect(core.pullRequestPrepareBranch).toHaveBeenCalledWith(alpha.id, 8));
    await waitFor(() => expect(useProjectsStore.getState().composingProjectId).toBe(alpha.id));
    expect(useProjectsStore.getState().composingBranch).toEqual(prepared);
    expect(useProjectsStore.getState().pullRequestsOpen, "the composer takes the panel").toBe(
      false,
    );
  });

  it("stays put and says why when the branch cannot be got ready", async () => {
    const user = show();
    core.pullRequestPrepareBranch.mockRejectedValue({
      code: "branch_checked_out",
      message: '"grace/retry" is already checked out at /somewhere/else.',
    });
    await user.click(row(/Add a retry/));
    await user.click(screen.getByRole("button", { name: "Start workspace" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("already checked out");
    expect(useProjectsStore.getState().composingProjectId).toBeNull();
    expect(useProjectsStore.getState().pullRequestsOpen).toBe(true);
  });
});
