import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssistStatus } from "@/lib/ipc";
import { harness, project, worktree } from "@/test/fixtures";

type DragHandler = (event: { payload: unknown }) => void;
const drag = vi.hoisted(() => ({
  handler: null as DragHandler | null,
  unlisten: vi.fn(),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: DragHandler) => {
      drag.handler = handler;
      return Promise.resolve(drag.unlisten);
    },
  }),
}));

const core = vi.hoisted(() => ({
  harnessesList: vi.fn(),
  projectBranches: vi.fn(),
  workspaceCreate: vi.fn(),
  uiStateSave: vi.fn(),
  assistSuggest: vi.fn(),
  projectUntrackedWorktrees: vi.fn(),
  workspacesImport: vi.fn(),
  ptySpawn: vi.fn(),
  memoryGet: vi.fn(),
  outcomesAgents: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useAssistStore } from "@/stores/assist";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";
import { Composer } from "./Composer";

const assistStatus = (extra: Partial<AssistStatus> = {}): AssistStatus => ({
  keySource: "keychain",
  keyHint: "…1234",
  problem: null,
  reviewChanges: true,
  suggestInComposer: true,
  sendProvenance: false,
  checkMemory: false,
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

const session = (workspace: string) => ({
  id: "s1",
  program: "/usr/bin/claude",
  args: [],
  cwd: null,
  pid: 1,
  size: { cols: 80, rows: 24 },
  labels: { workspace, harness: "claude" },
  state: { status: "running" as const },
  hasOutput: true,
  idleMs: 0,
});

const app = project("app");

async function renderComposer() {
  render(<Composer project={app} />);
  await screen.findAllByRole("option", { name: "main" });
  return userEvent.setup();
}

describe("Composer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    drag.handler = null;
    core.uiStateSave.mockResolvedValue(undefined);
    core.projectBranches.mockResolvedValue({
      branches: ["develop", "main", "ys/kept-earlier", "ys/in-use"],
      default: "main",
      checkedOut: ["main", "ys/in-use"],
    });
    core.harnessesList.mockResolvedValue([
      harness("claude", { efforts: ["low", "high"], models: ["opus", "sonnet"] }),
      harness("codex", { efforts: ["low", "medium"] }),
      harness("opencode"),
      harness("grok", { resolvedPath: null }),
    ]);
    core.outcomesAgents.mockResolvedValue([]);
    core.memoryGet.mockResolvedValue({
      projectId: "p-app",
      shared: false,
      preview: null,
      entries: [],
    });
    useHarnessStore.setState({ harnesses: [], loaded: false });
    useProjectsStore.setState({
      projects: [app],
      selectedWorkspaceId: "w-app",
      composingProjectId: "p-app",
      composingBranch: null,
      ui: {},
    });
    useTerminalStore.setState({ tabs: [], active: {}, lastSize: { cols: 100, rows: 30 } });
  });

  it("starts a workspace with the message on Enter and lands in its terminal", async () => {
    const created = worktree("app", "fix-login");
    core.workspaceCreate.mockResolvedValue({ workspace: created, session: session(created.id) });
    const user = await renderComposer();

    await user.type(screen.getByRole("textbox", { name: /work on/ }), "Fix the login bug{Enter}");

    expect(core.workspaceCreate).toHaveBeenCalledWith({
      projectId: "p-app",
      baseBranch: "main",
      existingBranch: null,
      harness: { id: "claude", model: null, effort: null, prompt: "Fix the login bug" },
      size: { cols: 100, rows: 30 },
    });
    const projects = useProjectsStore.getState();
    expect(projects.projects[0]!.workspaces.map((w) => w.name)).toEqual(["local", "fix-login"]);
    expect(projects.selectedWorkspaceId).toBe(created.id);
    expect(projects.composingProjectId).toBeNull();
    expect(useTerminalStore.getState().active[created.id]).toBe("s1");
  });

  it("passes the chosen harness, model, effort and base branch", async () => {
    const created = worktree("app", "x");
    core.workspaceCreate.mockResolvedValue({ workspace: created, session: session(created.id) });
    const user = await renderComposer();

    await user.selectOptions(screen.getByRole("combobox", { name: "Harness" }), "codex");
    await user.type(screen.getByRole("combobox", { name: "Model" }), " gpt-next ");
    await user.selectOptions(screen.getByRole("combobox", { name: "Effort" }), "medium");
    await user.selectOptions(screen.getByRole("combobox", { name: "Branch" }), "new:develop");
    await user.click(screen.getByRole("button", { name: "Start" }));

    expect(core.workspaceCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        baseBranch: "develop",
        harness: { id: "codex", model: "gpt-next", effort: "medium", prompt: null },
      }),
    );
    expect(JSON.parse(core.uiStateSave.mock.calls.at(-1)![1])).toEqual({
      harness: "codex",
      model: "gpt-next",
      effort: "medium",
    });
  });

  it("opens an existing branch instead of creating one, offering only branches nobody has checked out", async () => {
    const created = worktree("app", "kept-earlier");
    core.workspaceCreate.mockResolvedValue({ workspace: created, session: session(created.id) });
    const user = await renderComposer();

    const group = screen.getByRole("group", { name: "Open existing branch" });
    expect(
      within(group)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["develop", "ys/kept-earlier"]);

    await user.selectOptions(
      screen.getByRole("combobox", { name: "Branch" }),
      "open:ys/kept-earlier",
    );
    expect(screen.getByText(/Enter to start/)).toHaveTextContent(
      'Opens the existing branch "ys/kept-earlier"',
    );
    await user.click(screen.getByRole("button", { name: "Start" }));

    expect(core.workspaceCreate).toHaveBeenCalledWith(
      expect.objectContaining({ baseBranch: null, existingBranch: "ys/kept-earlier" }),
    );
  });

  it("opens on a pull request's branch when started from one, and says what it is", async () => {
    const created = worktree("app", "kept-earlier");
    core.workspaceCreate.mockResolvedValue({ workspace: created, session: session(created.id) });
    useProjectsStore.setState({
      composingBranch: { branch: "ys/kept-earlier", behind: 2, fork: true },
    });
    const user = await renderComposer();

    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Branch" })).toHaveValue("open:ys/kept-earlier"),
    );
    const hint = screen.getByText(/Enter to start/);
    expect(hint).toHaveTextContent('Opens the existing branch "ys/kept-earlier"');
    expect(hint).toHaveTextContent("a pull request from a fork");
    expect(hint).toHaveTextContent("Yardsort will not push it");
    expect(hint).toHaveTextContent("is 2 commits behind the pull request. It is opened as it is.");

    await user.click(screen.getByRole("button", { name: "Start" }));
    expect(core.workspaceCreate).toHaveBeenCalledWith(
      expect.objectContaining({ baseBranch: null, existingBranch: "ys/kept-earlier" }),
    );

    // Another branch chosen by hand is not the pull request's, and is not described as one.
    await user.selectOptions(screen.getByRole("combobox", { name: "Branch" }), "open:develop");
    expect(screen.getByText(/Enter to start/)).not.toHaveTextContent("pull request");
  });

  it("starts from the default branch when the pull request's branch was checked out meanwhile", async () => {
    useProjectsStore.setState({ composingBranch: { branch: "ys/in-use", behind: 0, fork: false } });
    await renderComposer();
    // Git allows a branch one place: the list does not offer it, so it is not chosen either.
    expect(screen.getByRole("combobox", { name: "Branch" })).toHaveValue("new:main");
  });

  it("Shift+Enter breaks the line instead of sending", async () => {
    const user = await renderComposer();
    const box = screen.getByRole("textbox", { name: /work on/ });
    await user.type(box, "line one{Shift>}{Enter}{/Shift}line two");
    expect(box).toHaveValue("line one\nline two");
    expect(core.workspaceCreate).not.toHaveBeenCalled();
  });

  it("hides effort for harnesses without it and forgets a model when the harness changes", async () => {
    const user = await renderComposer();
    await user.type(screen.getByRole("combobox", { name: "Model" }), "opus");
    expect(screen.getByRole("combobox", { name: "Effort" })).toBeInTheDocument();

    await user.selectOptions(screen.getByRole("combobox", { name: "Harness" }), "opencode");
    expect(screen.queryByRole("combobox", { name: "Effort" })).not.toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Model" })).toHaveValue("");
  });

  it("offers harnesses that are not installed as disabled, and recalls the last picks", async () => {
    useProjectsStore.setState({
      ui: {
        "composer.last.p-app": JSON.stringify({ harness: "codex", model: "m", effort: "low" }),
      },
    });
    await renderComposer();
    expect(screen.getByRole("option", { name: /GROK — not installed/ })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Harness" })).toHaveValue("codex");
    expect(screen.getByRole("combobox", { name: "Model" })).toHaveValue("m");
    expect(screen.getByRole("combobox", { name: "Effort" })).toHaveValue("low");
  });

  it("does not offer harnesses that are switched off", async () => {
    core.harnessesList.mockResolvedValue([harness("claude", { enabled: false }), harness("codex")]);
    await renderComposer();
    const picker = screen.getByRole("combobox", { name: "Harness" });
    expect(
      within(picker)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["CODEX"]);
  });

  it("keeps the message and explains when starting fails", async () => {
    core.workspaceCreate.mockRejectedValue({
      code: "no_commits",
      message: "This repository has no commits yet.",
    });
    const user = await renderComposer();
    const box = screen.getByRole("textbox", { name: /work on/ });
    await user.type(box, "important thoughts{Enter}");

    expect(await screen.findByRole("alert")).toHaveTextContent("no commits yet");
    expect(box).toHaveValue("important thoughts");
    expect(box).toBeEnabled();
    expect(useProjectsStore.getState().composingProjectId).toBe("p-app");
  });

  it("cannot start without an installed harness, and says why", async () => {
    core.harnessesList.mockResolvedValue([harness("claude", { resolvedPath: null })]);
    await renderComposer();
    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent(/No enabled harness was found/);
  });

  it("Escape cancels composing", async () => {
    const user = await renderComposer();
    await user.type(screen.getByRole("textbox", { name: /work on/ }), "{Escape}");
    expect(useProjectsStore.getState().composingProjectId).toBeNull();
  });

  it("drops a file into the message and starts the workspace with that path", async () => {
    const created = worktree("app", "notes");
    core.workspaceCreate.mockResolvedValue({ workspace: created, session: session(created.id) });
    const { container } = render(<Composer project={app} />);
    await screen.findAllByRole("option", { name: "main" });
    await vi.waitFor(() => expect(drag.handler).not.toBeNull());
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
      right: 800,
      bottom: 600,
      width: 800,
      height: 600,
      x: 0,
      y: 0,
      toJSON() {},
    });
    const panel = container.firstElementChild as HTMLElement;
    const box = screen.getByRole("textbox", { name: /work on/ }) as HTMLTextAreaElement;

    act(() => drag.handler!({ payload: { type: "over", position: { x: 100, y: 100 } } }));
    expect(panel.dataset.drop).toBe("over");
    act(() =>
      drag.handler!({
        payload: {
          type: "drop",
          paths: ["/home/me/My Docs/plan.md"],
          position: { x: 100, y: 100 },
        },
      }),
    );
    expect(panel.dataset.drop).toBeUndefined();
    expect(box).toHaveValue('"/home/me/My Docs/plan.md" ');
    expect(box).toHaveFocus();
    expect(box.selectionStart).toBe(box.value.length);

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Start" }));
    expect(core.workspaceCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        harness: expect.objectContaining({ prompt: '"/home/me/My Docs/plan.md"' }),
      }),
    );
  });

  it("ignores a drop outside the composer and one that arrives while starting", async () => {
    core.workspaceCreate.mockReturnValue(new Promise(() => {}));
    render(<Composer project={app} />);
    await screen.findAllByRole("option", { name: "main" });
    await vi.waitFor(() => expect(drag.handler).not.toBeNull());
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
      right: 800,
      bottom: 600,
      width: 800,
      height: 600,
      x: 0,
      y: 0,
      toJSON() {},
    });
    const box = screen.getByRole("textbox", { name: /work on/ });

    act(() =>
      drag.handler!({
        payload: { type: "drop", paths: ["/tmp/elsewhere.md"], position: { x: 900, y: 100 } },
      }),
    );
    expect(box).toHaveValue("");

    const user = userEvent.setup();
    await user.type(box, "keep this{Enter}");
    await screen.findByRole("button", { name: "Starting…" });
    act(() =>
      drag.handler!({
        payload: { type: "drop", paths: ["/tmp/too-late.md"], position: { x: 100, y: 100 } },
      }),
    );
    expect(box).toHaveValue("keep this");
  });

  it("stops listening when unmounted", async () => {
    const { unmount } = render(<Composer project={app} />);
    await vi.waitFor(() => expect(drag.handler).not.toBeNull());
    unmount();
    expect(drag.unlisten).toHaveBeenCalled();
  });
});

