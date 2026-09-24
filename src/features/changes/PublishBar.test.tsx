import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChangeSet, FileChange, PublishState, PullRequest } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  draftStatus: vi.fn(),
  draftCommitMessage: vi.fn(),
  draftPullRequest: vi.fn(),
  workspacePublishState: vi.fn(),
  workspaceCommit: vi.fn(),
  workspacePush: vi.fn(),
  workspaceOpenPullRequest: vi.fn(),
  projectPullRequests: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));

import { useDraftStore } from "@/stores/draft";
import { usePublishStore } from "@/stores/publish";
import { PublishBar } from "./PublishBar";

const change = (path: string): FileChange => ({
  path,
  oldPath: null,
  kind: "modified",
  additions: 1,
  deletions: 0,
});

const changes = (paths: string[]): ChangeSet => ({
  uncommitted: paths.map(change),
  committed: [],
  base: "main",
});

const state = (extra: Partial<PublishState> = {}): PublishState => ({
  branch: "ys/fix-login",
  base: "main",
  identity: "Ada <ada@example.com>",
  remote: "origin",
  repo: { host: "github.com", owner: "o", name: "r", kind: "github" },
  upstream: null,
  ahead: 0,
  behind: 0,
  unpushed: [],
  pullRequest: null,
  canOpen: true,
  gh: true,
  problem: null,
  loggedOut: false,
  ...extra,
});

/** A commit as `PublishState.unpushed` carries it. */
const commit = (subject: string, body = "") => ({ subject, body });

const pr = (extra: Partial<PullRequest> = {}): PullRequest => ({
  number: 42,
  url: "https://github.com/o/r/pull/42",
  title: "Fix the login redirect",
  branch: "ys/fix-login",
  state: "open",
  draft: false,
  checks: "passing",
  ...extra,
});

function seed(publish: Partial<PublishState> | null = {}) {
  usePublishStore.setState({
    workspaceId: "w1",
    state: publish === null ? null : state(publish),
    error: null,
    busy: null,
    byProject: {},
  });
}

function show(paths: string[] = ["src/login.rs"]) {
  const { rerender } = render(<PublishBar changes={changes(paths)} workspaceId="w1" />);
  return {
    user: userEvent.setup(),
    /** What the changes panel does after a commit: ask git again and pass on the answer. */
    showing: (next: string[]) => rerender(<PublishBar changes={changes(next)} workspaceId="w1" />),
  };
}

