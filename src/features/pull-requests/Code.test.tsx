import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileChange, FileDiff, PullRequest, PullRequestChanges } from "@/lib/ipc";
import { project, pullRequest, pullRequestsOf } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  pullRequestChanges: vi.fn(),
  pullRequestDiff: vi.fn(),
  uiStateSave: vi.fn(),
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@tauri-apps/plugin-opener", () => opener);
// CodeMirror needs a real layout. What matters here is what it is handed.
vi.mock("@/features/changes/CodeView", () => ({
  CodeView: (props: { path: string; text: string; original?: string; split?: boolean }) => (
    <pre data-testid="code" data-split={String(!!props.split)}>
      {`${props.path}\n--- ${props.original ?? ""}\n+++ ${props.text}`}
    </pre>
  ),
}));

import { useProjectsStore } from "@/stores/projects";
import { usePullRequestsStore } from "@/stores/pullRequests";
import { Code } from "./Code";
import { rowsOf } from "./rows";

const alpha = project("alpha");
const listed = pullRequest(8, { details: { headOid: "head-1" } });
const rowOf = (pr: PullRequest) => rowsOf([alpha], { [alpha.id]: pullRequestsOf([pr]) })[0]!;

const change = (path: string, extra: Partial<FileChange> = {}): FileChange => ({
  path,
  oldPath: null,
  kind: "modified",
  additions: 3,
  deletions: 1,
  ...extra,
});
const files = [
  change("src/upload.rs", { additions: 12, deletions: 4 }),
  change("docs/retry.md", {
    kind: "renamed",
    oldPath: "docs/again.md",
    additions: 0,
    deletions: 0,
  }),
  change("assets/logo.png", { kind: "added", additions: null, deletions: null }),
  change("src/old.rs", { kind: "deleted", additions: 0, deletions: 9 }),
];
const commits = { baseOid: "b".repeat(40), headOid: "a".repeat(40) };
const changes = (list = files, extra: Partial<PullRequestChanges> = {}): PullRequestChanges => ({
  files: list,
  ...commits,
  ...extra,
});
const text = (value: string) => ({ type: "text" as const, text: value });
const sides = (old: FileDiff["old"], next: FileDiff["new"]): FileDiff => ({ old, new: next });

function show(pr: PullRequest = listed) {
  const view = render(<Code row={rowOf(pr)} />);
  return {
    user: userEvent.setup(),
    /** What the minute's poll does: the same pull request, as a new object. */
    polled: (next: PullRequest) => view.rerender(<Code row={rowOf(next)} />),
  };
}
const fileRow = (name: RegExp) =>
  within(screen.getByRole("list", { name: "Changed files" })).getByRole("button", { name });

beforeEach(() => {
  vi.resetAllMocks();
  core.uiStateSave.mockResolvedValue(undefined);
  opener.openUrl.mockResolvedValue(undefined);
  core.pullRequestChanges.mockResolvedValue(changes());
  core.pullRequestDiff.mockResolvedValue(sides(text("old\n"), text("new\n")));
  useProjectsStore.setState({ ui: {} });
  usePullRequestsStore.setState({ selected: null, summaries: {}, changes: {}, tab: "code" });
});

describe("the files", () => {
  it("says the commits are being fetched, then lists what changed and how", async () => {
    show();
    expect(screen.getByText(/Fetching the pull request.s commits/)).toBeVisible();
    const list = await screen.findByRole("list", { name: "Changed files" });
    expect(core.pullRequestChanges).toHaveBeenCalledWith(alpha.id, 8);
    expect(
      within(list)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["Mupload.rssrc+12 −4", "Rretry.mddocs+0 −0", "Alogo.pngassets ", "Dold.rssrc+0 −9"]);
    // Where a renamed file came from is one hover away.
    expect(fileRow(/retry\.md/)).toHaveAttribute("title", "docs/again.md → docs/retry.md");
    expect(screen.getByText("4 files changed")).toHaveTextContent("+12 −13");
  });

  it("says so when there is nothing between the two commits", async () => {
    core.pullRequestChanges.mockResolvedValue(changes([]));
    show();
    expect(await screen.findByText(/No files differ/)).toBeVisible();
    expect(screen.getByText("0 files changed")).toBeVisible();
  });
});