describe("Composer with Assist", () => {
  const suggestion = (harnessId: string | null, efforts: Record<string, string> = {}) => ({
    harnessId,
    effortByHarness: efforts,
  });

  beforeEach(() => {
    vi.clearAllMocks();
    core.uiStateSave.mockResolvedValue(undefined);
    core.projectBranches.mockResolvedValue({ branches: ["main"], default: "main", checkedOut: [] });
    core.harnessesList.mockResolvedValue([
      harness("claude", { efforts: ["low", "medium", "high"] }),
      harness("codex", { efforts: ["low", "medium", "high"] }),
    ]);
    core.assistSuggest.mockResolvedValue(suggestion("codex", { codex: "high", claude: "high" }));
    useHarnessStore.setState({ harnesses: [], loaded: false });
    useAssistStore.setState({ status: assistStatus(), review: null, workspaceId: null });
    useProjectsStore.setState({
      projects: [app],
      selectedWorkspaceId: "w-app",
      composingProjectId: "p-app",
      composingBranch: null,
      ui: {},
    });
    useTerminalStore.setState({ tabs: [], active: {}, lastSize: { cols: 100, rows: 30 } });
  });

  it("offers a harness and an effort for the message, and applies them only when asked", async () => {
    const user = await renderComposer();
    await user.type(screen.getByRole("textbox", { name: /work on/ }), "Track down the flaky test");

    const offer = await screen.findByText(/Assist suggests/);
    expect(offer).toHaveTextContent("CODEX");
    expect(offer).toHaveTextContent("effort high");
    expect(core.assistSuggest).toHaveBeenLastCalledWith("Track down the flaky test");
    // Nothing is picked until the offer is taken.
    expect(screen.getByRole("combobox", { name: "Harness" })).toHaveValue("claude");

    await user.click(screen.getByRole("button", { name: "Use" }));
    expect(screen.getByRole("combobox", { name: "Harness" })).toHaveValue("codex");
    expect(screen.getByRole("combobox", { name: "Effort" })).toHaveValue("high");
    expect(screen.queryByText(/Assist suggests/)).not.toBeInTheDocument();
  });

  it("says nothing when Assist is off, has no key, has nothing to offer, or the message is short", async () => {
    const cases = [
      { status: assistStatus({ suggestInComposer: false }), message: "Track down the flaky test" },
      {
        status: assistStatus({ keySource: "none" as const }),
        message: "Track down the flaky test",
      },
      { status: assistStatus(), message: "fix tests" },
    ];
    for (const { status, message } of cases) {
      useAssistStore.setState({ status });
      const user = await renderComposer();
      await user.type(screen.getByRole("textbox", { name: /work on/ }), message);
      expect(core.assistSuggest).not.toHaveBeenCalled();
      expect(screen.queryByText(/Assist suggests/)).not.toBeInTheDocument();
      cleanup();
    }

    core.assistSuggest.mockResolvedValue(suggestion(null));
    const user = await renderComposer();
    await user.type(screen.getByRole("textbox", { name: /work on/ }), "Track down the flaky test");
    await waitFor(() => expect(core.assistSuggest).toHaveBeenCalled());
    expect(screen.queryByText(/Assist suggests/)).not.toBeInTheDocument();
  });

  it("does not offer what is already chosen", async () => {
    core.assistSuggest.mockResolvedValue(suggestion("claude", { claude: "low" }));
    const user = await renderComposer();
    await user.selectOptions(screen.getByRole("combobox", { name: "Effort" }), "low");
    await user.type(screen.getByRole("textbox", { name: /work on/ }), "Rename the status field");

    await waitFor(() => expect(core.assistSuggest).toHaveBeenCalled());
    expect(screen.queryByText(/Assist suggests/)).not.toBeInTheDocument();
  });

  it("offers to import worktrees that already exist, and adds what was imported", async () => {
    core.projectUntrackedWorktrees.mockResolvedValue([
      { path: "/elsewhere/app/old", branch: "old" },
    ]);
    const imported = worktree("app", "old", { path: "/elsewhere/app/old" });
    core.workspacesImport.mockResolvedValue([imported]);
    const user = await renderComposer();

    await user.click(screen.getByRole("button", { name: "Import worktrees…" }));
    const dialog = await screen.findByRole("dialog", { name: "Import worktrees into app" });
    await within(dialog).findByRole("checkbox", { name: "old" });
    await user.click(within(dialog).getByRole("button", { name: "Import 1 worktree" }));

    expect(core.workspacesImport).toHaveBeenCalledWith("p-app", ["/elsewhere/app/old"]);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useProjectsStore.getState().projects[0]!.workspaces.map((w) => w.name)).toEqual([
      "local",
      "old",
    ]);
  });

  it("starts from the handoff packet, says so, and sends it as a handoff rather than a task", async () => {
    const packet = '# Handoff from Yardsort: workspace "local" in app\n\nYou are taking over.';
    useProjectsStore.setState({
      composingProjectId: null,
      composingWorkspaceId: "w-app",
      composingPrompt: packet,
    });
    core.ptySpawn.mockResolvedValue(session("w-app"));
    const user = userEvent.setup();
    render(<Composer project={app} runIn={app.workspaces[0]} />);
    await screen.findByRole("form", { name: "Run in local" });
    expect(screen.getByRole("heading")).toHaveTextContent("Hand off in app");
    expect(screen.getByText(/never the last agent.s words/)).toBeInTheDocument();
    const box = screen.getByRole("textbox", { name: /work on/ });
    expect(box).toHaveValue(packet);

    await user.click(box);
    await user.keyboard("{End} Start with src/login.rs.{Enter}");
    await waitFor(() => expect(core.ptySpawn).toHaveBeenCalled());
    expect(core.ptySpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        workspaceId: "w-app",
        harness: {
          id: "claude",
          model: null,
          effort: null,
          prompt: `${packet} Start with src/login.rs.`,
          handoff: true,
        },
      }),
    );
    expect(core.workspaceCreate).not.toHaveBeenCalled();
  });

  it("says a run in a worktree runs in that workspace, and a run in local in the checkout", async () => {
    const tree = worktree("app", "fix-login");
    const { unmount } = render(<Composer project={app} runIn={tree} />);
    expect(
      await screen.findByText(/Runs in this workspace, on the branch it has out/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/project's own checkout/)).not.toBeInTheDocument();
    unmount();
    render(<Composer project={app} runIn={app.workspaces[0]} />);
    expect(await screen.findByText(/Runs in the project's own checkout/)).toBeInTheDocument();
  });

  it("adds the project's memory after the message unless told not to, and can show it", async () => {
    const preview =
      "## Project memory\n\n- The tests need TZ=UTC. (memory ab12cd34, from the user)\n";
    core.memoryGet.mockResolvedValue({
      projectId: "p-app",
      shared: true,
      preview,
      entries: [
        {
          id: "ab12cd34-0000",
          shortId: "ab12cd34",
          text: "The tests need TZ=UTC.",
          state: "approved",
          author: "user",
          from: "the user",
          createdAt: 0,
          updatedAt: 0,
          history: [],
        },
      ],
    });
    core.ptySpawn.mockResolvedValue(session("w-app"));
    useProjectsStore.setState({ composingProjectId: null, composingWorkspaceId: "w-app" });
    const user = userEvent.setup();
    render(<Composer project={app} runIn={app.workspaces[0]} />);
    const add = await screen.findByRole("checkbox", { name: /Add this project.s memory/ });
    expect(add).toBeChecked();
    expect(screen.getByText(/1 approved entry/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show" }));
    expect(screen.getByLabelText("Project memory to be added")).toHaveTextContent("TZ=UTC");

    await user.click(add);
    await user.type(screen.getByRole("textbox", { name: /work on/ }), "Fix it{Enter}");
    await waitFor(() => expect(core.ptySpawn).toHaveBeenCalled());
    expect(core.ptySpawn.mock.calls[0]![0].harness).toMatchObject({
      prompt: "Fix it",
      skipMemory: true,
    });
  });

  it("shows your history with the chosen agent, and says when it is too little to go on", async () => {
    core.outcomesAgents.mockResolvedValue([
      {
        harness: "claude",
        attempts: 6,
        kept: 3,
        keptByMerge: 1,
        partly: 1,
        discarded: 1,
        known: 5,
        enough: true,
      },
      {
        harness: "codex",
        attempts: 2,
        kept: 1,
        keptByMerge: 0,
        partly: 0,
        discarded: 0,
        known: 1,
        enough: false,
      },
    ]);
    const user = await renderComposer();
    const line = await screen.findByLabelText("Your history with this agent");
    expect(line).toHaveTextContent("kept 3 of 5 (1 by merge)");
    expect(line).toHaveTextContent("never used to choose for you");
    await user.selectOptions(screen.getByRole("combobox", { name: "Harness" }), "codex");
    expect(screen.getByLabelText("Your history with this agent")).toHaveTextContent(
      "too few to say yet — 1 outcome so far",
    );
  });

  it("does not offer importing when running in an existing workspace", async () => {
    render(<Composer project={app} runIn={app.workspaces[0]} />);
    await screen.findByRole("form", { name: "Run in local" });
    expect(screen.queryByRole("button", { name: "Import worktrees…" })).not.toBeInTheDocument();
  });
});
