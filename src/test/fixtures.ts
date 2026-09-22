import type { AddedProject, HarnessInfo, Project, SessionRecord, Workspace } from "@/lib/ipc";

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
