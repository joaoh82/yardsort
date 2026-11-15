import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileChange, OtherHostView } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  uiStateSave: vi.fn(async () => {}),
  otherHostChanges: vi.fn(),
  otherHostFiles: vi.fn(),
  otherHostDiff: vi.fn(),
  otherHostFile: vi.fn(),
  otherHostPublishState: vi.fn(),
  otherHostCommit: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
// CodeMirror needs a real layout engine; what it is *given* is what matters here.
vi.mock("@/features/changes/CodeView", () => ({
  CodeView: (props: { path: string; text: string; original?: string }) => (
    <pre data-testid="code">{JSON.stringify(props)}</pre>
  ),
}));

import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";
import { RemoteChangesPanel } from "./RemoteChangesPanel";

const there: OtherHostView = {
  hostId: "h-1",
  name: "Build box",
  link: { kind: "online" },
  mayType: true,
  mayStart: true,
  mayResume: true,
  mayManage: true,
  mayLook: true,
  mayPublish: false,
  readAt: 1_000,
  platform: "Linux",
  appVersion: "0.35.0",
  projects: [],
  activity: [],
};
const change = (path: string, extra: Partial<FileChange> = {}): FileChange => ({
  path,
  oldPath: null,
  kind: "modified",
  additions: 3,
  deletions: 1,
  ...extra,
});
const text = (value: string) => ({ type: "text" as const, text: value });
const entry = (path: string, isDir = false) => ({
  name: path.slice(path.lastIndexOf("/") + 1),
  path,
  isDir,
  ignored: false,
});
const shown = async () => {
  const { split, ...rest } = JSON.parse((await screen.findByTestId("code")).textContent!);
  void split;
  return rest;
};
const showing = (view: OtherHostView = there) =>
  useOtherComputersStore.setState({ views: { "h-1": view } });
const open = (view: OtherHostView = there) => {
  showing(view);
  render(<RemoteChangesPanel hostId="h-1" workspaceId="w-1" />);
  return screen.getByRole("complementary", { name: "Changes" });
};

beforeEach(() => {
  vi.clearAllMocks();
  useOtherComputersStore.setState({ status: null, views: {} });
  useProjectsStore.setState({ ui: {} });
  core.otherHostChanges.mockResolvedValue({
    changes: {
      uncommitted: [change("src/login.tsx"), change("notes.txt", { kind: "untracked" })],
      committed: [change("src/auth/session.ts", { kind: "renamed", oldPath: "src/session.ts" })],
      base: "main",
    },
    more: false,
  });
  core.otherHostFiles.mockImplementation(async (_host, _workspace, dir: string) =>
    dir === ""
      ? { entries: [entry("src", true), entry("README.md")], more: false }
      : { entries: [entry("src/login.tsx")], more: true },
  );
  core.otherHostDiff.mockResolvedValue({ old: text("before\n"), new: text("after\n") });
  core.otherHostFile.mockResolvedValue(text("on its disk\n"));
});

