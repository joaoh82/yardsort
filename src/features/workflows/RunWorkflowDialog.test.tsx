import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RunPreview, WorkflowItem } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  workflowList: vi.fn(),
  workflowPreview: vi.fn(),
  workflowStart: vi.fn(),
  harnessList: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useWorkflowStore } from "@/stores/workflows";
import { harness, project, worktree } from "@/test/fixtures";
import { RunWorkflowDialog } from "./RunWorkflowDialog";

const review: WorkflowItem = {
  id: "code-review",
  name: "Request code review",
  description: "A second agent reviews the pull request.",
  source: { kind: "builtIn" },
  problems: [],
  text: "",
  activeRuns: 0,
  workflow: {
    id: "code-review",
    name: "Request code review",
    description: "A second agent reviews the pull request.",
    version: 1,
    trigger: { kind: "manual", context: "workspace" },
    inputs: [
      {
        id: "reviewer",
        kind: "harness",
        label: "Who reviews",
        required: true,
        default: null,
        options: [],
      },
      {
        id: "focus",
        kind: "text",
        label: "Anything to look at",
        required: false,
        default: null,
        options: [],
      },
    ],
    steps: [],
  },
};

const withPr: RunPreview = {
  needsPullRequest: true,
  pullRequest: { number: 7, url: "https://github.com/o/r/pull/7", title: "Fix the login" },
  pullRequestProblem: null,
  needsOrigin: true,
  empty: [],
};

const fixLogin = worktree("app", "fix-login");

describe("RunWorkflowDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorkflowStore.setState({ items: [review], loaded: true });
    useHarnessStore.setState({
      loaded: true,
      harnesses: [
        harness("claude"),
        harness("codex"),
        harness("gemini", { resolvedPath: null }),
        harness("off", { enabled: false }),
      ],
    });
    useProjectsStore.setState({
      projects: [{ ...project("app"), workspaces: [...project("app").workspaces, fixLogin] }],
    });
  });

  it("shows the pull request the run will use, and starts it with the answers given", async () => {
    core.workflowPreview.mockResolvedValue(withPr);
    core.workflowStart.mockResolvedValue("run-1");
    const onStarted = vi.fn();
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(
      <RunWorkflowDialog
        workflowId="code-review"
        workspace={fixLogin}
        onClose={onClose}
        onStarted={onStarted}
      />,
    );
    expect(screen.getByRole("heading", { name: "Run “Request code review”" })).toBeInTheDocument();
    expect(await screen.findByText("#7 Fix the login")).toBeInTheDocument();
    expect(core.workflowPreview).toHaveBeenCalledWith("code-review", fixLogin.id);
    expect(
      screen.getByText(/tells the agent already running in this workspace/),
    ).toBeInTheDocument();

    const who = screen.getByRole("combobox", { name: "Who reviews" });
    expect(who).toHaveValue("claude");
    expect(screen.getByRole("option", { name: "GEMINI — not installed" })).toBeDisabled();
    expect(screen.queryByRole("option", { name: "OFF" })).toBeNull();
    await user.selectOptions(who, "codex");
    await user.type(screen.getByRole("textbox", { name: "Anything to look at" }), "the SQL");
    await user.click(screen.getByRole("button", { name: "Run" }));

    expect(core.workflowStart).toHaveBeenCalledWith("code-review", fixLogin.id, {
      reviewer: "codex",
      focus: "the SQL",
    });
    await waitFor(() => expect(onStarted).toHaveBeenCalledWith("run-1", "code-review"));
    expect(onClose).toHaveBeenCalled();
  });

  it("answers a required choice with the option it shows", async () => {
    const workflow = review.workflow!;
    useWorkflowStore.setState({
      items: [
        {
          ...review,
          workflow: {
            ...workflow,
            inputs: [
              {
                id: "depth",
                kind: "choice",
                label: "How deep",
                required: true,
                default: null,
                options: ["safe"],
              },
            ],
          },
        },
      ],
    });
    core.workflowPreview.mockResolvedValue({ ...withPr, needsPullRequest: false });
    core.workflowStart.mockResolvedValue("run-2");
    const user = userEvent.setup();
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    expect(screen.getByRole("combobox", { name: "How deep" })).toHaveValue("safe");
    const run = screen.getByRole("button", { name: "Run" });
    await waitFor(() => expect(run).toBeEnabled());
    await user.click(run);
    expect(core.workflowStart).toHaveBeenCalledWith("code-review", fixLogin.id, { depth: "safe" });
  });

  it("says what the run will render as nothing, and still lets it run", async () => {
    core.workflowPreview.mockResolvedValue({
      ...withPr,
      empty: [
        "`{{ workspace.task }}` will be empty: `fix-login` was started without a first message.",
      ],
    });
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    const list = await screen.findByRole("list", { name: "Will be empty" });
    expect(list).toHaveTextContent("started without a first message");
    await waitFor(() => expect(screen.getByRole("button", { name: "Run" })).toBeEnabled());
  });

  it("will not start without the pull request a workflow needs, and says why", async () => {
    core.workflowPreview.mockResolvedValue({
      ...withPr,
      pullRequest: null,
      pullRequestProblem: "`fix-login` has no open pull request.",
    });
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "`fix-login` has no open pull request.",
    );
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
  });

  it("asks which workflow and which workspace when neither is given", async () => {
    core.workflowPreview.mockResolvedValue({ ...withPr, needsOrigin: false });
    const user = userEvent.setup();
    render(<RunWorkflowDialog onClose={() => {}} />);
    const run = screen.getByRole("button", { name: "Run" });
    expect(run).toBeDisabled();
    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "code-review");
    expect(core.workflowPreview).not.toHaveBeenCalled();
    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), fixLogin.id);
    await waitFor(() => expect(run).toBeEnabled());
    expect(screen.getByRole("option", { name: "app / fix-login" })).toBeInTheDocument();
  });

  it("shows what the core refused, and stays open to fix it", async () => {
    core.workflowPreview.mockResolvedValue(withPr);
    core.workflowStart.mockRejectedValue({
      code: "workflow_already_running",
      message: "`code-review` is already running in `fix-login` (run 3f2a9c1e).",
    });
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={onClose} />);
    await screen.findByText("#7 Fix the login");
    await user.click(screen.getByRole("button", { name: "Run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("already running in `fix-login`");
    expect(onClose).not.toHaveBeenCalled();
  });
});
