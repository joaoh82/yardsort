import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PullRequest, PullRequestSummary } from "@/lib/ipc";
import { project, pullRequest, pullRequestsOf, pullRequestSummary } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  pullRequestSummary: vi.fn(),
  pullRequestComment: vi.fn(),
  projectPullRequests: vi.fn(),
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@tauri-apps/plugin-opener", () => opener);

import { usePullRequestsStore } from "@/stores/pullRequests";
import { rowsOf } from "./rows";
import { Summary } from "./Summary";

const alpha = project("alpha");
const now = Date.parse("2026-10-02T12:00:00Z");
const at = (iso: string) => Date.parse(iso);

const listed = pullRequest(8, {
  title: "Add a retry to the uploader",
  author: "grace",
  details: { updatedAt: "2026-10-01T10:00:00Z", headOid: "abc" },
});
const rowOf = (pr: PullRequest) => rowsOf([alpha], { [alpha.id]: pullRequestsOf([pr]) })[0]!;

function show(pr: PullRequest = listed) {
  const view = render(<Summary row={rowOf(pr)} now={now} />);
  return {
    user: userEvent.setup(),
    /** What the minute's poll does: the same pull request, as a new object. */
    polled: (next: PullRequest) => view.rerender(<Summary row={rowOf(next)} now={now} />),
  };
}

const answer = (summary: Partial<PullRequestSummary> = {}, pr: PullRequest = listed) =>
  core.pullRequestSummary.mockResolvedValue(pullRequestSummary(pr, summary));

beforeEach(() => {
  vi.resetAllMocks();
  opener.openUrl.mockResolvedValue(undefined);
  core.pullRequestComment.mockResolvedValue(undefined);
  core.projectPullRequests.mockResolvedValue(pullRequestsOf([listed]));
  usePullRequestsStore.setState({
    selected: null,
    summaries: {},
    changes: {},
    tab: "summary",
    busy: null,
    error: null,
  });
});

describe("the description", () => {
  it("says it is loading, then shows what the author wrote, as Markdown", async () => {
    answer({ body: "## Why\n\nUploads fail on a **flaky** network.\n\n<!-- template -->" });
    show();
    const section = screen.getByRole("region", { name: "Description" });
    expect(section).toHaveTextContent("Loading…");
    expect(await within(section).findByRole("heading", { name: "Why" })).toBeVisible();
    expect(section).toHaveTextContent("Uploads fail on a flaky network.");
    expect(section).not.toHaveTextContent("template");
    expect(core.pullRequestSummary).toHaveBeenCalledWith(alpha.id, 8, false);
  });

  it("says so when there is none", async () => {
    answer({ body: "  \n" });
    show();
    expect(await screen.findByText("No description.")).toBeVisible();
  });
});

describe("the checks", () => {
  it("sums them up from the list until each arrives by name, with its run", async () => {
    const read = pullRequest(8, {
      checks: "failing",
      details: {
        checkCounts: { passed: 1, failed: 1, running: 1 },
        checks: [
          { name: "build", state: "passing", workflow: "CI", url: "https://ci.example.com/1" },
          { name: "lint", state: "running", workflow: "CI", url: "https://ci.example.com/2" },
          { name: "test", state: "failing", workflow: "CI", url: "https://ci.example.com/3" },
          { name: "ci/legacy", state: "passing", workflow: null, url: null },
        ],
      },
    });
    let arrive: (summary: PullRequestSummary) => void = () => {};
    core.pullRequestSummary.mockReturnValue(new Promise((resolve) => (arrive = resolve)));
    const { user } = show();
    const section = screen.getByRole("region", { name: "Checks" });
    // The list's own counts, before anything else is known.
    expect(section).toHaveTextContent("All 3 passed");
    expect(within(section).queryByRole("listitem")).not.toBeInTheDocument();

    arrive(pullRequestSummary(read));
    await waitFor(() => expect(section).toHaveTextContent("1 of 3 failed"));
    // What needs attention comes first.
    expect(
      within(section)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["CI / test ↗failed", "CI / lint ↗running", "CI / build ↗passed", "ci/legacypassed"]);
    await user.click(within(section).getByRole("button", { name: /CI \/ test/ }));
    expect(opener.openUrl).toHaveBeenCalledWith("https://ci.example.com/3");
    // A check with nowhere to go is not a button to nowhere.
    expect(within(section).queryByRole("button", { name: /legacy/ })).not.toBeInTheDocument();
  });
});

