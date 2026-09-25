import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssistStatus, HarnessDef, HarnessInfo, SettingsInfo } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  harnessesList: vi.fn(),
  harnessSave: vi.fn(),
  harnessReset: vi.fn(),
  harnessPreview: vi.fn(),
  harnessTest: vi.fn(),
  settingsGet: vi.fn(),
  settingsSaveWorkspaces: vi.fn(),
  settingsSaveGeneral: vi.fn(),
  settingsSaveActivity: vi.fn(),
  activityDiagnostics: vi.fn(),
  activityClear: vi.fn(),
  assistStatus: vi.fn(),
  assistSaveKey: vi.fn(),
  assistForgetKey: vi.fn(),
  assistTestKey: vi.fn(),
  assistSaveSettings: vi.fn(),
  assistReview: vi.fn(),
  ptyClose: vi.fn(),
}));
const native = vi.hoisted(() => ({ pickFolder: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));
vi.mock("@/lib/native", () => ({ native }));
vi.mock("@/features/terminal/TerminalView", () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div>terminal for {sessionId}</div>,
}));

import { useAssistStore } from "@/stores/assist";
import { useHarnessStore } from "@/stores/harnesses";
import { SettingsDialog } from "./SettingsDialog";
import { useAppStore } from "@/stores/app";

const claude: HarnessInfo = {
  id: "claude",
  label: "Claude Code",
  command: "claude",
  baseArgs: [],
  modelArgs: ["--model", "{model}"],
  effortArgs: ["--effort", "{effort}"],
  sessionArgs: ["--session-id", "{session_id}"],
  promptArgs: ["{prompt}"],
  resumeArgs: ["--resume", "{session_id}"],
  forkArgs: ["--resume", "{session_id}", "--fork-session"],
  writeArgs: ["--print", "{prompt}"],
  efforts: ["low", "high"],
  models: ["opus"],
  promptTransport: "argv",
  sessionIdMode: "assigned",
  stdinReadyMs: 1500,
  enabled: true,
  resolvedPath: "/usr/bin/claude",
  builtin: true,
  modified: false,
};
const codex: HarnessInfo = {
  ...claude,
  id: "codex",
  label: "Codex",
  command: "codex",
  effortArgs: ["-c", 'model_reasoning_effort="{effort}"'],
  sessionArgs: [],
  sessionIdMode: "latestInCwd",
  resolvedPath: null,
};

const settings: SettingsInfo = {
  editorCommand: null,
  notifyWhenQuiet: true,
  checkForUpdates: true,
  workspaces: { worktreeRoot: null, branchPrefix: "ys" },
  defaultWorktreeRoot: "/home/me/yardsort",
  worktreeRootOverride: null,
  filePath: "/home/me/.config/yardsort/settings.toml",
  problem: null,
  activity: { recordLifecycle: true, showTimeline: false },
};

