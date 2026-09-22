import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssistStatus, ChangeSet, FileChange, FileReview } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  uiStateSave: vi.fn(),
  workspaceChanges: vi.fn(),
  workspaceDiff: vi.fn(),
  workspaceFile: vi.fn(),
  workspaceFiles: vi.fn(),
  workspaceWatch: vi.fn(),
  openInEditor: vi.fn(),
  onWorkspaceFilesChanged: vi.fn(),
  assistStatus: vi.fn(),
  assistReview: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
// CodeMirror needs a real layout engine; what it is *given* is what matters here.
vi.mock("./CodeView", () => ({
  CodeView: (props: { path: string; text: string; original?: string }) => (
    <pre data-testid="code">{JSON.stringify(props)}</pre>
  ),
}));

import { useAssistStore } from "@/stores/assist";
import { useChangesStore } from "@/stores/changes";
import { useProjectsStore } from "@/stores/projects";
import { ChangesPanel } from "./ChangesPanel";

const change = (path: string, extra: Partial<FileChange> = {}): FileChange => ({
  path,
  oldPath: null,
  kind: "modified",
  additions: 3,
  deletions: 1,
  ...extra,
});
const text = (value: string) => ({ type: "text" as const, text: value });
const shown = async () => {
  const { split, ...rest } = JSON.parse((await screen.findByTestId("code")).textContent!);
  return rest;
};
/** Whether the viewer is showing two panes; `split` is undefined for a plain file. */
const isSplit = async () =>
  JSON.parse((await screen.findByTestId("code")).textContent!).split === true;

let fileSystemChanged: (workspaceId: string) => void;

function setChanges(next: Partial<ChangeSet>) {
  core.workspaceChanges.mockResolvedValue({
    uncommitted: [],
    committed: [],
    base: "main",
    ...next,
  });
}

async function renderPanel() {
  render(<ChangesPanel />);
  await waitFor(() => expect(core.workspaceChanges).toHaveBeenCalled());
  return userEvent.setup();
}

