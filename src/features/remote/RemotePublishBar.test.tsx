import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OtherPublishState, OtherPullRequest } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  otherHostPublishState: vi.fn(),
  otherHostCommit: vi.fn(),
  otherHostPush: vi.fn(),
  otherHostOpenPullRequest: vi.fn(),
  otherHostMergePullRequest: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
const opener = vi.hoisted(() => ({ openUrl: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => opener);
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));

import { RemotePublishBar } from "./RemotePublishBar";

const HEAD = "3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a";
const standing = (extra: Partial<OtherPublishState> = {}): OtherPublishState => ({
  branch: "ys/fix-login",
  base: "main",
  identity: "Ana Example <ana@example.com>",
  remote: true,
  pushed: false,
  ahead: 0,
  behind: 0,
  unpushed: [],
  pullRequest: null,
  canOpen: false,
  gh: true,
  problem: null,
  ...extra,
});
const request = (extra: Partial<OtherPullRequest> = {}): OtherPullRequest => ({
  number: 7,
  url: "https://github.com/acme/app/pull/7",
  title: "Fix the login page",
  state: "open",
  draft: false,
  checks: "passing",
  headOid: HEAD,
  conflicting: false,
  ...extra,
});
const changed = vi.fn();
const open = async (state: OtherPublishState, uncommitted = 2) => {
  core.otherHostPublishState.mockResolvedValue(state);
  const shown = render(
    <RemotePublishBar
      hostId="h-1"
      workspaceId="w-1"
      name="Build box"
      uncommitted={uncommitted}
      revision={1}
      onChanged={changed}
    />,
  );
  return { ...shown, bar: await screen.findByRole("group", { name: "Send on from Build box" }) };
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("committing and pushing on another computer", () => {
  it("commits what changed there with the message, once, and reads how it stands again", async () => {
    const { bar } = await open(standing());
    // The forge is asked when the bar opens; a reading after that asks Git alone.
    expect(core.otherHostPublishState).toHaveBeenCalledWith("h-1", "w-1", true);
    const commit = within(bar).getByRole("button", { name: "Commit" });
    expect(commit).toBeDisabled();
    const message = within(bar).getByRole("textbox", { name: "Commit message" });
    expect(message).toHaveAttribute("placeholder", "Commit 2 files on Build box");
    await userEvent.type(message, "Fix the spacing");
    let committed!: (state: OtherPublishState) => void;
    core.otherHostCommit.mockReturnValue(new Promise((resolve) => (committed = resolve)));
    core.otherHostPublishState.mockResolvedValue(
      standing({ ahead: 1, unpushed: ["Fix the spacing"] }),
    );
    await userEvent.click(commit);
    // Pressed again while it is on its way, it is not sent twice.
    expect(within(bar).getByRole("button", { name: "Committing…" })).toBeDisabled();
    await act(async () => committed(standing({ ahead: 1 })));
    expect(core.otherHostCommit).toHaveBeenCalledTimes(1);
    expect(core.otherHostCommit).toHaveBeenCalledWith("h-1", "w-1", "Fix the spacing");
    expect(message).toHaveValue("");
    expect(changed).toHaveBeenCalled();
    expect(await within(bar).findByRole("button", { name: "Push 1 commit" })).toBeEnabled();
  });

  it("has nothing to commit with no changes, and says when that computer has no Git identity", async () => {
    const { bar, unmount } = await open(standing(), 0);
    expect(within(bar).getByRole("textbox", { name: "Commit message" })).toBeDisabled();
    expect(within(bar).getByRole("button", { name: "Commit" })).toBeDisabled();
    unmount();
    const { bar: nobody } = await open(standing({ identity: null }));
    await userEvent.type(within(nobody).getByRole("textbox", { name: "Commit message" }), "Fix");
    expect(within(nobody).getByRole("button", { name: "Commit" })).toBeDisabled();
    expect(nobody).toHaveTextContent("Git has no name and email on Build box");
  });

  it("pushes the branch, and says why in that computer's words when the remote refuses", async () => {
    const { bar } = await open(standing({ ahead: 2, unpushed: ["Second", "First"] }));
    core.otherHostPush.mockRejectedValueOnce({
      code: "remote_refused",
      message: "The remote has commits this branch does not have, so it refused the push.",
    });
    await userEvent.click(within(bar).getByRole("button", { name: "Push 2 commits" }));
    expect(core.otherHostPush).toHaveBeenCalledWith("h-1", "w-1");
    expect(await within(bar).findByRole("alert")).toHaveTextContent("refused the push");
    // Not tried again by itself: it is still there to press, and was sent once.
    expect(core.otherHostPush).toHaveBeenCalledTimes(1);
    core.otherHostPush.mockResolvedValueOnce(standing({ pushed: true }));
    core.otherHostPublishState.mockResolvedValue(standing({ pushed: true }));
    await userEvent.click(within(bar).getByRole("button", { name: "Push 2 commits" }));
    await vi.waitFor(() => expect(within(bar).queryByRole("button", { name: /^Push/ })).toBeNull());
    expect(within(bar).queryByRole("alert")).toBeNull();
  });

  it("says a lost answer may have been done, and reads how the workspace stands", async () => {
    const { bar } = await open(standing({ ahead: 1, unpushed: ["One"] }));
    core.otherHostPush.mockRejectedValueOnce({
      code: "remote_lost",
      message: "That computer's answer did not arrive, so it may have been done anyway.",
    });
    core.otherHostPublishState.mockResolvedValue(standing({ pushed: true }));
    await userEvent.click(within(bar).getByRole("button", { name: "Push 1 commit" }));
    expect(await within(bar).findByRole("alert")).toHaveTextContent("may have been done anyway");
    await vi.waitFor(() => expect(within(bar).queryByRole("button", { name: /^Push/ })).toBeNull());
    expect(core.otherHostPush).toHaveBeenCalledTimes(1);
  });
});

describe("a pull request on another computer", () => {
  it("opens one with a title and a description, by that computer's own sign-in", async () => {
    const { bar } = await open(
      standing({ pushed: true, canOpen: true, unpushed: ["Second", "Fix the login page"] }),
    );
    await userEvent.click(within(bar).getByRole("button", { name: "Open pull request" }));
    const form = within(bar).getByRole("form", { name: "Open pull request" });
    const title = within(form).getByRole("textbox", { name: "Title" });
    // The first commit of the branch is offered as its title.
    expect(title).toHaveValue("Fix the login page");
    await userEvent.type(within(form).getByRole("textbox", { name: "Description" }), "Why.");
    await userEvent.click(within(form).getByRole("checkbox", { name: "Open as a draft" }));
    core.otherHostOpenPullRequest.mockResolvedValue({
      url: "https://github.com/acme/app/pull/7",
      created: true,
    });
    core.otherHostPublishState.mockResolvedValue(
      standing({ pushed: true, pullRequest: request({ draft: true }) }),
    );
    await userEvent.click(within(form).getByRole("button", { name: "Open" }));
    expect(core.otherHostOpenPullRequest).toHaveBeenCalledWith(
      "h-1",
      "w-1",
      "Fix the login page",
      "Why.",
      true,
    );
    // The forge is asked again, and its pull request shown in place of the form.
    await vi.waitFor(() =>
      expect(core.otherHostPublishState).toHaveBeenLastCalledWith("h-1", "w-1", true),
    );
    const line = await within(bar).findByRole("group", { name: "Pull request #7" });
    expect(line).toHaveTextContent("draft · checks passing");
    expect(within(bar).queryByRole("form")).toBeNull();
    expect(within(bar).queryByRole("button", { name: "Open pull request" })).toBeNull();
    expect(opener.openUrl).not.toHaveBeenCalled();
  });

  it("opens the forge's own form here when that computer cannot open it itself", async () => {
    const { bar } = await open(standing({ pushed: true, canOpen: true, gh: false }));
    await userEvent.click(within(bar).getByRole("button", { name: "Open pull request" }));
    const form = within(bar).getByRole("form", { name: "Open pull request" });
    expect(form).toHaveTextContent("has no program to open it with");
    await userEvent.type(within(form).getByRole("textbox", { name: "Title" }), "Fix it");
    core.otherHostOpenPullRequest.mockResolvedValue({
      url: "https://github.com/acme/app/compare/main...ys/fix-login",
      created: false,
    });
    await userEvent.click(within(form).getByRole("button", { name: "Open" }));
    await vi.waitFor(() =>
      expect(opener.openUrl).toHaveBeenCalledWith(
        "https://github.com/acme/app/compare/main...ys/fix-login",
      ),
    );
    expect(bar).toHaveTextContent("the forge’s form was opened in your browser");
  });

  it("merges it only after a yes, at the commit shown, the way chosen", async () => {
    const { bar } = await open(standing({ pushed: true, pullRequest: request() }));
    const line = within(bar).getByRole("group", { name: "Pull request #7" });
    await userEvent.click(within(line).getByRole("button", { name: "#7 Fix the login page" }));
    expect(opener.openUrl).toHaveBeenCalledWith("https://github.com/acme/app/pull/7");
    // No: nothing is asked of that computer.
    native.confirm.mockResolvedValueOnce(false);
    await userEvent.click(within(line).getByRole("button", { name: "Merge…" }));
    await act(async () => {});
    expect(native.confirm.mock.calls[0]![0]).toContain("Merge pull request #7 into main?");
    expect(native.confirm.mock.calls[0]![0]).toContain("3f2a1b0");
    expect(core.otherHostMergePullRequest).not.toHaveBeenCalled();
    // Yes, as a rebase.
    await userEvent.selectOptions(
      within(line).getByRole("combobox", { name: "How to merge" }),
      "rebase",
    );
    native.confirm.mockResolvedValueOnce(true);
    core.otherHostMergePullRequest.mockResolvedValue(undefined);
    core.otherHostPublishState.mockResolvedValue(
      standing({ pushed: true, pullRequest: request({ state: "merged" }) }),
    );
    await userEvent.click(within(line).getByRole("button", { name: "Merge…" }));
    await vi.waitFor(() =>
      expect(core.otherHostMergePullRequest).toHaveBeenCalledWith("h-1", "w-1", 7, HEAD, "rebase"),
    );
    await vi.waitFor(() => expect(line).toHaveTextContent("merged"));
    expect(within(bar).queryByRole("button", { name: "Merge…" })).toBeNull();
  });

  it.each([
    ["a draft", request({ draft: true })],
    ["one with conflicts", request({ conflicting: true })],
    ["one whose head the forge did not say", request({ headOid: null })],
    ["a merged one", request({ state: "merged" })],
  ])("does not offer to merge %s", async (_what, pr) => {
    const { bar } = await open(standing({ pushed: true, pullRequest: pr }));
    expect(within(bar).getByRole("group", { name: "Pull request #7" })).toBeInTheDocument();
    expect(within(bar).queryByRole("button", { name: "Merge…" })).toBeNull();
  });

  it("does not offer to merge where that computer has no program for it, and opens no address that is not a web page", async () => {
    const { bar } = await open(
      standing({ pushed: true, gh: false, pullRequest: request({ url: null }) }),
    );
    expect(within(bar).queryByRole("button", { name: "Merge…" })).toBeNull();
    expect(within(bar).getByRole("button", { name: "#7 Fix the login page" })).toBeDisabled();
  });
});
