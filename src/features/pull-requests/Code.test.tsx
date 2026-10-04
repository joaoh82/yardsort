import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileChange, FileDiff, LineComment, PullRequest, PullRequestChanges } from "@/lib/ipc";
import { project, pullRequest, pullRequestsOf, worktree } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  pullRequestChanges: vi.fn(),
  pullRequestDiff: vi.fn(),
  pullRequestLineComments: vi.fn(),
  pullRequestLineComment: vi.fn(),
  pullRequestNoteHelper: vi.fn(),
  pullRequestSendNote: vi.fn(),
  pullRequestNoteText: vi.fn(),
  pullRequestPrepareBranch: vi.fn(),
  projectPullRequests: vi.fn(),
  sessionsList: vi.fn(),
  uiStateSave: vi.fn(),
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@tauri-apps/plugin-opener", () => opener);
// CodeMirror needs a real layout (its own tests cover the notes and the selection). What
// matters here is what it is handed: the text, the notes, and a way to select lines.
vi.mock("@/features/changes/CodeView", () => ({
  CodeView: (props: {
    path: string;
    text: string;
    original?: string;
    split?: boolean;
    notes?: { key: string; line: number; side: string }[];
    renderNote?: (key: string) => React.ReactNode;
    onSelect?: (
      selection: { side: "old" | "new"; from: number; to: number; text: string } | null,
    ) => void;
  }) => (
    <div>
      <pre data-testid="code" data-split={String(!!props.split)}>
        {`${props.path}\n--- ${props.original ?? ""}\n+++ ${props.text}`}
      </pre>
      {props.notes?.map((note) => (
        <div key={note.key} data-note={`${note.side}:${note.line}`}>
          {props.renderNote?.(note.key)}
        </div>
      ))}
      <button
        type="button"
        onClick={() => props.onSelect?.({ side: "new", from: 3, to: 4, text: "retry\nonce" })}
      >
        select lines
      </button>
    </div>
  ),
}));

import { useProjectsStore } from "@/stores/projects";
import { usePullRequestsStore } from "@/stores/pullRequests";
import { useTerminalStore } from "@/stores/terminals";
import { Code } from "./Code";
import { rowsOf } from "./rows";

const alpha = project("alpha");
/** The same project with a workspace on the pull request's branch. */
const withWorkspace = {
  ...alpha,
  workspaces: [...alpha.workspaces, worktree("alpha", "branch-8")],
};
const now = Date.parse("2026-10-02T12:00:00Z");
const listed = pullRequest(8, { details: { headOid: "head-1" } });
const rowOf = (pr: PullRequest, inProject = alpha) =>
  rowsOf([inProject], { [inProject.id]: pullRequestsOf([pr]) })[0]!;
const lineComment = (id: string, extra: Partial<LineComment> = {}): LineComment => ({
  id,
  path: "src/upload.rs",
  line: 3,
  startLine: null,
  side: "right",
  originalLine: 3,
  outdated: false,
  wholeFile: false,
  author: "linus",
  at: Date.parse("2026-10-01T10:00:00Z") + Number(id) * 1000,
  body: `comment ${id}`,
  url: `https://github.com/demo/app/pull/8#discussion_r${id}`,
  inReplyTo: null,
  ...extra,
});

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