describe("the reviewers", () => {
  it("are the ones the forge names now, not the ones the list remembers", async () => {
    answer(
      {},
      pullRequest(8, {
        details: {
          reviewRequests: [{ name: "maintainers", team: true }],
          reviews: [{ login: "linus", state: "approved" }],
        },
      }),
    );
    show();
    const section = screen.getByRole("region", { name: "Reviewers" });
    expect(section).toHaveTextContent("Nobody has been asked");
    await waitFor(() => expect(section).toHaveTextContent("linusapproved"));
    expect(section).toHaveTextContent("team maintainersreview requested");
  });
});

describe("the conversation", () => {
  it("shows what was said, oldest first, and what each person did", async () => {
    answer({
      posts: [
        {
          kind: "review",
          author: "linus",
          at: at("2026-09-30T12:00:00Z"),
          body: "",
          url: listed.url,
          review: "commented",
          hidden: null,
        },
        {
          kind: "comment",
          author: "grace",
          at: at("2026-10-01T12:00:00Z"),
          body: "Fixed in the **last** commit.",
          url: `${listed.url}#issuecomment-1`,
          review: null,
          hidden: null,
        },
        {
          kind: "comment",
          author: "spammer",
          at: at("2026-10-01T13:00:00Z"),
          body: "",
          url: null,
          review: null,
          hidden: "off_topic",
        },
        {
          kind: "review",
          author: null,
          at: at("2026-10-02T11:00:00Z"),
          body: "",
          url: listed.url,
          review: "approved",
          hidden: null,
        },
        {
          kind: "review",
          author: "ada",
          at: at("2026-10-02T11:59:50Z"),
          body: "Not yet: the retry never stops.",
          url: listed.url,
          review: "changesRequested",
          hidden: null,
        },
      ],
    });
    const { user } = show();
    const section = screen.getByRole("region", { name: "Conversation" });
    const posts = await within(section).findAllByRole("listitem");
    expect(posts.map((post) => post.querySelector("p")?.textContent)).toEqual([
      "linusreviewed2d ago↗",
      "gracecommented1d ago↗",
      "spammercommented23h ago",
      "ghostapproved1h ago↗",
      "adarequested changesjust now↗",
    ]);
    // A review with no words of its own is one whose words are on lines.
    expect(posts[0]).toHaveTextContent("Left comments on lines of the code.");
    await waitFor(() => expect(within(posts[1]!).getByText("last")).toBeVisible());
    expect(posts[1]!.querySelector("strong")).toHaveTextContent("last");
    // What the forge hid stays hidden, and says so.
    expect(posts[2]).toHaveTextContent("Hidden on GitHub as off topic.");
    expect(posts[3]).toHaveTextContent("Nothing further.");
    expect(posts[4]).toHaveTextContent("Not yet: the retry never stops.");

    await user.click(within(posts[1]!).getByRole("button", { name: "Open on GitHub" }));
    expect(opener.openUrl).toHaveBeenCalledWith(`${listed.url}#issuecomment-1`);
    // Comments on lines are under Code, and the conversation says so.
    expect(section).toHaveTextContent("Comments on particular lines are under Code");
  });

  it("says so when nothing has been said", async () => {
    answer();
    show();
    expect(await screen.findByText("Nobody has commented or reviewed yet.")).toBeVisible();
  });
});

describe("replying", () => {
  it("posts what was written, with Ctrl+Enter, and reads everything again", async () => {
    answer();
    const { user } = show();
    const box = await screen.findByRole("textbox", { name: "Your comment" });
    const send = screen.getByRole("button", { name: "Comment" });
    expect(send).toBeDisabled();
    await user.type(box, "Looks right to me.{Control>}{Enter}{/Control}");
    await waitFor(() =>
      expect(core.pullRequestComment).toHaveBeenCalledWith(alpha.id, 8, "Looks right to me."),
    );
    await waitFor(() => expect(box).toHaveValue(""));
    // The list, and the summary past the core's cache: the conversation has moved.
    await waitFor(() =>
      expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true),
    );
    await waitFor(() =>
      expect(core.pullRequestSummary).toHaveBeenLastCalledWith(alpha.id, 8, true),
    );
    expect(usePullRequestsStore.getState().notice).toBe("Commented on #8.");
  });

  it("clears the box as soon as the post lands, not after everything is read again", async () => {
    answer();
    // The reading-again is `gh` over the network. Here it never comes back at all.
    core.projectPullRequests.mockReturnValue(new Promise(() => {}));
    const { user } = show();
    const box = await screen.findByRole("textbox", { name: "Your comment" });
    await user.type(box, "Once.");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(core.pullRequestComment).toHaveBeenCalledTimes(1));
    // Were the box still full and the button still live, a second press would post it again.
    await waitFor(() => expect(box).toHaveValue(""));
    expect(core.projectPullRequests).toHaveBeenCalledWith(alpha.id, true, true);
    await user.type(box, "Twice.{Control>}{Enter}{/Control}");
    await waitFor(() => expect(core.pullRequestComment).toHaveBeenCalledTimes(2));
    expect(core.pullRequestComment).toHaveBeenLastCalledWith(alpha.id, 8, "Twice.");
  });

  it("keeps what was written when the forge refuses, and says why", async () => {
    answer();
    core.pullRequestComment.mockRejectedValue({
      code: "gh_failed",
      message: "`gh pr comment 8` failed: HTTP 403",
    });
    const { user } = show();
    const box = await screen.findByRole("textbox", { name: "Your comment" });
    await user.type(box, "Not posted");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(usePullRequestsStore.getState().error).toContain("HTTP 403"));
    expect(box).toHaveValue("Not posted");
  });
});

