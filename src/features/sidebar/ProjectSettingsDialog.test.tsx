import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { project } from "@/test/fixtures";
import { ProjectSettingsDialog } from "./ProjectSettingsDialog";

const core = vi.hoisted(() => ({
  projectAutomationGet: vi.fn(),
  projectAutomationSave: vi.fn(),
  testEnvGet: vi.fn(),
  testEnvSave: vi.fn(),
  testSettingsGet: vi.fn(),
  testSettingsSave: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
beforeEach(() => {
  vi.clearAllMocks();
  core.projectAutomationGet.mockResolvedValue({ copyFiles: [], setup: null, run: null });
  core.projectAutomationSave.mockResolvedValue(undefined);
  core.testEnvGet.mockResolvedValue([{ name: "TEST_PASSWORD", value: "hunter2" }]);
  core.testEnvSave.mockResolvedValue(undefined);
  core.testSettingsGet.mockResolvedValue({ askEveryTime: false, keepDays: 30 });
  core.testSettingsSave.mockResolvedValue(undefined);
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
it("keeps the smoke test environment with its values hidden, and what the project asks of runs", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  render(<ProjectSettingsDialog project={project("demo")} onClose={onClose} />);
  const password = await screen.findByLabelText("Variable 1 value");
  expect(password).toHaveAttribute("type", "password");
  expect(password).toHaveValue("hunter2");
  await user.click(screen.getByRole("button", { name: "Show values" }));
  expect(password).toHaveAttribute("type", "text");

  await user.click(screen.getByRole("button", { name: "+ Add a variable" }));
  await user.type(screen.getByLabelText("Variable 2 name"), "DATABASE_URL");
  await user.type(screen.getByLabelText("Variable 2 value"), "postgres://test");
  await user.click(screen.getByRole("checkbox", { name: "Ask every time" }));
  await user.clear(screen.getByRole("spinbutton", { name: "Keep runs for days" }));
  await user.type(screen.getByRole("spinbutton", { name: "Keep runs for days" }), "7");
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(core.testEnvSave).toHaveBeenCalledWith("p-demo", [
    { name: "TEST_PASSWORD", value: "hunter2" },
    { name: "DATABASE_URL", value: "postgres://test" },
  ]);
  expect(core.testSettingsSave).toHaveBeenCalledWith("p-demo", { askEveryTime: true, keepDays: 7 });
  expect(onClose).toHaveBeenCalledOnce();
});
it("a removed variable is gone when saved", async () => {
  const user = userEvent.setup();
  render(<ProjectSettingsDialog project={project("demo")} onClose={() => {}} />);
  await screen.findByLabelText("Variable 1 value");
  await user.click(screen.getByRole("button", { name: "Remove variable 1" }));
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(core.testEnvSave).toHaveBeenCalledWith("p-demo", []);
});