function show(pr: PullRequest = listed, inProject = alpha) {
  const view = render(<Code row={rowOf(pr, inProject)} now={now} />);
  return {
    user: userEvent.setup(),
    /** What the minute's poll does: the same pull request, as a new object. */
    polled: (next: PullRequest) => view.rerender(<Code row={rowOf(next, inProject)} now={now} />),
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
  core.pullRequestLineComments.mockResolvedValue([]);
  core.pullRequestLineComment.mockResolvedValue(undefined);
  core.projectPullRequests.mockResolvedValue(pullRequestsOf([listed]));
  core.sessionsList.mockResolvedValue([]);
  useProjectsStore.setState({
    projects: [withWorkspace],
    selectedWorkspaceId: null,
    pullRequestsOpen: true,
    composingProjectId: null,
    composingBranch: null,
    composingPrompt: null,
    collapsed: [],
  });
  useTerminalStore.setState({ tabs: [], active: {}, lastSize: { cols: 100, rows: 30 } });
  useProjectsStore.setState({ ui: {} });
  usePullRequestsStore.setState({
    selected: null,
    summaries: {},
    changes: {},
    lineComments: {},
    tab: "code",
  });
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

describe("comments on lines", () => {
  const comments = [
    lineComment("1"),
    lineComment("2", { inReplyTo: "1", author: "grace", body: "Agreed." }),
    lineComment("3", { line: 9, startLine: 7, side: "left", body: "On the old text." }),
    lineComment("4", { line: null, outdated: true, originalLine: 2, body: "Was here once." }),
    lineComment("5", { path: "docs/retry.md", line: 1 }),
  ];

  it("counts the threads on each file, and shows each beside its lines", async () => {
    core.pullRequestLineComments.mockResolvedValue(comments);
    const { user } = show();
    expect(await screen.findByLabelText("3 comment threads")).toHaveTextContent("3");
    expect(screen.getByLabelText("1 comment thread")).toBeVisible();
    expect(core.pullRequestLineComments).toHaveBeenCalledWith(alpha.id, 8);

    await user.click(screen.getByRole("button", { name: /upload\.rs/ }));
    const thread = await screen.findByRole("article", { name: "Comment on line 3" });
    expect(thread.closest("[data-note]")).toHaveAttribute("data-note", "new:3");
    expect(thread).toHaveTextContent("linus");
    // The body is Markdown, loaded lazily: until it comes the plain text stands in for it, and
    // a node found just before the swap is gone a moment later. Ask until the answer holds.
    await waitFor(() => expect(within(thread).getByText("comment 1")).toBeVisible());
    expect(thread).toHaveTextContent("Agreed.");
    const old = screen.getByRole("article", { name: "Comment on lines 7–9 of the old text" });
    expect(old.closest("[data-note]")).toHaveAttribute("data-note", "old:9");

    // The one the diff moved on from is listed, not placed.
    const aside = screen.getByText("1 comment the diff has moved on from");
    expect(aside).toHaveTextContent("outdated — was on line 2");
    expect(screen.getByText("Was here once.")).not.toBeVisible();
    await user.click(aside);
    expect(screen.getByText("Was here once.")).toBeVisible();
  });

  it("reads them again when the list says the pull request changed", async () => {
    const { polled } = show();
    await screen.findByRole("list", { name: "Changed files" });
    expect(core.pullRequestLineComments).toHaveBeenCalledTimes(1);
    polled(pullRequest(8, { details: { headOid: "head-1" } }));
    expect(core.pullRequestLineComments).toHaveBeenCalledTimes(1);
    polled(pullRequest(8, { details: { headOid: "head-1", updatedAt: "2026-10-02T11:00:00Z" } }));
    await waitFor(() => expect(core.pullRequestLineComments).toHaveBeenCalledTimes(2));
  });
});

describe("a note on selected lines", () => {
  /** The agent's terminal, as the core describes it once the note has gone to it. */
  const session = (id: string) => ({
    id,
    program: "claude",
    args: [],
    cwd: null,
    pid: 1,
    size: { cols: 80, rows: 24 },
    labels: { workspace: "w-alpha-branch-8", harness: "claude", record: "rec-1" },
    state: { status: "running" as const },
    hasOutput: false,
    busy: false,
    idleMs: 0,
  });
  const openNote = async (user: ReturnType<typeof userEvent.setup>) => {
    await user.click(await screen.findByRole("button", { name: /upload\.rs/ }));
    const button = screen.getByRole("button", { name: "Note on lines…" });
    expect(button).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "select lines" }));
    expect(button).toBeEnabled();
    await user.click(button);
    return screen.getByRole("dialog", { name: /Note on lines 3–4 of upload\.rs/ });
  };

  it("goes to the agent in the pull request's workspace, and says who and how first", async () => {
    core.pullRequestNoteHelper.mockResolvedValue({
      sessionId: "rec-1",
      harnessLabel: "Claude Code",
      title: "add the retry",
      reach: "type",
    });
    core.pullRequestSendNote.mockResolvedValue({
      reach: "type",
      session: session("s1"),
    });
    const { user } = show(listed, withWorkspace);
    const dialog = await openNote(user);
    expect(dialog).toHaveTextContent("retry once");
    await waitFor(() =>
      expect(core.pullRequestNoteHelper).toHaveBeenCalledWith("w-alpha-branch-8", 8),
    );
    expect(dialog).toHaveTextContent(
      "typed into Claude Code in “add the retry”, which is running and quiet in branch-8",
    );

    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    await waitFor(() =>
      expect(core.pullRequestSendNote).toHaveBeenCalledWith(
        "w-alpha-branch-8",
        8,
        "rec-1",
        { path: "src/upload.rs", side: "right", from: 3, to: 4, text: "retry\nonce" },
        "Fold these.",
        { cols: 100, rows: 30 },
      ),
    );
    // Nothing on GitHub unless the box is ticked.
    expect(core.pullRequestLineComment).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-alpha-branch-8");
  });

  it("also posts the note on GitHub when asked, on those lines of that head", async () => {
    core.pullRequestNoteHelper.mockResolvedValue({
      sessionId: "rec-1",
      harnessLabel: "Codex",
      title: "t",
      reach: "resume",
    });
    core.pullRequestSendNote.mockResolvedValue({
      reach: "resume",
      session: session("s2"),
    });
    const { user } = show(listed, withWorkspace);
    const dialog = await openNote(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    await waitFor(() =>
      expect(core.pullRequestLineComment).toHaveBeenCalledWith(
        alpha.id,
        8,
        { commit: "a".repeat(40), path: "src/upload.rs", side: "right", line: 4, startLine: 3 },
        "Fold these.",
      ),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(core.pullRequestSendNote).toHaveBeenCalledTimes(1);
    // GitHub first: it is the one that can refuse.
    expect(core.pullRequestLineComment.mock.invocationCallOrder[0]).toBeLessThan(
      core.pullRequestSendNote.mock.invocationCallOrder[0]!,
    );
  });

  it("starts a workspace with the note when the pull request has none", async () => {
    core.pullRequestNoteText.mockResolvedValue("Pull request #8 … Fold these.");
    core.pullRequestPrepareBranch.mockResolvedValue({
      branch: "ys/branch-8",
      behind: 0,
      fork: false,
    });
    const { user } = show();
    const dialog = await openNote(user);
    expect(dialog).toHaveTextContent("There is no workspace for this pull request yet.");
    expect(core.pullRequestNoteHelper).not.toHaveBeenCalled();
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    await user.click(within(dialog).getByRole("button", { name: "Start a workspace with it" }));
    await waitFor(() => expect(core.pullRequestPrepareBranch).toHaveBeenCalledWith(alpha.id, 8));
    expect(core.pullRequestNoteText).toHaveBeenCalledWith(
      alpha.id,
      8,
      { path: "src/upload.rs", side: "right", from: 3, to: 4, text: "retry\nonce" },
      "Fold these.",
    );
    const projects = useProjectsStore.getState();
    expect(projects.composingProjectId).toBe(alpha.id);
    expect(projects.composingBranch?.branch).toBe("ys/branch-8");
    expect(projects.composingPrompt).toBe("Pull request #8 … Fold these.");
  });

  it("can go to GitHub alone, and cancelling sends nothing anywhere", async () => {
    core.pullRequestNoteHelper.mockResolvedValue({
      sessionId: "rec-1",
      harnessLabel: "Claude Code",
      title: "t",
      reach: "type",
    });
    const { user } = show(listed, withWorkspace);
    let dialog = await openNote(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Just saying.");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(core.pullRequestSendNote).not.toHaveBeenCalled();
    expect(core.pullRequestLineComment).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Note on lines…" }));
    dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Just saying.");
    await user.click(within(dialog).getByRole("button", { name: "Post on GitHub only" }));
    await waitFor(() => expect(core.pullRequestLineComment).toHaveBeenCalledTimes(1));
    expect(core.pullRequestSendNote).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("posts on GitHub before the agent hears of it, so a refusal leaves nothing half done", async () => {
    core.pullRequestNoteHelper.mockResolvedValue({
      sessionId: "rec-1",
      harnessLabel: "Claude Code",
      title: "t",
      reach: "start",
    });
    core.pullRequestSendNote.mockResolvedValue({
      reach: "start",
      session: session("s3"),
    });
    // What GitHub says about a line outside the diff's hunks.
    core.pullRequestLineComment.mockRejectedValueOnce({
      code: "gh_failed",
      message: "HTTP 422: Validation Failed (line must be part of the diff)",
    });
    const { user } = show(listed, withWorkspace);
    const dialog = await openNote(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("part of the diff");
    expect(core.pullRequestLineComment).toHaveBeenCalledTimes(1);
    expect(core.pullRequestSendNote).not.toHaveBeenCalled();
    expect(dialog).toBeInTheDocument();

    // Sending again, now that GitHub accepts it: the agent hears of it once.
    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(core.pullRequestSendNote).toHaveBeenCalledTimes(1);
    expect(core.pullRequestLineComment).toHaveBeenCalledTimes(2);
  });

  it("does not post on GitHub twice when the agent could not be reached the first time", async () => {
    core.pullRequestNoteHelper.mockResolvedValue({
      sessionId: "rec-1",
      harnessLabel: "Claude Code",
      title: "t",
      reach: "type",
    });
    core.pullRequestSendNote
      .mockRejectedValueOnce({ code: "agent_busy", message: "Claude Code is working right now." })
      .mockResolvedValueOnce({
        reach: "type",
        session: session("s1"),
      });
    const { user } = show(listed, withWorkspace);
    const dialog = await openNote(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("working right now");
    expect(core.pullRequestLineComment).toHaveBeenCalledTimes(1);
    expect(dialog).toHaveTextContent("Posted on GitHub as a comment on these lines.");
    expect(within(dialog).getByRole("checkbox")).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Post on GitHub only" })).toBeDisabled();

    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(core.pullRequestSendNote).toHaveBeenCalledTimes(2);
    expect(core.pullRequestLineComment).toHaveBeenCalledTimes(1);
  });

  it("cannot be closed with Esc or the backdrop while a send is under way", async () => {
    core.pullRequestNoteHelper.mockResolvedValue({
      sessionId: "rec-1",
      harnessLabel: "Claude Code",
      title: "t",
      reach: "type",
    });
    let finish: (asked: unknown) => void = () => {};
    core.pullRequestSendNote.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const { user } = show(listed, withWorkspace);
    const dialog = await openNote(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    await user.click(within(dialog).getByRole("button", { name: "Send to agent" }));
    await waitFor(() => expect(core.pullRequestSendNote).toHaveBeenCalledTimes(1));
    expect(within(dialog).getByRole("button", { name: "Sending…" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.pointer({ keys: "[MouseLeft]", target: dialog.parentElement! });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();

    finish({ reach: "type", session: session("s1") });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("says when the agent cannot be asked, and does not pretend otherwise", async () => {
    core.pullRequestNoteHelper.mockRejectedValue({
      code: "agent_busy",
      message: "Claude Code is working right now. Ask again once it is quiet.",
    });
    const { user } = show(listed, withWorkspace);
    const dialog = await openNote(user);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("working right now");
    await user.type(within(dialog).getByRole("textbox", { name: "Note" }), "Fold these.");
    expect(within(dialog).getByRole("button", { name: "Send to agent" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Post on GitHub only" })).toBeEnabled();
  });
});
