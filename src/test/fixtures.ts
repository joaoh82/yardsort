import type {
  AddedProject,
  HarnessInfo,
  Project,
  ProjectPullRequests,
  ProjectTasks,
  PullRequest,
  PullRequestSummary,
  Schedule,
  ScheduleRun,
  SessionRecord,
  Task,
  TaskDetail,
  TestReview,
  TestRun,
  TestStepRun,
  TriggerFire,
  TriggerItem,
  TriggerPreset,
  Workspace,
  RemoteRepository,
  Repositories,
} from "@/lib/ipc";

/** A project with its `local` workspace, as the core would describe it. */
export function project(name: string, overrides: Partial<Project> = {}): Project {
  const id = `p-${name}`;
  return {
    id,
    name,
    rootPath: `/code/${name}`,
    missing: false,
    workspaces: [
      {
        id: `w-${name}`,
        projectId: id,
        kind: "local",
        name: "local",
        path: `/code/${name}`,
        head: { label: "main", detached: false, unborn: false },
        missing: false,
        archived: false,
        branchGone: false,
        tasks: [],
      },
    ],
    ...overrides,
  };
}

export const added = (name: string, flags: Partial<AddedProject> = {}): AddedProject => ({
  project: project(name),
  alreadyKnown: false,
  openedRootInstead: false,
  revived: false,
  ...flags,
});

/** A worktree workspace belonging to `project(projectName)`. */
export const worktree = (
  projectName: string,
  name: string,
  overrides: Partial<Workspace> = {},
): Workspace => ({
  id: `w-${projectName}-${name}`,
  projectId: `p-${projectName}`,
  kind: "worktree",
  name,
  path: `/worktrees/${projectName}/${name}`,
  head: { label: `ys/${name}`, detached: false, unborn: false },
  missing: false,
  archived: false,
  branchGone: false,
  tasks: [],
  ...overrides,
});

/** A harness conversation on record, ended cleanly and resumable unless overridden. */
export const record = (id: string, overrides: Partial<SessionRecord> = {}): SessionRecord => ({
  id,
  workspaceId: "ws",
  harnessId: "claude",
  harnessLabel: "Claude Code",
  model: null,
  effort: null,
  title: "fix the login bug",
  forkedFrom: null,
  running: false,
  ptySessionId: null,
  exitCode: 0,
  interrupted: false,
  startedAt: Date.now() - 3_600_000,
  endedAt: Date.now() - 1_800_000,
  resumable: true,
  forkable: true,
  unavailableReason: null,
  ...overrides,
});

/** A harness as the core describes it, installed and with no model or effort choices. */
export const harness = (id: string, extra: Partial<HarnessInfo> = {}): HarnessInfo => ({
  id,
  label: id.toUpperCase(),
  command: id,
  baseArgs: [],
  modelArgs: [],
  effortArgs: [],
  sessionArgs: [],
  promptArgs: [],
  resumeArgs: [],
  forkArgs: [],
  writeArgs: [],
  contextArgs: [],
  resumeContextArgs: [],
  efforts: [],
  models: [],
  promptTransport: "argv",
  sessionIdMode: "assigned",
  stdinReadyMs: 1500,
  enabled: true,
  builtin: true,
  modified: false,
  resolvedPath: `/usr/bin/${id}`,
  ...extra,
});

type PullRequestDetails = NonNullable<PullRequest["details"]>;

/** An open pull request with passing checks, as the core would describe it. */
export const pullRequest = (
  number: number,
  overrides: Partial<Omit<PullRequest, "details">> & { details?: Partial<PullRequestDetails> } = {},
): PullRequest => {
  const { details, ...rest } = overrides;
  return {
    number,
    url: `https://github.com/demo/app/pull/${number}`,
    title: `Pull request ${number}`,
    branch: `ys/branch-${number}`,
    state: "open",
    draft: false,
    checks: "passing",
    author: "grace",
    createdAt: Date.parse("2026-09-28T09:00:00Z"),
    ...rest,
    details: {
      base: "main",
      headOid: `head-${number}`,
      baseOid: "b".repeat(40),
      additions: 10,
      deletions: 2,
      review: "",
      updatedAt: "2026-09-28T10:00:00Z",
      checks: [],
      checkCounts: { passed: 3, failed: 0, running: 0 },
      mergeable: "mergeable",
      reviewRequests: [],
      reviews: [],
      crossRepository: false,
      ...details,
    },
  };
};