describe("PublishBar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    opener.openUrl.mockResolvedValue(undefined);
    native.confirm.mockResolvedValue(true);
    // Nothing can write unless a case says so, so the ✦ button stays out of the way.
    useDraftStore.setState({ status: null, busy: null, error: null });
    core.workspacePublishState.mockResolvedValue(state());
    core.projectPullRequests.mockResolvedValue({
      gh: true,
      pullRequests: [],
      problem: null,
      loggedOut: false,
    });
    seed();
  });

  it("will not commit without a message, and says how many files it would take", async () => {
    const { user, showing } = show(["src/login.rs", "src/api.rs"]);
    const button = screen.getByRole("button", { name: "Commit 2 files" });
    expect(button).toBeDisabled();

    await user.type(screen.getByLabelText("Commit message"), "Fix the login redirect");
    expect(button).toBeEnabled();

    core.workspaceCommit.mockResolvedValue(
      state({ ahead: 1, unpushed: [commit("Fix the login redirect")] }),
    );
    await user.click(button);
    expect(core.workspaceCommit).toHaveBeenCalledWith("w1", "Fix the login redirect");

    // Once the files are committed the box goes, and comes back empty for the next batch.
    showing([]);
    expect(screen.queryByLabelText("Commit message")).not.toBeInTheDocument();
    showing(["src/session.rs"]);
    expect(screen.getByLabelText("Commit message")).toHaveValue("");
  });

  it("offers nothing to commit when nothing is uncommitted", async () => {
    show([]);
    expect(screen.queryByLabelText("Commit message")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Commit/ })).not.toBeInTheDocument();
  });

  it("explains itself instead of failing when git has no identity", async () => {
    seed({ identity: null });
    const { user } = show();
    await user.type(screen.getByLabelText("Commit message"), "Fix it");
    expect(screen.getByRole("button", { name: "Commit 1 file" })).toBeDisabled();
    expect(screen.getByText(/git config --global user.name/)).toBeInTheDocument();
  });

  it("offers a push only when the remote is behind, and counts the commits", async () => {
    seed({ ahead: 0 });
    const { unmount } = render(<PublishBar changes={changes([])} workspaceId="w1" />);
    expect(screen.queryByRole("button", { name: /^Push/ })).not.toBeInTheDocument();
    unmount();

    seed({ ahead: 2, unpushed: [commit("Second"), commit("First")] });
    const { user } = show([]);
    core.workspacePush.mockResolvedValue(state({ ahead: 0, upstream: "origin/ys/fix-login" }));
    await user.click(screen.getByRole("button", { name: "Push 2 commits" }));
    expect(core.workspacePush).toHaveBeenCalledWith("w1");
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /^Push/ })).not.toBeInTheDocument(),
    );
  });

  it("opens a pull request from the commits the remote has not got, and shows it afterwards", async () => {
    seed({
      ahead: 2,
      unpushed: [commit("Tidy the redirect"), commit("Fix the login redirect")],
    });
    const { user } = show([]);
    await user.click(screen.getByRole("button", { name: "Open pull request" }));

    // The oldest commit is the one the branch is about; all of them are listed, as they happened.
    expect(screen.getByLabelText("Title")).toHaveValue("Fix the login redirect");
    expect(screen.getByLabelText("Description")).toHaveValue(
      "- Fix the login redirect\n- Tidy the redirect",
    );
    expect(screen.getByText(/2 commits will be pushed to/)).toBeInTheDocument();

    core.workspaceOpenPullRequest.mockResolvedValue({ url: pr().url, created: true });
    core.workspacePublishState.mockResolvedValue(state({ pullRequest: pr(), canOpen: false }));
    await user.click(screen.getByRole("button", { name: "Open" }));

    expect(core.workspaceOpenPullRequest).toHaveBeenCalledWith("w1", {
      title: "Fix the login redirect",
      body: "- Fix the login redirect\n- Tidy the redirect",
      draft: false,
    });
    await waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith(pr().url));
    expect(await screen.findByRole("button", { name: /#42 · checks passing/ })).toBeInTheDocument();
  });

  it("uses a single commit's own body as the description", async () => {
    seed({
      ahead: 1,
      unpushed: [
        commit(
          "Fix the login redirect",
          "The cookie was set on the wrong domain,\nso the session never came back.",
        ),
      ],
    });
    const { user } = show([]);
    await user.click(screen.getByRole("button", { name: "Open pull request" }));
    expect(screen.getByLabelText("Title")).toHaveValue("Fix the login redirect");
    expect(screen.getByLabelText("Description")).toHaveValue(
      "The cookie was set on the wrong domain,\nso the session never came back.",
    );
  });

  it("confirms every commit, naming the files and the branch", async () => {
    const { user } = show(["src/login.rs", "src/api.rs"]);
    await user.type(screen.getByLabelText("Commit message"), "Fix the login redirect");
    core.workspaceCommit.mockResolvedValue(state());
    await user.click(screen.getByRole("button", { name: "Commit 2 files" }));

    expect(native.confirm).toHaveBeenCalledWith(
      expect.stringContaining('Commit all 2 files to "ys/fix-login"?'),
      expect.objectContaining({ title: "Commit 2 files", okLabel: "Commit" }),
    );
    expect(native.confirm.mock.calls[0]![0]).toContain("Fix the login redirect");
    expect(core.workspaceCommit).toHaveBeenCalledWith("w1", "Fix the login redirect");
  });

  it("commits nothing when the confirmation is declined", async () => {
    native.confirm.mockResolvedValue(false);
    const { user } = show();
    await user.type(screen.getByLabelText("Commit message"), "Fix it");
    await user.click(screen.getByRole("button", { name: "Commit 1 file" }));

    expect(native.confirm).toHaveBeenCalled();
    expect(core.workspaceCommit).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Commit message")).toHaveValue("Fix it");
  });

  it("says the browser will finish the job when gh is not installed", async () => {
    seed({ gh: false, ahead: 1, unpushed: [commit("Fix the login redirect")] });
    const { user } = show([]);
    await user.click(screen.getByRole("button", { name: "Open pull request" }));
    expect(screen.getByText(/is not installed, so the branch will be pushed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Push and continue" })).toBeInTheDocument();
  });

  it("says how to log gh in, rather than treating it as a failure", async () => {
    seed({ loggedOut: true, problem: "gh: To get started, run: gh auth login", ahead: 1 });
    const { user } = show([]);
    await user.click(screen.getByRole("button", { name: "Open pull request" }));
    expect(screen.getByText(/is installed but not logged in/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Push and continue" })).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says why there are no check results when gh could not read the repository", async () => {
    seed({ problem: "none of the git remotes point to a known GitHub host", ahead: 1 });
    const { user } = show([]);
    await user.click(screen.getByRole("button", { name: "Open pull request" }));
    expect(screen.getByText(/no check results on the workspace rows/)).toHaveTextContent(
      "none of the git remotes point to a known GitHub host",
    );
  });

  describe("writing with a model", () => {
    const canWrite = (extra = {}) =>
      useDraftStore.setState({
        status: {
          enabled: true,
          available: true,
          harness: "Claude Code",
          key: false,
          model: "claude-opus-5",
          problem: null,
        },
        busy: null,
        error: null,
        ...extra,
      });

    it("offers nothing when nothing can write", async () => {
      show();
      expect(
        screen.queryByRole("button", { name: "Write the commit message" }),
      ).not.toBeInTheDocument();
    });

    it("puts the written message in the box, for you to read before committing", async () => {
      canWrite();
      const { user } = show();
      core.draftCommitMessage.mockResolvedValue("Fix the login redirect\n\nThe cookie was wrong.");

      await user.click(screen.getByRole("button", { name: "Write the commit message" }));
      expect(core.draftCommitMessage).toHaveBeenCalledWith("w1", null);
      await waitFor(() =>
        expect(screen.getByLabelText("Commit message")).toHaveValue(
          "Fix the login redirect\n\nThe cookie was wrong.",
        ),
      );
      // Written, not committed: the confirmation still stands between it and git.
      expect(native.confirm).not.toHaveBeenCalled();
      expect(core.workspaceCommit).not.toHaveBeenCalled();
    });

    it("leaves what you typed alone when the model cannot answer", async () => {
      canWrite();
      const { user } = show();
      await user.type(screen.getByLabelText("Commit message"), "mine");
      core.draftCommitMessage.mockRejectedValue({
        code: "draft_agent_timeout",
        message: "Claude Code took longer than 120 seconds to answer.",
      });

      await user.click(screen.getByRole("button", { name: "Write the commit message" }));
      await waitFor(() =>
        expect(useDraftStore.getState().error).toBe(
          "Claude Code took longer than 120 seconds to answer.",
        ),
      );
      expect(screen.getByLabelText("Commit message")).toHaveValue("mine");
    });

    it("fills both pull request fields from one press", async () => {
      canWrite();
      seed({ ahead: 1, unpushed: [commit("Fix it")] });
      const { user } = show([]);
      await user.click(screen.getByRole("button", { name: "Open pull request" }));

      core.draftPullRequest.mockResolvedValue({
        title: "Fix the login redirect",
        body: "The cookie was set on the wrong domain.",
      });
      await user.click(screen.getByRole("button", { name: "Write the title and description" }));

      await waitFor(() =>
        expect(screen.getByLabelText("Title")).toHaveValue("Fix the login redirect"),
      );
      expect(screen.getByLabelText("Description")).toHaveValue(
        "The cookie was set on the wrong domain.",
      );
      expect(core.workspaceOpenPullRequest).not.toHaveBeenCalled();
    });

    it("names whoever would do the writing", async () => {
      canWrite();
      show();
      expect(screen.getByRole("button", { name: "Write the commit message" })).toHaveAttribute(
        "title",
        "Write the commit message with Claude Code",
      );

      act(() =>
        useDraftStore.setState({
          status: {
            enabled: true,
            available: true,
            harness: null,
            key: true,
            model: "claude-opus-5",
            problem: null,
          },
        }),
      );
      expect(screen.getByRole("button", { name: "Write the commit message" })).toHaveAttribute(
        "title",
        "Write the commit message with claude-opus-5",
      );
    });
  });

  it("calls it a merge request on GitLab", async () => {
    seed({ repo: { host: "gitlab.com", owner: "o", name: "r", kind: "gitlab" } });
    show([]);
    expect(screen.getByRole("button", { name: "Open merge request" })).toBeInTheDocument();
  });

  it("offers nothing to open when the pull request is already open", async () => {
    seed({ pullRequest: pr(), canOpen: false });
    show([]);
    expect(screen.queryByRole("button", { name: /^Open/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /#42 · checks passing/ })).toBeInTheDocument();
  });

  it("stays out of the way on a detached HEAD", async () => {
    seed({ branch: null, canOpen: false });
    show();
    expect(screen.queryByLabelText("Commit message")).not.toBeInTheDocument();
  });

  it("shows what the core said went wrong", async () => {
    const { user } = show();
    core.workspaceCommit.mockRejectedValue({
      code: "no_git_identity",
      message: "git has no name and email to commit with.",
    });
    await user.type(screen.getByLabelText("Commit message"), "Fix it");
    await user.click(screen.getByRole("button", { name: "Commit 1 file" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "git has no name and email to commit with.",
    );
    expect(screen.getByLabelText("Commit message")).toHaveValue("Fix it");
  });
});
