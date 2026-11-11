import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RunPreview, WorkflowItem } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  workflowList: vi.fn(),
  workflowPreview: vi.fn(),
  workflowStart: vi.fn(),
  harnessList: vi.fn(),
  testList: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { useWorkflowStore } from "@/stores/workflows";
import {
  harness,
  project,
  pullRequest,
  pullRequestsOf,
  trackedOf,
  worktree,
} from "@/test/fixtures";
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
  pullRequestNote: null,
  needsOrigin: true,
  empty: [],
};

const fixLogin = worktree("app", "fix-login");

describe("RunWorkflowDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.testList.mockResolvedValue([]);
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
    expect(core.workflowPreview).toHaveBeenCalledWith("code-review", fixLogin.id, null);
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

    expect(core.workflowStart).toHaveBeenCalledWith(
      "code-review",
      fixLogin.id,
      { reviewer: "codex", focus: "the SQL" },
      null,
    );
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
    expect(core.workflowStart).toHaveBeenCalledWith(
      "code-review",
      fixLogin.id,
      { depth: "safe" },
      null,
    );
  });

  it("picks a smoke test for a `test` input from the workspace's runnable tests", async () => {
    const workflow = review.workflow!;
    useWorkflowStore.setState({
      items: [
        {
          ...review,
          workflow: {
            ...workflow,
            inputs: [
              {
                id: "suite",
                kind: "test",
                label: "Which smoke test",
                required: true,
                default: null,
                options: [],
              },
            ],
          },
        },
      ],
    });
    const item = (id: string, name: string, extra = {}) => ({
      id,
      name,
      description: null,
      path: `/w/.yardsort/tests/${id}.yaml`,
      text: "",
      problems: [],
      runnable: true,
      usesBrowser: false,
      browserMissing: false,
      inputs: [],
      ...extra,
    });
    core.testList.mockResolvedValue([
      item("login", "Log in"),
      item("broken", "Broken", { runnable: false }),
    ]);
    core.workflowPreview.mockResolvedValue({ ...withPr, needsPullRequest: false });
    core.workflowStart.mockResolvedValue("run-t");
    const user = userEvent.setup();
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    const picker = await screen.findByRole("combobox", { name: "Which smoke test" });
    // Only the runnable test is offered, and it is the default answer.
    expect(screen.getByRole("option", { name: "Log in" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Broken" })).toBeNull();
    expect(picker).toHaveValue("login");
    expect(core.testList).toHaveBeenCalledWith(fixLogin.id);
    const run = screen.getByRole("button", { name: "Run" });
    await waitFor(() => expect(run).toBeEnabled());
    await user.click(run);
    expect(core.workflowStart).toHaveBeenCalledWith(
      "code-review",
      fixLogin.id,
      { suite: "login" },
      null,
    );
  });

  it("revalidates the test pick when the workspace changes", async () => {
    // One workflow, one required `test` input, chosen workspace and all.
    const smokeOnly: WorkflowItem = {
      ...review,
      id: "smoke",
      name: "Smoke",
      workflow: {
        ...review.workflow!,
        id: "smoke",
        name: "Smoke",
        inputs: [
          {
            id: "suite",
            kind: "test",
            label: "Which smoke test",
            required: true,
            default: null,
            options: [],
          },
        ],
      },
    };
    useWorkflowStore.setState({ items: [smokeOnly], loaded: true });
    const alpha = worktree("app", "alpha");
    const beta = worktree("app", "beta");
    const gamma = worktree("app", "gamma");
    useProjectsStore.setState({
      projects: [{ ...project("app"), workspaces: [alpha, beta, gamma] }],
    });
    const item = (id: string, name: string) => ({
      id,
      name,
      description: null,
      path: `/w/.yardsort/tests/${id}.yaml`,
      text: "",
      problems: [],
      runnable: true,
      usesBrowser: false,
      browserMissing: false,
      inputs: [],
    });
    const lists: Record<string, ReturnType<typeof item>[]> = {
      [alpha.id]: [item("login", "Log in"), item("logout", "Log out")],
      [beta.id]: [item("health", "Health")],
      [gamma.id]: [],
    };
    core.testList.mockImplementation((ws: string) => Promise.resolve(lists[ws] ?? []));
    core.workflowPreview.mockResolvedValue({
      needsPullRequest: false,
      pullRequest: null,
      pullRequestProblem: null,
      pullRequestNote: null,
      needsOrigin: false,
      empty: [],
    });
    core.workflowStart.mockResolvedValue("run-s");
    const user = userEvent.setup();
    render(<RunWorkflowDialog onClose={() => {}} />);

    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "smoke");
    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), alpha.id);
    const picker = await screen.findByRole("combobox", { name: "Which smoke test" });
    await waitFor(() =>
      expect(screen.getByRole("option", { name: "Log out" })).toBeInTheDocument(),
    );
    // Explicitly pick the second test, which only exists in workspace alpha.
    await user.selectOptions(picker, "logout");
    expect(picker).toHaveValue("logout");

    // Switch to a workspace with no tests: the stale pick cannot carry over, and Run is disabled.
    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), gamma.id);
    await waitFor(() =>
      expect(
        screen.getByRole("option", { name: "No runnable smoke tests here" }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();

    // Switch to a workspace whose only test is `health`: the pick revalidates to it, and that is
    // what both the select shows and Run submits — never the stale `logout`.
    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), beta.id);
    await waitFor(() => expect(picker).toHaveValue("health"));
    expect(screen.queryByRole("option", { name: "Log out" })).toBeNull();
    const run = screen.getByRole("button", { name: "Run" });
    await waitFor(() => expect(run).toBeEnabled());
    await user.click(run);
    expect(core.workflowStart).toHaveBeenCalledWith("smoke", beta.id, { suite: "health" }, null);
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

  it("starts when the branch has no pull request yet, and says the run will open one", async () => {
    core.workflowPreview.mockResolvedValue({
      ...withPr,
      pullRequest: null,
      pullRequestNote: "Will use the PR the run opens.",
    });
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    expect(await screen.findByText("Will use the PR the run opens.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    await waitFor(() => expect(screen.getByRole("button", { name: "Run" })).toBeEnabled());
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

describe("RunWorkflowDialog on a pull request elsewhere", () => {
  const elsewhere: WorkflowItem = {
    ...review,
    id: "code-review-elsewhere",
    name: "Request code review elsewhere",
    workflow: {
      ...review.workflow!,
      id: "code-review-elsewhere",
      name: "Request code review elsewhere",
      trigger: { kind: "manual", context: "tracked_pull_request" },
    },
  };
  const tracked = {
    id: "t-7",
    workspaceId: fixLogin.id,
    sessionId: null,
    repo: { host: "github.com", owner: "demo", name: "lib", kind: "github" as const },
    number: 7,
    url: "https://github.com/demo/lib/pull/7",
    trackedBy: "hook",
    pullRequest: null,
    problem: null,
  };

  beforeEach(() => {
    vi.clearAllMocks();
    core.testList.mockResolvedValue([]);
    useWorkflowStore.setState({ items: [review, elsewhere], loaded: true });
    useHarnessStore.setState({ loaded: true, harnesses: [harness("claude"), harness("codex")] });
    useProjectsStore.setState({
      projects: [{ ...project("app"), workspaces: [...project("app").workspaces, fixLogin] }],
    });
    usePublishStore.setState({ byProject: {} });
  });

  const noneOfItsOwn: RunPreview = {
    ...withPr,
    pullRequest: null,
    pullRequestProblem: "`fix-login` has no open pull request.",
  };
  const merged = trackedOf(fixLogin.id, pullRequest(61, { state: "merged", title: "Old one" }));
  const trackedHere = (
    tracked = [
      trackedOf(fixLogin.id, pullRequest(226, { title: "Usage totals" })),
      merged,
      trackedOf("w-other", pullRequest(9, { title: "Someone else's" })),
    ],
  ) =>
    usePublishStore.setState({
      byProject: { [fixLogin.projectId]: pullRequestsOf([], { tracked }) },
    });

  it("offers the open pull requests the workspace tracks when it has none of its own", async () => {
    core.workflowPreview.mockImplementation((_id: string, _ws: string, trackedId: string | null) =>
      Promise.resolve(
        trackedId
          ? {
              ...withPr,
              pullRequest: {
                number: 226,
                url: "https://github.com/demo/lib/pull/226",
                title: "Usage totals",
                elsewhere: { repo: "demo/lib", clone: "/code/app/lib" },
              },
            }
          : noneOfItsOwn,
      ),
    );
    core.workflowStart.mockResolvedValue("run-3");
    trackedHere();
    const user = userEvent.setup();
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("has no open pull request");
    const offered = screen.getByRole("list", { name: "Open elsewhere" });
    // Only the open one, and only this workspace's.
    expect(
      within(offered)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["demo/lib#226 Usage totals"]);
    await user.selectOptions(screen.getByRole("combobox", { name: "Who reviews" }), "codex");
    await user.click(within(offered).getByRole("button", { name: "demo/lib#226 Usage totals" }));

    expect(
      screen.getByRole("heading", { name: "Run “Request code review elsewhere”" }),
    ).toBeInTheDocument();
    expect(await screen.findByText(/demo\/lib#\s*226 Usage totals/)).toBeInTheDocument();
    expect(core.workflowPreview).toHaveBeenLastCalledWith(
      "code-review-elsewhere",
      fixLogin.id,
      "t-226",
    );
    expect(screen.getByRole("combobox", { name: "Who reviews" })).toHaveValue("codex");
    await user.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() =>
      expect(core.workflowStart).toHaveBeenCalledWith(
        "code-review-elsewhere",
        fixLogin.id,
        { reviewer: "codex" },
        "t-226",
      ),
    );
  });

  it("goes back to the list to pick another, keeping what was answered", async () => {
    core.workflowPreview.mockImplementation((_id: string, _ws: string, trackedId: string | null) =>
      Promise.resolve(trackedId ? withPr : noneOfItsOwn),
    );
    trackedHere([
      trackedOf(fixLogin.id, pullRequest(226, { title: "Usage totals" })),
      trackedOf(fixLogin.id, pullRequest(61, { title: "Model spend" })),
    ]);
    const user = userEvent.setup();
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    await user.click(await screen.findByRole("button", { name: "demo/lib#226 Usage totals" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Who reviews" }), "codex");
    await user.click(screen.getByRole("button", { name: "Choose another pull request" }));

    expect(screen.getByRole("heading", { name: "Run “Request code review”" })).toBeInTheDocument();
    await user.click(await screen.findByRole("button", { name: "demo/lib#61 Model spend" }));
    expect(core.workflowPreview).toHaveBeenLastCalledWith(
      "code-review-elsewhere",
      fixLogin.id,
      "t-61",
    );
    expect(screen.getByRole("combobox", { name: "Who reviews" })).toHaveValue("codex");
  });

  it("lets a picked pull request go when the workspace changes", async () => {
    const other = worktree("app", "other");
    useProjectsStore.setState({
      projects: [
        { ...project("app"), workspaces: [...project("app").workspaces, fixLogin, other] },
      ],
    });
    core.workflowPreview.mockImplementation((_id: string, _ws: string, trackedId: string | null) =>
      Promise.resolve(trackedId ? withPr : noneOfItsOwn),
    );
    trackedHere([
      trackedOf(fixLogin.id, pullRequest(226, { title: "Usage totals" })),
      trackedOf(other.id, pullRequest(61, { title: "Model spend" }), { id: "t-other-61" }),
    ]);
    const user = userEvent.setup();
    render(<RunWorkflowDialog workflowId="code-review" onClose={() => {}} />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), fixLogin.id);
    await user.click(await screen.findByRole("button", { name: "demo/lib#226 Usage totals" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), other.id);

    expect(core.workflowPreview).toHaveBeenLastCalledWith("code-review", other.id, null);
    expect(screen.getByRole("heading", { name: "Run “Request code review”" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "demo/lib#61 Model spend" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /#226/ })).toBeNull();
  });

  it("names them, without offering a run, when the workflow has no twin about one", async () => {
    core.workflowPreview.mockResolvedValue(noneOfItsOwn);
    useWorkflowStore.setState({ items: [review], loaded: true });
    trackedHere();
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    expect(await screen.findByText(/tracks open pull requests elsewhere \(demo\/lib#226\)/));
    expect(screen.queryByRole("list", { name: "Open elsewhere" })).toBeNull();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
  });

  it("offers nothing when the workspace tracks no open pull request", async () => {
    core.workflowPreview.mockResolvedValue(noneOfItsOwn);
    trackedHere([merged]);
    render(<RunWorkflowDialog workflowId="code-review" workspace={fixLogin} onClose={() => {}} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("has no open pull request");
    expect(screen.queryByText(/tracks open pull requests elsewhere/)).toBeNull();
  });

  it("runs on that pull request, read by its link, and offers only workflows about one", async () => {
    core.workflowPreview.mockResolvedValue({
      ...withPr,
      pullRequest: {
        number: 7,
        url: tracked.url,
        title: "Bump the client",
        elsewhere: { repo: "demo/lib", clone: "/code/app/lib" },
      },
    });
    core.workflowStart.mockResolvedValue("run-2");
    const user = userEvent.setup();
    render(<RunWorkflowDialog workspace={fixLogin} tracked={tracked} onClose={() => {}} />);
    const picker = screen.getByRole("combobox", { name: "Workflow" });
    const offered = [...picker.querySelectorAll("option")].map((o) => o.textContent);
    expect(offered).toEqual(["Choose…", "Request code review elsewhere"]);
    await user.selectOptions(picker, "code-review-elsewhere");
    expect(await screen.findByText(/demo\/lib#\s*7 Bump the client/)).toBeInTheDocument();
    expect(core.workflowPreview).toHaveBeenCalledWith("code-review-elsewhere", fixLogin.id, "t-7");
    expect(screen.getByText(/tells the agent that opened the pull request/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() =>
      expect(core.workflowStart).toHaveBeenCalledWith(
        "code-review-elsewhere",
        fixLogin.id,
        { reviewer: "claude" },
        "t-7",
      ),
    );
  });

  it("says why it cannot run without the project's clone of the repository", async () => {
    core.workflowPreview.mockResolvedValue({
      ...withPr,
      pullRequest: null,
      pullRequestProblem:
        "This project's folder holds no clone of demo/lib for a reviewer to run its checks in.",
    });
    render(
      <RunWorkflowDialog
        workflowId="code-review-elsewhere"
        workspace={fixLogin}
        tracked={tracked}
        onClose={() => {}}
      />,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("no clone of demo/lib");
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
  });
});