describe("keeping up with the forge", () => {
  it("says why it could not be read, and reads it again on Retry", async () => {
    core.pullRequestSummary.mockRejectedValueOnce({
      code: "gh_failed",
      message: "`gh pr view 8` failed: HTTP 504",
    });
    const { user } = show();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "Could not read this pull request: `gh pr view 8` failed: HTTP 504",
    );
    // What the list knows is still shown.
    expect(screen.getByText("Add a retry to the uploader")).toBeVisible();
    expect(screen.getByRole("region", { name: "Checks" })).toHaveTextContent("All 3 passed");

    answer({ body: "Second time lucky." });
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByText("Second time lucky.")).toBeVisible());
    expect(core.pullRequestSummary).toHaveBeenLastCalledWith(alpha.id, 8, true);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("is read once while the list says the same, and again when the list says it changed", async () => {
    answer({ body: "As first written." });
    const { polled } = show();
    await waitFor(() => expect(screen.getByText("As first written.")).toBeVisible());

    // The minute's poll: a new object that says the same thing.
    polled(pullRequest(8, { ...listed, details: { ...listed.details } }));
    polled(pullRequest(8, { ...listed, details: { ...listed.details } }));
    expect(core.pullRequestSummary).toHaveBeenCalledTimes(1);

    // A comment or a push moves `updatedAt`: the summary is read again, past the core's cache.
    let arrive: (summary: PullRequestSummary) => void = () => {};
    core.pullRequestSummary.mockReturnValue(new Promise((resolve) => (arrive = resolve)));
    const moved = pullRequest(8, {
      ...listed,
      details: { ...listed.details, updatedAt: "2026-10-02T11:00:00Z" },
    });
    polled(moved);
    await waitFor(() => expect(core.pullRequestSummary).toHaveBeenCalledTimes(2));
    expect(core.pullRequestSummary).toHaveBeenLastCalledWith(alpha.id, 8, true);
    // What was there stays until the new one arrives: no blink back to "Loading…".
    expect(screen.getByText("As first written.")).toBeVisible();
    expect(screen.queryByText("Loading…")).not.toBeInTheDocument();
    arrive(pullRequestSummary(moved, { body: "As edited." }));
    await waitFor(() => expect(screen.getByText("As edited.")).toBeVisible());

    // Checks finishing move nothing but the counts, and that is enough.
    polled(
      pullRequest(8, {
        ...moved,
        details: { ...moved.details, checkCounts: { passed: 2, failed: 1, running: 0 } },
      }),
    );
    await waitFor(() => expect(core.pullRequestSummary).toHaveBeenCalledTimes(3));
  });

  it("drops an answer that a later question has overtaken", async () => {
    const answers: ((summary: PullRequestSummary) => void)[] = [];
    core.pullRequestSummary.mockImplementation(
      () => new Promise((resolve) => answers.push(resolve)),
    );
    const { polled } = show();
    const moved = pullRequest(8, {
      ...listed,
      details: { ...listed.details, updatedAt: "2026-10-02T11:00:00Z" },
    });
    polled(moved);
    await waitFor(() => expect(answers).toHaveLength(2));
    // The second question is answered first, then the first one turns up late.
    answers[1]!(pullRequestSummary(moved, { body: "The newer answer." }));
    await waitFor(() => expect(screen.getByText("The newer answer.")).toBeVisible());
    answers[0]!(pullRequestSummary(listed, { body: "The older answer." }));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.getByText("The newer answer.")).toBeVisible();
    expect(screen.queryByText("The older answer.")).not.toBeInTheDocument();
  });
});
