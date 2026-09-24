import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { project } from "@/test/fixtures";
import { ProjectSettingsDialog } from "./ProjectSettingsDialog";

const core = vi.hoisted(() => ({ projectAutomationGet: vi.fn(), projectAutomationSave: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
beforeEach(() => {
  vi.clearAllMocks();
  core.projectAutomationGet.mockResolvedValue({ copyFiles: [], setup: null, run: null });
  core.projectAutomationSave.mockResolvedValue(undefined);
});
it("saves per-project files and literal command arguments", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  render(<ProjectSettingsDialog project={project("demo")} onClose={onClose} />);
  await waitFor(() => expect(screen.getByLabelText("Setup executable")).toBeEnabled());
  await user.type(screen.getByLabelText("Files to copy"), ".env\nconfig/local.json");
  await user.type(screen.getByLabelText("Setup executable"), "bash");
  await user.type(screen.getByLabelText("Setup arguments"), "scripts/setup file.sh\n\n$literal\n");
  await user.type(screen.getByLabelText("Run executable"), "bun");
  await user.type(screen.getByLabelText("Run arguments"), "run\n  \ndev\n");
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(core.projectAutomationSave).toHaveBeenCalledWith("p-demo", {
    copyFiles: [".env", "config/local.json"],
    setup: { program: "bash", args: ["scripts/setup file.sh", "$literal"] },
    run: { program: "bun", args: ["run", "dev"] },
  });
  expect(onClose).toHaveBeenCalledOnce();
});
it("cancel leaves saved settings alone and errors keep edits visible", async () => {
  const user = userEvent.setup();
  core.projectAutomationSave.mockRejectedValue({ code: "invalid", message: "Unsafe path" });
  const onClose = vi.fn();
  render(<ProjectSettingsDialog project={project("demo")} onClose={onClose} />);
  await waitFor(() => expect(screen.getByLabelText("Files to copy")).toBeEnabled());
  await user.type(screen.getByLabelText("Files to copy"), "../secret");
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Unsafe path");
  expect(screen.getByLabelText("Files to copy")).toHaveValue("../secret");
  expect(onClose).not.toHaveBeenCalled();
  core.projectAutomationSave.mockClear();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(core.projectAutomationSave).not.toHaveBeenCalled();
  expect(onClose).toHaveBeenCalledOnce();
});