describe("ChangesPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.workspaceWatch.mockResolvedValue(undefined);
    core.uiStateSave.mockResolvedValue(undefined);
    core.openInEditor.mockResolvedValue(undefined);
    core.onWorkspaceFilesChanged.mockImplementation((handler) => {
      fileSystemChanged = handler;
      return Promise.resolve(() => {});
    });
    setChanges({
      uncommitted: [
        change("src/app.ts"),
        change("notes.md", { kind: "untracked", additions: null, deletions: null }),
      ],
      committed: [change("src/old name.ts", { kind: "renamed", oldPath: "src/legacy.ts" })],
    });
    core.workspaceDiff.mockResolvedValue({ old: text("before"), new: text("after") });
    useProjectsStore.setState({ selectedWorkspaceId: "w1", composingProjectId: null, ui: {} });
    useChangesStore.setState({
      workspaceId: null,
      changes: null,
      viewing: null,
      diff: null,
      file: null,
      error: null,
    });
  });

  it("groups what is uncommitted and what the branch has committed, with counts", async () => {
    await renderPanel();
    const uncommitted = await screen.findByRole("region", { name: "Uncommitted" });
    expect(
      within(uncommitted)
        .getAllByRole("button")
        .map((b) => b.title),
    ).toEqual(["src/app.ts", "notes.md"]);
    expect(uncommitted).toHaveTextContent("+3");
    const committed = screen.getByRole("region", { name: "On this branch · vs main" });
    expect(within(committed).getByRole("button")).toHaveAttribute(
      "title",
      "src/legacy.ts → src/old name.ts",
    );
    expect(screen.getByRole("tab", { name: "Changes 3" })).toBeInTheDocument();
    expect(core.workspaceWatch).toHaveBeenCalledWith("w1");
  });

  it("opens a diff with both sides, and an added file as all new", async () => {
    const user = await renderPanel();
    await user.click(await screen.findByTitle("src/app.ts"));
    expect(await shown()).toEqual({ path: "src/app.ts", text: "after", original: "before" });
    expect(core.workspaceDiff).toHaveBeenCalledWith(
      "w1",
      expect.objectContaining({ path: "src/app.ts" }),
      "uncommitted",
    );

    core.workspaceDiff.mockResolvedValue({ old: { type: "absent" }, new: text("fresh") });
    await user.click(screen.getByTitle("notes.md"));
    await waitFor(async () =>
      expect(await shown()).toEqual({ path: "notes.md", text: "fresh", original: "" }),
    );
  });

  // A wide diff in a narrow panel is what an inline view is worst at, so the two versions can
  // sit side by side instead — and the choice is remembered.
  it("switches a diff between inline and side by side, and remembers which", async () => {
    const user = await renderPanel();
    await user.click(await screen.findByTitle("src/app.ts"));
    expect(await isSplit()).toBe(false);

    await user.click(screen.getByRole("button", { name: "Side by side" }));
    await waitFor(async () => expect(await isSplit()).toBe(true));
    expect(core.uiStateSave).toHaveBeenCalledWith("changes.diffMode", JSON.stringify("split"));

    await user.click(screen.getByRole("button", { name: "Inline" }));
    await waitFor(async () => expect(await isSplit()).toBe(false));
  });

  it("offers no side-by-side for a plain file, which has nothing to compare", async () => {
    core.workspaceFiles.mockResolvedValue([
      { name: "README.md", path: "README.md", isDir: false, ignored: false },
    ]);
    core.workspaceFile.mockResolvedValue(text("hello"));
    const user = await renderPanel();

    await user.click(screen.getByRole("tab", { name: "Files" }));
    await user.click(
      within(await screen.findByRole("treeitem", { name: "README.md" })).getByRole("button"),
    );
    await shown();
    expect(screen.queryByRole("button", { name: "Side by side" })).not.toBeInTheDocument();
  });

  it("updates the list and the open diff when the files change on disk", async () => {
    const user = await renderPanel();
    await user.click(await screen.findByTitle("src/app.ts"));
    await shown();

    setChanges({
      uncommitted: [
        change("src/app.ts", { additions: 9 }),
        change("brand-new.ts", { kind: "added" }),
      ],
    });
    core.workspaceDiff.mockResolvedValue({ old: text("before"), new: text("after, edited again") });
    act(() => fileSystemChanged("w1"));

    expect(await screen.findByTitle("brand-new.ts")).toBeInTheDocument();
    await waitFor(async () => expect((await shown()).text).toBe("after, edited again"));
    expect(screen.getByRole("region", { name: "Uncommitted" })).toHaveTextContent("+9");
  });

  it("ignores signals about other workspaces", async () => {
    await renderPanel();
    await screen.findByTitle("src/app.ts");
    const before = core.workspaceChanges.mock.calls.length;
    act(() => fileSystemChanged("some-other-workspace"));
    expect(core.workspaceChanges).toHaveBeenCalledTimes(before);
  });

  it("says why a file cannot be shown instead of showing garbage", async () => {
    const user = await renderPanel();
    core.workspaceDiff.mockResolvedValue({ old: { type: "absent" }, new: { type: "binary" } });
    await user.click(await screen.findByTitle("src/app.ts"));
    expect(await screen.findByText("Binary file — not shown.")).toBeInTheDocument();

    core.workspaceDiff.mockResolvedValue({
      old: text(""),
      new: { type: "tooLarge", bytes: 5_242_880 },
    });
    await user.click(screen.getByTitle("notes.md"));
    expect(await screen.findByText("File too large to show (5.0 MB).")).toBeInTheDocument();
  });

  it("browses files lazily and opens one read-only", async () => {
    core.workspaceFiles.mockImplementation(async (_id: string, dir: string) =>
      dir === ""
        ? [
            { name: "src", path: "src", isDir: true, ignored: false },
            { name: "README.md", path: "README.md", isDir: false, ignored: false },
          ]
        : [{ name: "main.rs", path: "src/main.rs", isDir: false, ignored: false }],
    );
    core.workspaceFile.mockResolvedValue(text("fn main() {}"));
    const user = await renderPanel();

    await user.click(screen.getByRole("tab", { name: "Files" }));
    await screen.findByRole("treeitem", { name: "README.md" });
    expect(core.workspaceFiles).toHaveBeenCalledTimes(1);

    await user.click(within(screen.getByRole("treeitem", { name: "src" })).getByRole("button"));
    await user.click(
      within(await screen.findByRole("treeitem", { name: "main.rs" })).getByRole("button"),
    );
    expect(core.workspaceFiles).toHaveBeenCalledWith("w1", "src", false);
    expect(await shown()).toEqual({ path: "src/main.rs", text: "fn main() {}" });
  });

  it("hides ignored files until asked, then shows them dimmed — contents included — and remembers", async () => {
    core.workspaceFiles.mockImplementation(
      async (_id: string, dir: string, showIgnored: boolean) => {
        if (dir === "node_modules")
          return [{ name: "react", path: "node_modules/react", isDir: true, ignored: false }];
        const tracked = [{ name: "README.md", path: "README.md", isDir: false, ignored: false }];
        return showIgnored
          ? [
              { name: ".git", path: ".git", isDir: true, ignored: true },
              { name: "node_modules", path: "node_modules", isDir: true, ignored: true },
              ...tracked,
            ]
          : tracked;
      },
    );
    const user = await renderPanel();
    await user.click(screen.getByRole("tab", { name: "Files" }));
    await screen.findByRole("treeitem", { name: "README.md" });
    expect(screen.queryByRole("treeitem", { name: "node_modules" })).not.toBeInTheDocument();

    const toggle = screen.getByRole("button", { name: "ignored" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    await user.click(toggle);

    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(core.uiStateSave).toHaveBeenCalledWith("files.showIgnored", "true");
    const modules = await screen.findByRole("treeitem", { name: "node_modules" });
    expect(within(modules).getByRole("button")).toHaveAttribute(
      "title",
      "node_modules — ignored by git",
    );
    expect(screen.getByRole("treeitem", { name: ".git" })).toBeInTheDocument();
    expect(
      within(screen.getByRole("treeitem", { name: "README.md" })).getByRole("button"),
    ).toHaveAttribute("title", "README.md");

    // What is inside an ignored folder is ignored too, even though the rules only name the folder.
    await user.click(within(modules).getByRole("button"));
    const react = await screen.findByRole("treeitem", { name: "react" });
    expect(within(react).getByRole("button")).toHaveAttribute(
      "title",
      "node_modules/react — ignored by git",
    );

    await user.click(toggle);
    await waitFor(() =>
      expect(screen.queryByRole("treeitem", { name: "node_modules" })).not.toBeInTheDocument(),
    );
  });

  it("opens the file in the editor — and the folder, for a deleted file", async () => {
    const user = await renderPanel();
    await user.click(await screen.findByTitle("src/app.ts"));
    await user.click(await screen.findByRole("button", { name: "Open in editor" }));
    expect(core.openInEditor).toHaveBeenLastCalledWith("w1", "src/app.ts");

    setChanges({ uncommitted: [change("gone.ts", { kind: "deleted" })] });
    act(() => fileSystemChanged("w1"));
    await user.click(await screen.findByTitle("gone.ts"));
    await user.click(await screen.findByRole("button", { name: "Open in editor" }));
    expect(core.openInEditor).toHaveBeenLastCalledWith("w1", null);
  });

  it("expands the viewer for comfortable reading and shrinks it back", async () => {
    const user = await renderPanel();
    await user.click(await screen.findByTitle("src/app.ts"));
    await user.click(await screen.findByRole("button", { name: "Expand" }));
    expect(screen.getByRole("dialog", { name: "src/app.ts" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Shrink" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close viewer" }));
    expect(screen.queryByTestId("code")).not.toBeInTheDocument();
  });

  it("stops watching while a workspace is being composed or none is selected", async () => {
    useProjectsStore.setState({ composingProjectId: "p1" });
    render(<ChangesPanel />);
    expect(await screen.findByText("Select a workspace to see its changes.")).toBeInTheDocument();
    expect(core.workspaceChanges).not.toHaveBeenCalled();
  });
});