describe("the changes of a workspace on another computer", () => {
  it("lists what changed there as a local workspace's are, and says where it was read from", async () => {
    const panel = open();
    expect(await within(panel).findByRole("tab", { name: "Changes 3" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(core.otherHostChanges).toHaveBeenCalledWith("h-1", "w-1");
    const uncommitted = within(panel).getByRole("region", { name: "Uncommitted" });
    expect(within(uncommitted).getByRole("button", { name: /login\.tsx/ })).toHaveTextContent(
      "+3 −1",
    );
    expect(within(uncommitted).getByTitle("Untracked")).toHaveTextContent("U");
    const committed = within(panel).getByRole("region", { name: "On this branch · vs main" });
    expect(within(committed).getByRole("button", { name: /session\.ts/ })).toHaveAttribute(
      "title",
      "src/session.ts → src/auth/session.ts",
    );
    expect(panel).toHaveTextContent("Read from Build box, and not kept here.");
    // Nothing here writes: no commit box, no editor, nothing to publish.
    expect(within(panel).queryByRole("textbox")).toBeNull();
    expect(
      within(panel).queryByRole("button", { name: /Commit|Push|Edit|pull request/i }),
    ).toBeNull();
  });

  it("opens both sides of a change, read from that computer, and the file itself from there", async () => {
    const panel = open();
    await userEvent.click(await within(panel).findByRole("button", { name: /session\.ts/ }));
    expect(core.otherHostDiff).toHaveBeenCalledWith(
      "h-1",
      "w-1",
      "src/auth/session.ts",
      "src/session.ts",
      "committed",
    );
    expect(await shown()).toMatchObject({ text: "after\n", original: "before\n" });
    const viewer = within(panel).getByRole("region", { name: "Viewing src/auth/session.ts" });
    await userEvent.click(within(viewer).getByRole("button", { name: "Open file" }));
    expect(core.otherHostFile).toHaveBeenCalledWith("h-1", "w-1", "src/auth/session.ts");
    expect(await shown()).toEqual({ path: "src/auth/session.ts", text: "on its disk\n" });
    // Read-only: the viewer has no way to change the file, here or in an editor.
    expect(within(viewer).queryByRole("button", { name: /editor|Edit|Save/i })).toBeNull();
    await userEvent.click(within(viewer).getByRole("button", { name: "Close viewer" }));
    expect(within(panel).queryByRole("region", { name: /^Viewing/ })).toBeNull();
  });

  it("says what cannot be shown, and what that computer refused", async () => {
    const panel = open();
    core.otherHostDiff.mockResolvedValueOnce({ old: { type: "binary" }, new: { type: "binary" } });
    await userEvent.click(await within(panel).findByRole("button", { name: /login\.tsx/ }));
    expect(await within(panel).findByText(/Binary file/)).toBeInTheDocument();
    core.otherHostDiff.mockRejectedValueOnce({
      code: "remote_refused",
      message: "That is not a path inside the workspace.",
    });
    await userEvent.click(within(panel).getByRole("button", { name: /notes\.txt/ }));
    expect(await within(panel).findByRole("alert")).toHaveTextContent("not a path inside");
  });

  it("says when that computer listed fewer files than changed, and when nothing changed", async () => {
    core.otherHostChanges.mockResolvedValueOnce({
      changes: { uncommitted: [change("a.txt")], committed: [], base: null },
      more: true,
    });
    const panel = open();
    expect(await within(panel).findByText(/More files changed than Build box lists/)).toBeVisible();
    expect(within(panel).queryByRole("region", { name: /On this branch/ })).toBeNull();
  });

  it("reads again when that computer was read again, what is open too", async () => {
    const panel = open();
    await userEvent.click(await within(panel).findByRole("button", { name: /login\.tsx/ }));
    await shown();
    expect(core.otherHostChanges).toHaveBeenCalledTimes(1);
    expect(core.otherHostDiff).toHaveBeenCalledTimes(1);
    core.otherHostChanges.mockResolvedValue({
      changes: { uncommitted: [change("src/login.tsx")], committed: [], base: "main" },
      more: false,
    });
    core.otherHostDiff.mockResolvedValue({ old: text("before\n"), new: text("after, again\n") });
    await act(async () => showing({ ...there, readAt: 2_000 }));
    expect(await within(panel).findByRole("tab", { name: "Changes 1" })).toBeInTheDocument();
    await vi.waitFor(async () => expect(await shown()).toMatchObject({ text: "after, again\n" }));
    expect(core.otherHostDiff).toHaveBeenCalledTimes(2);
  });
});

describe("sending on from a workspace on another computer", () => {
  const stands = {
    branch: "ys/fix-login",
    base: "main",
    identity: "Ana <ana@example.com>",
    remote: true,
    pushed: false,
    ahead: 0,
    behind: 0,
    unpushed: [],
    pullRequest: null,
    canOpen: false,
    gh: true,
    problem: null,
  };

  it("offers to commit where that computer takes it, and reads the list again once it has", async () => {
    core.otherHostPublishState.mockResolvedValue(stands);
    core.otherHostCommit.mockResolvedValue({ ...stands, ahead: 1 });
    const panel = open({ ...there, mayPublish: true });
    const bar = await within(panel).findByRole("group", { name: "Send on from Build box" });
    expect(panel).toHaveTextContent(
      "Commits, pushes and pull requests are made by Build box, as itself.",
    );
    await userEvent.type(within(bar).getByRole("textbox", { name: "Commit message" }), "Fix it");
    expect(core.otherHostChanges).toHaveBeenCalledTimes(1);
    await userEvent.click(within(bar).getByRole("button", { name: "Commit" }));
    expect(core.otherHostCommit).toHaveBeenCalledWith("h-1", "w-1", "Fix it");
    await vi.waitFor(() => expect(core.otherHostChanges).toHaveBeenCalledTimes(2));
    // On the Files tab there is nothing to commit from.
    await userEvent.click(within(panel).getByRole("tab", { name: "Files" }));
    expect(within(panel).queryByRole("group", { name: "Send on from Build box" })).toBeNull();
  });

  it("offers none of it where that computer's Yardsort cannot be asked", async () => {
    const panel = open();
    await within(panel).findByRole("tab", { name: "Changes 3" });
    expect(within(panel).queryByRole("group", { name: /Send on/ })).toBeNull();
    expect(core.otherHostPublishState).not.toHaveBeenCalled();
    expect(panel).toHaveTextContent("Nothing is changed from this panel.");
  });
});

describe("the files of a workspace on another computer", () => {
  it("lists a folder when it is opened, and opens a file read from that computer", async () => {
    const panel = open();
    await userEvent.click(within(panel).getByRole("tab", { name: "Files" }));
    const tree = await within(panel).findByRole("tree", { name: "Files" });
    expect(core.otherHostFiles).toHaveBeenCalledWith("h-1", "w-1", "");
    const folder = await within(tree).findByRole("treeitem", { name: "src" });
    expect(folder).toHaveAttribute("aria-expanded", "false");
    // Nothing below it is asked for until it is opened.
    expect(core.otherHostFiles).toHaveBeenCalledTimes(1);
    await userEvent.click(within(folder).getByRole("button", { name: "src" }));
    expect(core.otherHostFiles).toHaveBeenLastCalledWith("h-1", "w-1", "src");
    const file = await within(folder).findByRole("treeitem", { name: "login.tsx" });
    expect(folder).toHaveTextContent("more than are listed");
    await userEvent.click(within(file).getByRole("button"));
    expect(core.otherHostFile).toHaveBeenCalledWith("h-1", "w-1", "src/login.tsx");
    expect(await shown()).toEqual({ path: "src/login.tsx", text: "on its disk\n" });
    // The list was laid out again beside the viewer: its folder is still open, its file is the
    // one selected, and that computer was not asked for either folder again.
    const still = within(panel).getByRole("treeitem", { name: "src" });
    expect(still).toHaveAttribute("aria-expanded", "true");
    expect(within(still).getByRole("treeitem", { name: "login.tsx" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(core.otherHostFiles).toHaveBeenCalledTimes(2);
  });

  it("says a file is too large, not text, or gone, in place of it", async () => {
    const panel = open();
    await userEvent.click(within(panel).getByRole("tab", { name: "Files" }));
    await within(panel).findByRole("treeitem", { name: "README.md" });
    for (const [content, said] of [
      [{ type: "tooLarge", bytes: 5_242_880 }, /too large to show \(5\.0 MB\)/],
      [{ type: "binary" }, /Binary file/],
      [{ type: "absent" }, /no longer exists/],
      [
        { type: "notEditable", reason: "That computer does not show this kind of file." },
        /does not show/,
      ],
    ] as const) {
      core.otherHostFile.mockResolvedValueOnce(content);
      const readme = within(panel).getByRole("treeitem", { name: "README.md" });
      await userEvent.click(within(readme).getByRole("button"));
      expect(await within(panel).findByText(said)).toBeInTheDocument();
      await userEvent.click(within(panel).getByRole("button", { name: "Close viewer" }));
    }
  });
});

describe("where another computer's changes and files are not read", () => {
  it.each([
    [
      { mayType: false, mayLook: false },
      "lets this computer watch its terminals, not read its files",
    ],
    [{ mayLook: false }, "Yardsort is older than this one"],
    [
      { link: { kind: "offline" as const, since: null, why: "It is not connected" } },
      "They are read from it when it is back",
    ],
    [{ readAt: null }, "has not read Build box yet"],
  ])("keeps its place and says why, asking nothing: %o", (change, said) => {
    const panel = open({ ...there, ...change });
    expect(panel).toHaveTextContent("Changes and files stay on Build box.");
    expect(panel).toHaveTextContent(said);
    expect(within(panel).queryByRole("tab")).toBeNull();
    expect(core.otherHostChanges).not.toHaveBeenCalled();
    expect(core.otherHostFiles).not.toHaveBeenCalled();
  });
});