/** What the core says about one project's pull requests: `gh` there, logged in as `ada`. */
export const pullRequestsOf = (
  pullRequests: PullRequest[],
  overrides: Partial<ProjectPullRequests> = {},
): ProjectPullRequests => ({
  gh: true,
  pullRequests,
  problem: null,
  loggedOut: false,
  workspaces: {},
  repo: { host: "github.com", owner: "demo", name: "app", kind: "github" },
  viewer: "ada",
  openTotal: pullRequests.filter((pr) => pr.state === "open").length,
  openProblem: null,
  ...overrides,
});

/** A pull request read in full: a description, and nothing said about it yet. */
export const pullRequestSummary = (
  pr: PullRequest,
  overrides: Partial<PullRequestSummary> = {},
): PullRequestSummary => ({
  pullRequest: pr,
  body: `What ${pr.title} is about.`,
  changedFiles: 2,
  posts: [],
  ...overrides,
});

/** An open task nobody has answered yet, as the core would describe a GitHub issue. */
export const task = (number: number, overrides: Partial<Task> = {}): Task => ({
  source: "github",
  key: `#${number}`,
  url: `https://github.com/demo/app/issues/${number}`,
  title: `Task ${number}`,
  state: "open",
  closedAs: null,
  author: "grace",
  labels: [],
  assignees: [],
  comments: 0,
  createdAt: "2026-09-28T09:00:00Z",
  updatedAt: "2026-09-28T10:00:00Z",
  needsAnswer: false,
  linkedPullRequests: [],
  ...overrides,
});

/** What the core says about one project's tasks: `gh` there, logged in as `ada`. */
export const tasksOf = (tasks: Task[], overrides: Partial<ProjectTasks> = {}): ProjectTasks => ({
  gh: true,
  tasks,
  problem: null,
  loggedOut: false,
  repo: { host: "github.com", owner: "demo", name: "app", kind: "github" },
  viewer: "ada",
  openTotal: tasks.filter((it) => it.state === "open").length,
  disabled: false,
  closed: false,
  ...overrides,
});

/** A task read in full: a description, and nothing said about it yet. */
export const taskDetail = (it: Task, overrides: Partial<TaskDetail> = {}): TaskDetail => ({
  task: it,
  body: `What ${it.title} is about.`,
  comments: [],
  ...overrides,
});

/** A repository on the forge as the clone dialog lists it: public, not a fork, pushed to
 *  within the hour. `parent` names a fork's parent, which gets both URLs from its name. */
export const repository = (
  nameWithOwner: string,
  overrides: Partial<Omit<RemoteRepository, "parent">> & { parent?: string } = {},
): RemoteRepository => {
  const { parent, ...rest } = overrides;
  return {
    nameWithOwner,
    description: null,
    cloneUrl: `https://github.com/${nameWithOwner}.git`,
    isPrivate: false,
    isFork: !!parent,
    isArchived: false,
    parent: parent ? { nameWithOwner: parent, cloneUrl: `https://github.com/${parent}.git` } : null,
    language: null,
    pushedAt: new Date(Date.now() - 30 * 60_000).toISOString(),
    projectId: null,
    ...rest,
  };
};

/** What `forge_repositories` answers: every page read, nothing wrong. */
export const repositoriesOf = (
  repositories: RemoteRepository[],
  overrides: Partial<Repositories> = {},
): Repositories => ({
  repositories,
  total: repositories.length,
  answered: true,
  problem: null,
  loggedOut: false,
  ...overrides,
});

/** A smoke test run of `health` in `fix-login`, asked for by an agent and waiting for approval
 *  unless overridden. */
export const testRun = (id: string, overrides: Partial<TestRun> = {}): TestRun => ({
  id,
  projectId: "p-app",
  workspaceId: "w-app-fix-login",
  workspaceName: "fix-login",
  testId: "health",
  testName: "The server answers",
  definition:
    "id: health\nname: The server answers\nversion: 1\nsteps:\n  - id: ping\n    action: http\n    get: http://localhost:{{ run.port }}/health\n",
  contentHash: "abc",
  inputs: {},
  status: "awaiting_approval",
  requestedBy: "cli",
  sessionRecordId: null,
  port: null,
  error: null,
  outputDir: null,
  stopping: false,
  createdAt: Date.now() - 60_000,
  startedAt: null,
  endedAt: null,
  ...overrides,
});

/** One step of a run: pending, in the `steps` phase, unless overridden. */
export const testStep = (stepId: string, overrides: Partial<TestStepRun> = {}): TestStepRun => ({
  stepId,
  phase: "steps",
  action: "run",
  status: "pending",
  startedAt: null,
  endedAt: null,
  exit: null,
  outputs: {},
  note: null,
  screenshots: [],
  ...overrides,
});