describe("ChangesPanel with Assist", () => {
  const status = (extra: Partial<AssistStatus> = {}): AssistStatus => ({
    keySource: "keychain",
    keyHint: "…1234",
    problem: null,
    reviewChanges: true,
    suggestInComposer: false,
    thresholds: {
      flagAtPercent: 70,
      offTaskAtPercent: 60,
      suggestAtPercent: 50,
      defaults: [70, 60, 50],
      range: [5, 95],
    },
    model: "jev-1.13.0",
    ...extra,
  });
  const reviewed = (extra: Partial<FileReview>): FileReview => ({
    path: "src/app.ts",
    scope: "uncommitted",
    relevance: "direct",
    flags: [],
    notChecked: null,
    ...extra,
  });

  beforeEach(() => {
    vi.clearAllMocks();
    core.workspaceWatch.mockResolvedValue(undefined);
    core.onWorkspaceFilesChanged.mockImplementation(() => Promise.resolve(() => {}));
    core.assistStatus.mockResolvedValue(status());
    setChanges({
      uncommitted: [change("src/app.ts"), change("ci.yml"), change(".env")],
    });
    useProjectsStore.setState({ selectedWorkspaceId: "w1", composingProjectId: null, ui: {} });
    useChangesStore.setState({ workspaceId: null, changes: null, viewing: null, error: null });
    useAssistStore.setState({
      status: status(),
      workspaceId: "w1",
      reviewing: false,
      error: null,
      review: {
        task: "Fix the login redirect",
        model: "jev-1.13.0",
        problem: null,
        files: [
          reviewed({ path: "src/app.ts" }),
          reviewed({ path: "ci.yml", relevance: "unrelated", flags: ["disablesChecks"] }),
          reviewed({ path: ".env", relevance: null, flags: ["credentialsFile"] }),
        ],
      },
    });
  });

  it("badges the files Assist flagged and leaves the others alone", async () => {
    await renderPanel();
    const row = (path: string) => screen.getByTitle(path).textContent;

    expect(row("ci.yml")).toContain("off-task");
    expect(row("ci.yml")).toContain("checks");
    expect(row(".env")).toContain("credentials");
    expect(row("src/app.ts")).not.toContain("off-task");
    expect(screen.getByText(/Assist flagged 2 of 3 files/)).toBeInTheDocument();
  });

  it("checks again on demand, and says what stopped it", async () => {
    core.assistReview.mockResolvedValue({
      files: [],
      task: null,
      model: "jev-1.13.0",
      problem: null,
    });
    const user = await renderPanel();
    await user.click(screen.getByRole("button", { name: "Check now" }));
    expect(core.assistReview).toHaveBeenCalledWith("w1");

    useAssistStore.setState({ error: "TypeSafe is rate limiting this API key." });
    expect(await screen.findByRole("alert")).toHaveTextContent("rate limiting");
  });

  it("shows nothing at all while the check is switched off", async () => {
    core.assistStatus.mockResolvedValue(status({ reviewChanges: false }));
    useAssistStore.setState({ status: status({ reviewChanges: false }), review: null });
    await renderPanel();
    expect(screen.queryByRole("button", { name: "Check now" })).not.toBeInTheDocument();
    expect(screen.getByTitle("ci.yml").textContent).not.toContain("off-task");
  });
});