const assistStatus = (extra: Partial<AssistStatus> = {}): AssistStatus => ({
  keySource: "none",
  keyHint: null,
  problem: null,
  reviewChanges: false,
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

/** A key already saved in the credential store. */
const withKey = (extra: Partial<AssistStatus> = {}): AssistStatus =>
  assistStatus({ keySource: "keychain", keyHint: "…1234", ...extra });

const savedDef = () => core.harnessSave.mock.calls.at(-1)![0] as HarnessDef;

async function openSettings() {
  const onClose = vi.fn();
  render(<SettingsDialog onClose={onClose} />);
  await screen.findByRole("button", { name: /Codex/ });
  return { user: userEvent.setup(), onClose };
}

describe("Settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.activityDiagnostics.mockResolvedValue({
      events: 0,
      runs: 0,
      spoolDir: "/tmp/ys/activity/spool",
      spoolPending: 0,
      counters: [],
    });
    core.harnessesList.mockResolvedValue([claude, codex]);
    core.harnessPreview.mockImplementation(async (def: HarnessDef) => ({
      resolvedPath: def.command === "claude" ? "/usr/bin/claude" : null,
      start: [def.command, ...def.modelArgs, ...def.promptArgs],
      resume: [def.command, ...def.resumeArgs],
      fork: [def.command, ...def.forkArgs],
      problem: null,
    }));
    core.settingsGet.mockResolvedValue(settings);
    core.assistStatus.mockResolvedValue(assistStatus());
    useAssistStore.setState({ status: null });
    core.ptyClose.mockResolvedValue(undefined);
    useHarnessStore.setState({ harnesses: [], loaded: false });
  });

  describe("harnesses", () => {
    it("lists every harness with its status and shows the first one's definition", async () => {
      await openSettings();
      const list = screen.getByRole("navigation", { name: "Harnesses" });
      expect(
        within(list)
          .getAllByRole("button")
          .map((b) => b.textContent),
      ).toEqual(["Claude Code", "Codex", "Add custom harness"]);
      expect(screen.getByLabelText("Command")).toHaveValue("claude");
      expect(screen.getByLabelText("Fork args")).toHaveValue(
        "--resume {session_id} --fork-session",
      );
      expect(screen.getByLabelText("Id")).toBeDisabled();
      expect(await screen.findByText("Found at /usr/bin/claude")).toBeInTheDocument();
    });

    it("shows arguments with quotes the way they must be typed, and says when a command is missing", async () => {
      const { user } = await openSettings();
      await user.click(screen.getByRole("button", { name: /Codex/ }));
      expect(screen.getByLabelText("Effort args")).toHaveValue(
        `-c 'model_reasoning_effort="{effort}"'`,
      );
      expect(await screen.findByText("Not found on your PATH.")).toBeInTheDocument();
    });

    it("previews the exact command line as the definition is edited", async () => {
      const { user } = await openSettings();
      const preview = screen.getByRole("region", { name: "Command preview" });
      await within(preview).findByText("claude --model {model} {prompt}");

      const always = screen.getByLabelText("Always args");
      await user.type(always, "--verbose");
      await waitFor(() =>
        expect(core.harnessPreview).toHaveBeenLastCalledWith(
          expect.objectContaining({ baseArgs: ["--verbose"] }),
        ),
      );
    });

    it("saves edits as parsed arguments and lists", async () => {
      core.harnessSave.mockResolvedValue([{ ...claude, modified: true }, codex]);
      const { user } = await openSettings();
      const save = screen.getByRole("button", { name: "Save" });
      expect(save).toBeDisabled();

      const always = screen.getByLabelText("Always args");
      await user.type(always, `--append-system-prompt "be brief"`);
      const efforts = screen.getByLabelText("Effort levels");
      await user.clear(efforts);
      await user.type(efforts, "low, medium , max");
      await user.click(screen.getByLabelText(/Offer this harness/));
      await user.click(save);

      expect(savedDef()).toMatchObject({
        id: "claude",
        baseArgs: ["--append-system-prompt", "be brief"],
        efforts: ["low", "medium", "max"],
        enabled: false,
      });
      expect(savedDef()).not.toHaveProperty("resolvedPath");
      expect(
        await within(screen.getByRole("navigation")).findByText("modified"),
      ).toBeInTheDocument();
    });

    it("refuses to save while a quote is open, pointing at the field", async () => {
      const { user } = await openSettings();
      await user.type(screen.getByLabelText("Prompt args"), ` --msg "oops`);
      expect(await screen.findByText("A quote is left open.")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
      expect(screen.getByText(/Fix the arguments above/)).toBeInTheDocument();
    });

    it("shows why the core rejected a definition and keeps the edits", async () => {
      core.harnessSave.mockRejectedValue({
        code: "invalid_harness",
        message: "The command cannot be empty.",
      });
      const { user } = await openSettings();
      await user.clear(screen.getByLabelText("Command"));
      await user.click(screen.getByRole("button", { name: "Save" }));
      expect(await screen.findByText("The command cannot be empty.")).toBeInTheDocument();
      expect(screen.getByLabelText("Command")).toHaveValue("");
    });

    it("restores a modified built-in, and only a modified one", async () => {
      core.harnessesList.mockResolvedValue([
        { ...claude, command: "/opt/claude", modified: true },
        codex,
      ]);
      core.harnessReset.mockResolvedValue([claude, codex]);
      const { user } = await openSettings();
      expect(screen.getByLabelText("Command")).toHaveValue("/opt/claude");

      await user.click(screen.getByRole("button", { name: "Restore defaults" }));

      expect(core.harnessReset).toHaveBeenCalledWith("claude");
      await waitFor(() => expect(screen.getByLabelText("Command")).toHaveValue("claude"));
      expect(screen.getByRole("button", { name: "Restore defaults" })).toBeDisabled();
    });

    it("adds a custom harness, which can later be deleted", async () => {
      const aider: HarnessInfo = {
        ...codex,
        id: "aider",
        label: "Aider",
        command: "aider",
        modelArgs: [],
        effortArgs: [],
        promptArgs: ["--message", "{prompt}"],
        resumeArgs: [],
        forkArgs: [],
        writeArgs: [],
        efforts: [],
        models: [],
        builtin: false,
      };
      core.harnessSave.mockResolvedValue([claude, codex, aider]);
      core.harnessReset.mockResolvedValue([claude, codex]);
      const { user } = await openSettings();

      await user.click(screen.getByRole("button", { name: "Add custom harness" }));
      await user.type(screen.getByLabelText("Label"), "Aider");
      await user.type(screen.getByLabelText("Id"), "aider");
      await user.type(screen.getByLabelText("Command"), "aider");
      const prompt = screen.getByLabelText("Prompt args");
      await user.clear(prompt);
      await user.type(prompt, "--message {{prompt}");
      await user.click(screen.getByRole("button", { name: "Add harness" }));

      expect(savedDef()).toMatchObject({
        id: "aider",
        command: "aider",
        promptArgs: ["--message", "{prompt}"],
      });
      expect(await within(screen.getByRole("navigation")).findByText("custom")).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Delete harness" }));
      expect(core.harnessReset).toHaveBeenCalledWith("aider");
      await waitFor(() =>
        expect(screen.queryByRole("button", { name: /Aider/ })).not.toBeInTheDocument(),
      );
    });

    it("test-launches the definition as edited, and closes the session with the dialog", async () => {
      core.harnessTest.mockResolvedValue({ id: "test-session" });
      const { user } = await openSettings();
      await user.type(screen.getByLabelText("Always args"), "--debug");
      await user.click(screen.getByRole("button", { name: "Test launch" }));

      expect(await screen.findByText("terminal for test-session")).toBeInTheDocument();
      expect(core.harnessTest.mock.calls[0]![0]).toMatchObject({ baseArgs: ["--debug"] });
      expect(core.harnessSave).not.toHaveBeenCalled();

      await user.click(screen.getByRole("button", { name: "Close" }));
      expect(core.ptyClose).toHaveBeenCalledWith("test-session");
    });
  });

  describe("workspaces", () => {
    async function openWorkspaces() {
      const opened = await openSettings();
      await opened.user.click(screen.getByRole("tab", { name: "Workspaces" }));
      await screen.findByLabelText("Branch prefix");
      return opened;
    }

    it("shows the defaults and an example branch that follows the prefix", async () => {
      const { user } = await openWorkspaces();
      expect(screen.getByLabelText("Worktree folder")).toHaveAttribute(
        "placeholder",
        "/home/me/yardsort",
      );
      expect(screen.getByText("ys/fix-login-bug")).toBeInTheDocument();
      await user.clear(screen.getByLabelText("Branch prefix"));
      expect(screen.getByText("fix-login-bug")).toBeInTheDocument();
    });

    it("saves the folder and prefix", async () => {
      core.settingsSaveWorkspaces.mockImplementation(async (workspaces) => ({
        ...settings,
        workspaces,
      }));
      native.pickFolder.mockResolvedValue("/data/worktrees");
      const { user } = await openWorkspaces();

      await user.click(screen.getByRole("button", { name: "Browse…" }));
      const prefix = screen.getByLabelText("Branch prefix");
      await user.clear(prefix);
      await user.type(prefix, "joao");
      await user.click(screen.getByRole("button", { name: "Save" }));

      expect(core.settingsSaveWorkspaces).toHaveBeenCalledWith({
        worktreeRoot: "/data/worktrees",
        branchPrefix: "joao",
      });
      expect(await screen.findByText("Saved.")).toBeInTheDocument();
    });

    it("shows a rejected value's reason", async () => {
      core.settingsSaveWorkspaces.mockRejectedValue({
        code: "invalid_settings",
        message: '"has space" cannot be part of a git branch name.',
      });
      const { user } = await openWorkspaces();
      await user.type(screen.getByLabelText("Branch prefix"), " space");
      await user.click(screen.getByRole("button", { name: "Save" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "cannot be part of a git branch name",
      );
    });

    it("warns about an unreadable settings file and an environment override", async () => {
      core.settingsGet.mockResolvedValue({
        ...settings,
        problem: "settings.toml could not be read: expected `]`",
        worktreeRootOverride: "/tmp/ys-wt",
      });
      await openWorkspaces();
      expect(screen.getByRole("alert")).toHaveTextContent(
        /could not be read[\s\S]*settings\.toml\.unreadable/,
      );
      expect(
        screen.getByText(/Overridden by YARDSORT_WORKTREE_ROOT.*\/tmp\/ys-wt/),
      ).toBeInTheDocument();
    });
  });

  it("saves the editor command, and clearing it goes back to auto-detect", async () => {
    core.settingsSaveGeneral.mockImplementation(async (general) => ({ ...settings, ...general }));
    const { user } = await openSettings();
    await user.click(screen.getByRole("tab", { name: "General" }));
    const editor = await screen.findByLabelText("Editor command");
    expect(editor).toHaveAttribute("placeholder", "auto-detect");

    await user.type(editor, " zed ");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(core.settingsSaveGeneral).toHaveBeenLastCalledWith({
      editorCommand: "zed",
      notifyWhenQuiet: true,
      checkForUpdates: true,
    });

    await user.clear(editor);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(core.settingsSaveGeneral).toHaveBeenLastCalledWith(
      expect.objectContaining({ editorCommand: null }),
    );
  });

  it("turns finish notifications off, and tells the rest of the app", async () => {
    core.settingsSaveGeneral.mockImplementation(async (general) => ({ ...settings, ...general }));
    const { user } = await openSettings();
    await user.click(screen.getByRole("tab", { name: "General" }));
    const save = await screen.findByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    await user.click(screen.getByRole("checkbox", { name: /Notify me when an agent finishes/ }));
    await user.click(save);
    expect(core.settingsSaveGeneral).toHaveBeenLastCalledWith(
      expect.objectContaining({ notifyWhenQuiet: false, checkForUpdates: true }),
    );
    expect(useAppStore.getState().notifyWhenQuiet).toBe(false);
  });

  it("takes the keyboard when it opens and hands it back when it closes", async () => {
    // Opened by shortcut, focus is still in the terminal, which swallows every key.
    const terminal = document.createElement("textarea");
    document.body.append(terminal);
    terminal.focus();

    const { user } = await openSettings();
    expect(screen.getByRole("dialog", { name: "Settings" })).toHaveFocus();

    expect(user).toBeDefined();
    cleanup(); // closing unmounts the dialog
    expect(terminal).toHaveFocus();
    terminal.remove();
  });

  describe("assist", () => {
    const openAssist = async () => {
      const opened = await openSettings();
      await opened.user.click(screen.getByRole("tab", { name: "Assist" }));
      await screen.findByLabelText("TypeSafe API key");
      return opened;
    };

    it("takes a key, checks it with TypeSafe, and never shows it again", async () => {
      core.assistSaveKey.mockResolvedValue(withKey());
      const { user } = await openAssist();
      expect(screen.getByRole("checkbox", { name: /Check changed files/ })).toBeDisabled();

      const field = screen.getByLabelText("TypeSafe API key");
      await user.type(field, "ts-live-abcd1234");
      await user.click(screen.getByRole("button", { name: "Save" }));

      expect(core.assistSaveKey).toHaveBeenCalledWith("ts-live-abcd1234");
      expect(field).toHaveValue("");
      expect(await screen.findByText(/Saved in your system credential store/)).toHaveTextContent(
        "…1234",
      );
      expect(screen.getByRole("checkbox", { name: /Check changed files/ })).toBeEnabled();
    });

    it("explains a key TypeSafe will not take, and keeps the features off", async () => {
      core.assistSaveKey.mockRejectedValue({
        code: "assist_bad_key",
        message: "TypeSafe did not accept the API key.",
      });
      const { user } = await openAssist();
      await user.type(screen.getByLabelText("TypeSafe API key"), "nope");
      await user.click(screen.getByRole("button", { name: "Save" }));

      expect(await screen.findByRole("alert")).toHaveTextContent("did not accept the API key");
      expect(screen.getByRole("checkbox", { name: /Check changed files/ })).toBeDisabled();
    });

    it("switches each feature on by itself, and says what each one sends", async () => {
      core.assistStatus.mockResolvedValue(withKey());
      core.assistSaveSettings.mockResolvedValue(withKey({ reviewChanges: true }));
      const { user } = await openAssist();

      await user.click(screen.getByRole("checkbox", { name: /Check changed files/ }));
      expect(core.assistSaveSettings).toHaveBeenCalledWith(true, false, withKey().thresholds);
      expect(screen.getByText(/Sends the diff of each changed file/)).toBeInTheDocument();
      expect(screen.getByText(/Sends the message you are typing/)).toBeInTheDocument();
    });

    it("saves the thresholds and re-reads what Jev already answered", async () => {
      core.assistStatus.mockResolvedValue(withKey({ reviewChanges: true }));
      core.assistSaveSettings.mockResolvedValue(withKey({ reviewChanges: true }));
      core.assistReview.mockResolvedValue({
        files: [],
        task: null,
        model: "jev-1.13.0",
        problem: null,
      });
      useAssistStore.setState({ workspaceId: "w1", reviewing: false });
      const { user } = await openAssist();

      const field = screen.getByLabelText("Flag a risky change at (%)");
      expect(field).toHaveValue(70);
      await user.clear(field);
      await user.type(field, "40");
      await user.click(screen.getByRole("button", { name: "Save thresholds" }));

      expect(core.assistSaveSettings).toHaveBeenCalledWith(
        true,
        false,
        expect.objectContaining({ flagAtPercent: 40, offTaskAtPercent: 60 }),
      );
      // Re-reading is free: the badges are recomputed from answers already given.
      await waitFor(() => expect(core.assistReview).toHaveBeenCalledWith("w1"));
    });

    it("refuses a threshold the core will not take, and can go back to the defaults", async () => {
      core.assistStatus.mockResolvedValue(withKey({}));
      core.assistSaveSettings.mockRejectedValue({
        code: "invalid_thresholds",
        message: "Thresholds must be between 5% and 95%.",
      });
      const { user } = await openAssist();
      expect(screen.getByRole("button", { name: "Restore defaults" })).toBeDisabled();

      const field = screen.getByLabelText("Call a file off-task at (%)");
      await user.clear(field);
      await user.type(field, "99");
      await user.click(screen.getByRole("button", { name: "Save thresholds" }));
      expect(await screen.findByRole("alert")).toHaveTextContent("between 5% and 95%");
      expect(screen.getByRole("button", { name: "Restore defaults" })).toBeEnabled();
    });

    it("says when the credential store cannot be used", async () => {
      const problem = "The system credential store is not available. Set TYPESAFE_API_KEY.";
      core.assistStatus.mockResolvedValue(assistStatus({ keySource: "none", problem }));
      await openAssist();
      expect(await screen.findByRole("alert")).toHaveTextContent(problem);
    });
  });

  it("closes on Escape and on Done", async () => {
    const { user, onClose } = await openSettings();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