/** A file as the approval card reads it: one server and one request, one name unset. */
export const testReview = (overrides: Partial<TestReview> = {}): TestReview => ({
  steps: [
    {
      id: "server",
      phase: "setup",
      action: "start",
      words: "start `bun run dev --port {{ run.port }}` and keep it running until the run ends",
    },
    {
      id: "ping",
      phase: "steps",
      action: "http",
      words: "GET http://localhost:{{ run.port }}/health; expect status 200",
    },
  ],
  env: [
    { name: "TEST_PASSWORD", set: true },
    { name: "DATABASE_URL", set: false },
  ],
  approved: false,
  askEveryTime: false,
  previous: null,
  contentHash: "abc",
  runsAProgram: true,
  environment: ["TEST_PASSWORD", "DATABASE_URL"],
  ...overrides,
});

/** A schedule: the workflow `ping`, in `fix-login`, daily at nine, on and waiting, unless
 *  overridden. */
export const schedule = (id: string, overrides: Partial<Schedule> = {}): Schedule => ({
  id,
  name: "Morning ping",
  target: "workflow",
  targetId: "ping",
  projectId: "p-app",
  workspaceId: "w-app-fix-login",
  workspaceName: "fix-login",
  inputs: {},
  spec: { kind: "daily", time: { hour: 9, minute: 0 } },
  when: "Daily at 09:00",
  missed: "run_once",
  notify: "failure",
  enabled: true,
  nextAt: Date.now() + 3_600_000,
  standing: "waiting",
  last: null,
  createdAt: Date.now() - 86_400_000,
  ...overrides,
});

/** One time a schedule came due: it started a run, still going, unless overridden. */
export const scheduleRun = (id: string, overrides: Partial<ScheduleRun> = {}): ScheduleRun => ({
  id,
  scheduleId: "s1",
  dueAt: Date.now() - 120_000,
  firedAt: Date.now() - 120_000,
  outcome: "started",
  manual: false,
  runId: `run-${id}`,
  note: null,
  result: null,
  awaitingApproval: false,
  ...overrides,
});

/** A trigger: the preset Address reviews, in `fix-login`, on and listening, unless overridden. */
export const trigger = (id: string, overrides: Partial<TriggerItem> = {}): TriggerItem => ({
  id,
  name: "Address reviews",
  preset: "address-reviews",
  projectId: "p-app",
  workspaceId: "w-app-fix-login",
  workspaceName: "fix-login",
  sessionId: null,
  sessionTitle: null,
  sessionRunning: false,
  events: ["pr.review", "pr.comment"],
  filters: {},
  target: "workflow",
  targetId: "address-review",
  inputs: {},
  quietMs: 120_000,
  cap: 3,
  notify: "failure",
  enabled: true,
  when: "When a review comes in on the pull request or someone comments on the pull request, run the workflow `address-review`",
  standing: "listening",
  capped: false,
  last: null,
  watch: { number: 142, closed: false, problem: null, lookedAt: Date.now() - 30_000 },
  createdAt: Date.now() - 86_400_000,
  ...overrides,
});

/** One fire of a trigger: two posts gathered into a run, still going, unless overridden. */
export const triggerFire = (id: string, overrides: Partial<TriggerFire> = {}): TriggerFire => ({
  id,
  triggerId: "t1",
  subject: "pr:142",
  event: "pr.review",
  events: 2,
  firstAt: Date.now() - 300_000,
  dueAt: Date.now() - 180_000,
  decidedAt: Date.now() - 180_000,
  outcome: "started",
  runId: `run-${id}`,
  note: null,
  summary: "1 review and 1 comment from bob",
  result: null,
  awaitingApproval: false,
  depth: 0,
  ...overrides,
});

/** The presets, as the core lists them. */
export const triggerPresets: TriggerPreset[] = [
  {
    id: "review-when-done",
    name: "Review when the work is done",
    does: "When the agent says its work is done, a second agent reviews the branch and the first is told what it found.",
    events: ["workspace.done"],
    workflow: "review-branch",
    asks: { input: "reviewer", question: "Who reviews" },
    quietMs: 0,
  },
  {
    id: "address-reviews",
    name: "Address reviews",
    does: "When someone other than you reviews or comments on the workspace's pull request, two minutes after the last one, the agent is told to address it.",
    events: ["pr.review", "pr.comment"],
    workflow: "address-review",
    asks: null,
    quietMs: 120_000,
  },
  {
    id: "smoke-when-done",
    name: "Smoke test when the work is done",
    does: "When the agent says its work is done, a smoke test runs and the agent is told how it went.",
    events: ["workspace.done"],
    workflow: "smoke-then-tell",
    asks: { input: "suite", question: "Which smoke test" },
    quietMs: 0,
  },
];
