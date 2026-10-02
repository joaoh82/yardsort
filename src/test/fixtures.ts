import type {
  AddedProject,
  HarnessInfo,
  Project,
  ProjectPullRequests,
  PullRequest,
  PullRequestSummary,
  SessionRecord,
  Workspace,
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