describe("one file", () => {
  it("shows both sides, read between the two commits the list was read between", async () => {
    core.pullRequestDiff.mockResolvedValue(sides(text("retry once\n"), text("retry thrice\n")));
    const { user } = show();
    await user.click(await screen.findByRole("button", { name: /upload\.rs/ }));
    expect(await screen.findByTestId("code")).toHaveTextContent(
      "src/upload.rs --- retry once +++ retry thrice",
    );
    expect(core.pullRequestDiff).toHaveBeenCalledWith(alpha.id, commits, {
      path: "src/upload.rs",
      oldPath: null,
    });
    expect(screen.getByText("1 of 4")).toBeVisible();
  });

  it("reads a renamed file's old side from where it used to be", async () => {
    const { user } = show();
    await user.click(await screen.findByRole("button", { name: /retry\.md/ }));
    await waitFor(() =>
      expect(core.pullRequestDiff).toHaveBeenCalledWith(alpha.id, commits, {
        path: "docs/retry.md",
        oldPath: "docs/again.md",
      }),
    );
    expect(screen.getByText("docs/again.md → docs/retry.md")).toBeVisible();
  });

  it("moves between files with the dropdown and the arrows, and back to the list", async () => {
    const { user } = show();
    await user.click(await screen.findByRole("button", { name: /upload\.rs/ }));
    const previous = screen.getByRole("button", { name: "Previous file" });
    const next = screen.getByRole("button", { name: "Next file" });
    expect(previous).toBeDisabled();

    await user.click(next);
    expect(screen.getByRole("combobox", { name: "Files" })).toHaveValue("docs/retry.md");
    expect(screen.getByText("2 of 4")).toBeVisible();

    await user.selectOptions(screen.getByRole("combobox", { name: "Files" }), "src/old.rs");
    expect(screen.getByText("4 of 4")).toBeVisible();
    expect(next).toBeDisabled();
    await waitFor(() =>
      expect(core.pullRequestDiff).toHaveBeenLastCalledWith(alpha.id, commits, {
        path: "src/old.rs",
        oldPath: null,
      }),
    );
    await user.click(previous);
    expect(screen.getByText("3 of 4")).toBeVisible();

    await user.click(screen.getByRole("button", { name: /All files/ }));
    expect(screen.getByRole("list", { name: "Changed files" })).toBeVisible();
  });

  it("shows a deleted file as all removed, and says why a binary one is not shown", async () => {
    const { user } = show();
    core.pullRequestDiff.mockResolvedValue(sides(text("gone\n"), { type: "absent" }));
    await user.click(await screen.findByRole("button", { name: /old\.rs/ }));
    expect(await screen.findByTestId("code")).toHaveTextContent("src/old.rs --- gone +++");

    core.pullRequestDiff.mockResolvedValue(sides({ type: "absent" }, { type: "binary" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Files" }), "assets/logo.png");
    expect(await screen.findByText("Binary file — not shown.")).toBeVisible();
    expect(screen.queryByTestId("code")).not.toBeInTheDocument();
  });

  it("switches between inline and side by side, and remembers it with the changes panel's", async () => {
    const { user } = show();
    await user.click(await screen.findByRole("button", { name: /upload\.rs/ }));
    expect(await screen.findByTestId("code")).toHaveAttribute("data-split", "false");
    await user.click(screen.getByRole("button", { name: "Side by side" }));
    expect(screen.getByTestId("code")).toHaveAttribute("data-split", "true");
    expect(core.uiStateSave).toHaveBeenCalledWith("changes.diffMode", '"split"');
    expect(screen.getByRole("button", { name: "Inline" })).toBeVisible();
  });

  it("says what went wrong when a file cannot be read", async () => {
    core.pullRequestDiff.mockRejectedValue({ code: "git_failed", message: "bad object abc" });
    const { user } = show();
    await user.click(await screen.findByRole("button", { name: /upload\.rs/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("bad object abc");
    // The other files are still one step away.
    expect(screen.getByRole("combobox", { name: "Files" })).toBeEnabled();
  });
});

describe("fetching", () => {
  it("says what git said when the commits cannot be fetched, and offers the forge's diff", async () => {
    core.pullRequestChanges.mockRejectedValueOnce({
      code: "git_failed",
      message: "`git fetch` failed: Permission denied (publickey).",
    });
    const { user } = show();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "Could not fetch this pull request’s commits: `git fetch` failed: Permission denied (publickey).",
    );
    await user.click(within(alert).getByRole("button", { name: /See the diff on GitHub/ }));
    expect(opener.openUrl).toHaveBeenCalledWith(`${listed.url}/files`);

    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("list", { name: "Changed files" })).toBeVisible();
    expect(core.pullRequestChanges).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("fetches once for a head commit, and again only when the list says it moved", async () => {
    const { user, polled } = show();
    await user.click(await screen.findByRole("button", { name: /upload\.rs/ }));
    await screen.findByTestId("code");

    // The minute's poll, and a comment that moves everything but the head.
    polled(pullRequest(8, { details: { headOid: "head-1" } }));
    polled(pullRequest(8, { details: { headOid: "head-1", updatedAt: "2026-10-02T11:00:00Z" } }));
    expect(core.pullRequestChanges).toHaveBeenCalledTimes(1);

    // A push: the list is read again, and the file in view stays in view.
    const pushed = { baseOid: commits.baseOid, headOid: "c".repeat(40) };
    core.pullRequestChanges.mockResolvedValue(changes(files.slice(0, 2), pushed));
    polled(pullRequest(8, { details: { headOid: "head-2" } }));
    await waitFor(() => expect(core.pullRequestChanges).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByText("1 of 2")).toBeVisible());
    await waitFor(() =>
      expect(core.pullRequestDiff).toHaveBeenLastCalledWith(alpha.id, pushed, {
        path: "src/upload.rs",
        oldPath: null,
      }),
    );

    // Another push takes that file out of the diff: there is nothing of it left to look at.
    core.pullRequestChanges.mockResolvedValue(changes([files[1]!], pushed));
    polled(pullRequest(8, { details: { headOid: "head-3" } }));
    expect(await screen.findByRole("list", { name: "Changed files" })).toBeVisible();
    expect(screen.getByText("1 file changed")).toBeVisible();
  });
});
