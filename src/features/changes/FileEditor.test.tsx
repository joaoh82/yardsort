import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
const core = vi.hoisted(() => ({ workspaceSaveFile: vi.fn(), uiStateSave: vi.fn() }));
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@/lib/native", () => ({ native }));
vi.mock("./CodeView", () => ({
  CodeView: ({ text, onChange }: { text: string; onChange: (next: string) => void }) => (
    <textarea aria-label="File contents" value={text} onChange={(e) => onChange(e.target.value)} />
  ),
}));
import { FileEditor } from "./FileEditor";
import { useProjectsStore } from "@/stores/projects";
import { useChangesStore } from "@/stores/changes";
const refresh = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  core.workspaceSaveFile.mockResolvedValue(undefined);
  core.uiStateSave.mockResolvedValue(undefined);
  refresh.mockResolvedValue(undefined);
  useProjectsStore.setState({ ui: {} });
  useChangesStore.setState({
    workspaceId: "w1",
    viewing: { kind: "file", path: "app.ts" },
    refresh,
  });
});
async function edit(text = "edited") {
  fireEvent.change(await screen.findByRole("textbox", { name: "File contents" }), {
    target: { value: text },
  });
}
it("keeps drafts across remounts and disk refreshes, and saves against the original", async () => {
  const user = userEvent.setup();
  const view = render(<FileEditor workspaceId="w1" path="app.ts" text="old" />);
  await edit();
  await waitFor(() =>
    expect(core.uiStateSave).toHaveBeenCalledWith(
      'fileDraft:["w1","app.ts"]',
      JSON.stringify({ expected: "old", text: "edited" }),
    ),
  );
  view.unmount();
  render(<FileEditor workspaceId="w1" path="app.ts" text="agent edit" />);
  expect(await screen.findByRole("textbox")).toHaveValue("edited");
  expect(screen.getByRole("status")).toHaveTextContent("changed on disk");
  core.workspaceSaveFile.mockRejectedValueOnce(new Error("File changed"));
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(core.workspaceSaveFile).toHaveBeenCalledWith("w1", "app.ts", "old", "edited");
  expect(await screen.findByRole("alert")).toHaveTextContent("File changed");
  expect(screen.getByRole("textbox")).toHaveValue("edited");
});
it("respects cancelling discard and reloads only after confirmation", async () => {
  const user = userEvent.setup();
  render(<FileEditor workspaceId="w1" path="app.ts" text="old" />);
  await edit();
  native.confirm.mockResolvedValueOnce(false);
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByRole("textbox")).toHaveValue("edited");
  expect(refresh).not.toHaveBeenCalled();
  native.confirm.mockResolvedValueOnce(true);
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByRole("textbox")).toHaveValue("old");
  expect(refresh).toHaveBeenCalledOnce();
});
it("retains typing during a save with the saved text as its new baseline", async () => {
  let finish!: () => void;
  core.workspaceSaveFile.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const user = userEvent.setup();
  render(<FileEditor workspaceId="w1" path="app.ts" text="old" />);
  await edit();
  await user.click(screen.getByRole("button", { name: "Save" }));
  await edit("newer");
  await act(async () => finish());
  expect(screen.getByRole("textbox")).toHaveValue("newer");
  expect(JSON.parse(useProjectsStore.getState().ui['fileDraft:["w1","app.ts"]']!)).toEqual({
    expected: "edited",
    text: "newer",
  });
});
it("previews SVG and lets its source be edited", async () => {
  const user = userEvent.setup();
  render(
    <FileEditor
      workspaceId="w1"
      path="logo.svg"
      text='<svg xmlns="http://www.w3.org/2000/svg" />'
    />,
  );
  expect(screen.getByRole("img", { name: "logo.svg" })).toHaveAttribute(
    "src",
    expect.stringContaining("data:image/svg+xml"),
  );
  await user.click(screen.getByRole("button", { name: "Edit source" }));
  expect(await screen.findByRole("textbox")).toHaveValue(
    '<svg xmlns="http://www.w3.org/2000/svg" />',
  );
});

it("clears a saved draft so later disk updates appear", async () => {
  const user = userEvent.setup();
  const view = render(<FileEditor workspaceId="w1" path="app.ts" text="old" />);
  await edit();
  await user.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(refresh).toHaveBeenCalledOnce());
  expect(useProjectsStore.getState().ui['fileDraft:["w1","app.ts"]']).toBe("null");
  view.rerender(<FileEditor workspaceId="w1" path="app.ts" text="later disk edit" />);
  expect(screen.getByRole("textbox")).toHaveValue("later disk edit");
});

it("keeps a reopening draft without claiming a conflict while the read is pending", async () => {
  useProjectsStore.setState({
    ui: { 'fileDraft:["w1","app.ts"]': JSON.stringify({ expected: "old", text: "draft" }) },
  });
  const view = render(<FileEditor workspaceId="w1" path="app.ts" text={null} />);
  expect(await screen.findByRole("textbox")).toHaveValue("draft");
  expect(screen.getByRole("status")).toHaveTextContent("Loading the file from disk");
  expect(screen.queryByText(/file changed on disk/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  view.rerender(<FileEditor workspaceId="w1" path="app.ts" text="old" />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  view.rerender(<FileEditor workspaceId="w1" path="app.ts" text="external" />);
  expect(screen.getByRole("status")).toHaveTextContent("file changed on disk");
});
