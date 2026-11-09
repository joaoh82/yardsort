/**
 * The only module that talks to the Rust core. Everything else imports from here, never from
 * `bindings.ts` (generated) or `@tauri-apps/api` directly.
 */
import { Channel, isTauri } from "@tauri-apps/api/core";
import {
  commands,
  events,
  type MachineReport as RawMachineReport,
  type UsageReport as RawUsageReport,
  type ActivityCounter,
  type ActivityDiagnostics,
  type ActivityEvent,
  type ActivityPage,
  type ActivitySettingsDto,
  type ContextUsage,
  type CoordinatorStatus,
  type ConversationContext,
  type AgentOutcomes,
  type Attempt,
  type FileReports,
  type MemoryCheck,
  type MemoryDecision,
  type MemoryEntry,
  type MemoryWaiting,
  type ProjectMemory,
  type ProjectOutcomes,
  type HandoffPacket,
  type ForkPacket,
  type ObservedMatch,
  type ObservedWrite,
  type Provenance,
  type RunCoverage,
  type WriteReport,
  type AddedProject,
  type AppInfo,
  type AssistStatus,
  type AvailableUpdate,
  type BranchList,
  type ChangeSet,
  type Content,
  type CreatedWorkspace,
  type DaemonStatus,
  type DownloadProgress,
  type DraftedPullRequest,
  type DraftStatus,
  type EnvInfo,
  type ExitInfo,
  type FileChange,
  type FileDiff,
  type FileEntry,
  type FileReview,
  type HarnessDef,
  type HarnessInfo,
  type HarnessPreview,
  type HarnessRequest,
  type HostEvent,
  type IpcError,
  type NewWorkspace,
  type Preflight,
  type Project,
  type ProjectAutomation,
  type ProjectPullRequests,
  type ProjectTasks,
  type Task,
  type TaskComment,
  type TaskDetail,
  type TaskLabel,
  type TaskRef,
  type TaskChoices,
  type TaskEdit,
  type NewTask,
  type CreatedTask,
  type CloseReason,
  type Delegated,
  type PublishState,
  type PullRequest,
  type PullRequestOpened,
  type MergeMethod,
  type CheckCounts,
  type Checks,
  type PreparedBranch,
  type DiffSide,
  type Excerpt,
  type LineComment,
  type LinePlace,
  type PullRequestChanges,
  type PullRequestCheck,
  type PullRequestPost,
  type PullRequestReview,
  type PullRequestSummary,
  type TrackedPullRequest,
  type Repo,
  type ReviewRequest,
  type ReviewState,
  type ConflictHelper,
  type ConflictsAsked,
  type Relevance,
  type Review,
  type ReviewFlag,
  type Scope,
  type SessionId,
  type SessionInfo,
  type SessionRecord,
  type SettingsInfo,
  type SpawnRequest,
  type Suggestion,
  type TermSize,
  type ThresholdsDto,
  type UntrackedWorktree,
  type UpdateStatus,
  type Workspace,
  type WorkspaceSettingsDto,
  type YsStatus,
  type Described,
  type WorkflowItem,
  type WorkflowCheck,
  type WorkflowRun,
  type WorkflowStepRun,
  type RunPreview,
  type Workflow,
  type Step,
  type Input,
  type InputKind,
  type Problem,
  type Source,
  type PrFacts,
  type OtherComputers,
  type OtherHost,
  type OtherHostView,
  type OtherLink,
  type RemoteController,
  type RemoteTerminalSeen,
  type OtherPairing,
  type OtherSession,
  type OtherTerminal,
  type OtherWorkspace,
  type RemoteAnswer,
  type RemoteDevice,
  type RemoteEarlierDaemon,
  type RemoteLink,
  type RemotePairing,
  type RemoteRequest,
  type RemoteRequests,
  type RemoteStatus,
  type RemoteRepository,
  type Repositories,
  type ResolvedLink,
  type LinkKind,
  type TestItem,
  type TestListing,
  type LastRun,
  type TestDescribed,
  type TestInput,
  type TestRun as RawTestRun,
  type TestStepRun as RawTestStepRun,
  type TestStarted,
  type TestEnvEntry,
  type TestSettings,
  type TestReview,
  type StepWords,
  type EnvUse,
  type OutputChunk as RawOutputChunk,
  type TestSent,
  type TestResultSent,
  type Schedule as RawSchedule,
  type ScheduleRun as RawScheduleRun,
  type ScheduleSpec as RawScheduleSpec,
  type ScheduleDraft as RawScheduleDraft,
  type SchedulePreview as RawSchedulePreview,
  type ScheduledRun,
  type ScheduleTarget,
  type ScheduleStanding,
  type ScheduleOutcome,
  type ScheduleNotify,
  type IfMissed,
  type TimeOfDay,
  type Weekday,
  type TriggerItem as RawTriggerItem,
  type TriggerFire as RawTriggerFire,
  type TriggerDraft_Deserialize as RawTriggerDraft,
  type SavedTrigger as RawSavedTrigger,
  type TriggerPreset,
  type PresetQuestion,
  type PresetPick,
  type TriggeredRun,
  type TriggerEvent,
  type TriggerStanding,
  type Filters_Deserialize as TriggerFilters,
  type FromFilter,
  type ReviewStateFilter,
  type ExitFilter,
  type VerdictFilter,
  type OutcomeFilter,
  type UntoldAgent,
} from "./bindings";

export type {
  CoordinatorStatus,
  RemoteRepository,
  Repositories,
  ResolvedLink,
  LinkKind,
  TestItem,
  TestListing,
  LastRun,
  TestDescribed,
  TestInput,
  TestStarted,
  TestEnvEntry,
  TestSettings,
  TestReview,
  StepWords,
  EnvUse,
  TestSent,
  TestResultSent,
  ScheduledRun,
  ScheduleTarget,
  ScheduleStanding,
  ScheduleOutcome,
  ScheduleNotify,
  IfMissed,
  TimeOfDay,
  Weekday,
  TriggerPreset,
  PresetQuestion,
  PresetPick,
  TriggeredRun,
  TriggerEvent,
  TriggerStanding,
  TriggerFilters,
  FromFilter,
  ReviewStateFilter,
  ExitFilter,
  VerdictFilter,
  OutcomeFilter,
  UntoldAgent,
  Described,
  WorkflowItem,
  WorkflowCheck,
  WorkflowRun,
  WorkflowStepRun,
  RunPreview,
  Workflow,
  Step,
  Input,
  InputKind,
  Problem,
  Source,
  PrFacts,
  ActivityCounter,
  ActivityDiagnostics,
  ActivityEvent,
  ActivityPage,
  ActivitySettingsDto,
  ContextUsage,
  ConversationContext,
  AgentOutcomes,
  Attempt,
  FileReports,
  MemoryCheck,
  MemoryDecision,
  MemoryEntry,
  MemoryWaiting,
  ProjectMemory,
  ProjectOutcomes,
  HandoffPacket,
  ForkPacket,
  ObservedMatch,
  ObservedWrite,
  Provenance,
  RunCoverage,
  WriteReport,
  ProjectAutomation,
  AddedProject,
  AppInfo,
  AssistStatus,
  AvailableUpdate,
  BranchList,
  ChangeSet,
  Content,
  CreatedWorkspace,
  DaemonStatus,
  DownloadProgress,
  DraftedPullRequest,
  DraftStatus,
  EnvInfo,
  ExitInfo,
  FileChange,
  FileDiff,
  FileEntry,
  FileReview,
  HarnessDef,
  HarnessInfo,
  HarnessPreview,
  HarnessRequest,
  HostEvent,
  IpcError,
  NewWorkspace,
  Preflight,
  Project,
  ProjectPullRequests,
  ProjectTasks,
  OtherComputers,
  OtherHost,
  OtherHostView,
  OtherLink,
  OtherPairing,
  RemoteController,
  RemoteTerminalSeen,
  OtherSession,
  OtherTerminal,
  OtherWorkspace,
  RemoteAnswer,
  RemoteDevice,
  RemoteEarlierDaemon,
  RemoteLink,
  RemotePairing,
  RemoteRequest,
  RemoteRequests,
  RemoteStatus,
  Task,
  TaskComment,
  TaskDetail,
  TaskLabel,
  TaskRef,
  TaskChoices,
  TaskEdit,
  NewTask,
  CreatedTask,
  CloseReason,
  Delegated,
  PublishState,
  PullRequest,
  PullRequestOpened,
  MergeMethod,
  CheckCounts,
  Checks,
  PreparedBranch,
  DiffSide,
  Excerpt,
  LineComment,
  LinePlace,
  PullRequestChanges,
  PullRequestCheck,
  PullRequestPost,
  PullRequestReview,
  PullRequestSummary,
  TrackedPullRequest,
  Repo,
  ReviewRequest,
  ReviewState,
  ConflictHelper,
  ConflictsAsked,
  Relevance,
  Review,
  ReviewFlag,
  Scope,
  SessionId,
  SessionInfo,
  SessionRecord,
  SettingsInfo,
  SpawnRequest,
  Suggestion,
  TermSize,
  ThresholdsDto,
  UntrackedWorktree,
  UpdateStatus,
  Workspace,
  WorkspaceSettingsDto,
  YsStatus,
};

/** Session labels: the workspace a session belongs to, and the harness it runs (if any). */
export const WORKSPACE_LABEL = "workspace";
export const HARNESS_LABEL = "harness";
/** The label naming the session record (conversation) a terminal belongs to, if any. */
export const RECORD_LABEL = "record";
/** On an agent told about the workspace's other conversations: which, e.g. "claude's conversation". */
export const TOLD_ABOUT_LABEL = "toldAbout";

/** False in a plain browser tab and in unit tests, where there is no core to call. */
export const hasCore = isTauri;

export function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as IpcError).code === "string" &&
    typeof (value as IpcError).message === "string"
  );
}

/** A message fit to show the user, whatever was thrown. */
export function errorMessage(error: unknown): string {
  if (isIpcError(error)) return error.message;
  return error instanceof Error ? error.message : String(error);
}

type Outcome<T> = { status: "ok"; data: T } | { status: "error"; error: IpcError };

/**
 * specta types every Rust `f64` as `number | null`, because JSON has no NaN. The usage reports'
 * figures are always finite, so they are narrowed to `number` here, once — except the fields that
 * really are optional, named in `Keep`.
 */
type Finite<T, Keep extends string> = [T] extends [number | null]
  ? number
  : T extends (infer U)[]
    ? Finite<U, Keep>[]
    : T extends object
      ? { [K in keyof T]: K extends Keep ? T[K] : Finite<T[K], Keep> }
      : T;
export type UsageReport = Finite<RawUsageReport, "knownCost" | "resetsAt">;
/** A smoke test run: asked for at `createdAt`, started and ended when it was. */
export type TestRun = Finite<RawTestRun, "startedAt" | "endedAt" | "port">;
export type TestStepRun = Finite<RawTestStepRun, "startedAt" | "endedAt" | "exit">;
export type OutputChunk = Finite<RawOutputChunk, never>;
/** When a schedule fires. A `monthly` with no `day` means the month's last day. */
export type ScheduleSpec = Finite<RawScheduleSpec, "day">;
/** A schedule; `nextAt` is null when it has no time to come. */
export type Schedule = Finite<RawSchedule, "nextAt" | "day">;
export type ScheduleRun = Finite<RawScheduleRun, never>;
export type SchedulePreview = Finite<RawSchedulePreview, never>;
export type ScheduleDraft = Omit<RawScheduleDraft, "spec"> & { spec: ScheduleSpec };
/** A trigger; `decidedAt` on its last line is null while it is gathering. */
export type TriggerItem = Finite<RawTriggerItem, "decidedAt">;
export type TriggerFire = Finite<RawTriggerFire, "decidedAt">;
export type TriggerDraft = RawTriggerDraft;
export type SavedTrigger = Finite<RawSavedTrigger, "decidedAt">;
export type MachineReport = Finite<RawMachineReport, "loadOne">;
export type AgentLimits = UsageReport["limits"][number];
export type ModelUsage = UsageReport["models"][number];
export type PlaceUsage = UsageReport["places"][number];
export type Load = MachineReport["yardsort"];
export type TerminalLoad = MachineReport["loose"][number];

/** Generated commands return a result object; the app prefers exceptions. Throws `IpcError`. */
async function unwrap<T>(outcome: Promise<Outcome<T>>): Promise<T> {
  const result = await outcome;
  if (result.status === "error") throw result.error;
  return result.data;
}

/** For commands that return nothing (Rust's `()` arrives as `null`). */
async function done(outcome: Promise<Outcome<null>>): Promise<void> {
  await unwrap(outcome);
}

export const ipc = {
  appInfo: (): Promise<AppInfo> => commands.appInfo(),
  benchReport: async (report: string): Promise<void> => void (await commands.benchReport(report)),
  envInfo: (reload = false) => unwrap(commands.envInfo(reload)),
  /** Is git here? Which agents? With `reload`, the login shell is asked again first. */
  preflight: (reload = false) => unwrap(commands.preflight(reload)),
  /** Where `ys` is on `PATH`, and whether it is this version. */
  ysStatus: () => unwrap(commands.ysStatus()),
  /** Put `ys` on `PATH`. Rejects with `ys_exists` when a file that is not a `ys` is in the way,
   *  unless `replace` is set — ask the user first. */
  ysInstall: (replace = false) => unwrap(commands.ysInstall(replace)),

  projectsList: () => unwrap(commands.projectsList()),
  /** Rejects with code `not_a_git_repo` unless `initGit` is set. */
  projectOpen: (path: string, initGit = false) => unwrap(commands.projectOpen(path, initGit)),
  /** `upstream` is a fork's parent, which becomes the clone's `upstream` remote. */
  projectClone: (repository: string, name: string, parent: string, upstream: string | null) =>
    unwrap(commands.projectClone(repository, name, parent, upstream)),
  /** The repositories the `gh` account owns or collaborates on, with the ones that are already
   *  projects marked. Fails only without `gh`; a page that fails is said in `problem`. */
  forgeRepositories: () => unwrap(commands.forgeRepositories()),
  /** Repositories anywhere on the forge whose name contains `text`, the twenty it ranks first. */
  forgeSearchRepositories: (text: string) => unwrap(commands.forgeSearchRepositories(text)),
  /** A pasted link to an issue or a pull request, settled: its repository's project if any,
   *  and which of the two it is. Fails with `invalid_link` for anything else. */
  forgeResolveLink: (text: string) => unwrap(commands.forgeResolveLink(text)),
  projectCreate: (name: string, parent: string) => unwrap(commands.projectCreate(name, parent)),
  /** Take a project off the list. With `keepHistory` it comes back whole when opened again. */
  projectRemove: (id: string, keepHistory: boolean) =>
    done(commands.projectRemove(id, keepHistory)),
  projectsReorder: (orderedIds: string[]) => done(commands.projectsReorder(orderedIds)),
  uiStateLoad: () => unwrap(commands.uiStateLoad()),
  uiStateSave: (key: string, value: string) => done(commands.uiStateSave(key, value)),

  harnessesList: () => unwrap(commands.harnessesList()),
  /** Save a definition; built-ins keep only their differences. Resolves to the new list. */
  harnessSave: (def: HarnessDef) => unwrap(commands.harnessSave(def)),
  /** Restore a built-in, or delete a custom harness. Resolves to the new list. */
  harnessReset: (id: string) => unwrap(commands.harnessReset(id)),
  harnessPreview: (def: HarnessDef) => unwrap(commands.harnessPreview(def)),
  /** Start a (possibly unsaved) definition with no prompt, to see whether it comes up. */
  harnessTest: (def: HarnessDef, size: TermSize) => unwrap(commands.harnessTest(def, size)),
  settingsGet: () => unwrap(commands.settingsGet()),
  settingsSaveWorkspaces: (workspaces: WorkspaceSettingsDto) =>
    unwrap(commands.settingsSaveWorkspaces(workspaces)),
  projectBranches: (projectId: string) => unwrap(commands.projectBranches(projectId)),
  /** Worktree + branch + harness in one step; leaves nothing behind if any part fails. */
  workspaceCreate: (request: NewWorkspace) => unwrap(commands.workspaceCreate(request)),
  /** Rejects with code `worktree_dirty` unless `force` is set. The branch is always kept. */
  workspaceDelete: (id: string, force = false) => done(commands.workspaceDelete(id, force)),

  settingsSaveGeneral: (general: {
    editorCommand: string | null;
    notifyWhenQuiet: boolean;
    checkForUpdates: boolean;
  }) =>
    unwrap(
      commands.settingsSaveGeneral(
        general.editorCommand,
        general.notifyWhenQuiet,
        general.checkForUpdates,
      ),
    ),

  settingsSaveActivity: (activity: ActivitySettingsDto) =>
    unwrap(commands.settingsSaveActivity(activity)),
  /** Whether Usage is offered at the foot of the sidebar. */
  settingsSaveUsage: (showInSidebar: boolean) => unwrap(commands.settingsSaveUsage(showInSidebar)),

  /** What Yardsort and its agents use of this machine now, with the last five minutes. */
  usageMachine: () => unwrap(commands.usageMachine()) as Promise<MachineReport>,
  /**
   * Tokens the agents spent over the last `days` days, read from their own logs on this
   * machine and priced at API rates. Days are the viewer's own.
   */
  usageTokens: (days: number) =>
    unwrap(commands.usageTokens(days, -new Date().getTimezoneOffset())) as Promise<UsageReport>,

  /**
   * A page of a workspace's recorded activity, newest first: events before `beforeSeq`, or the
   * newest when it is null. What Yardsort itself saw — starts and exits — and, when capture is
   * on, what Claude Code reported through its hooks. Every event says which.
   */
  activityTimeline: (workspaceId: string, beforeSeq: number | null = null, limit = 50) =>
    unwrap(commands.activityTimeline(workspaceId, beforeSeq, limit)),
  /** New events were recorded for these workspaces: a timeline showing one should reload. */
  onActivityChanged: (handler: (workspaceIds: string[]) => void) =>
    events.activityChanged.listen((event) => handler(event.payload.workspaceIds)),
  /** An agent was seen opening a pull request in another repository, and a workspace in each
   *  of these projects now follows it: their pull requests are worth asking about again. */
  onTrackedPullRequestsChanged: (handler: (projectIds: string[]) => void) =>
    events.trackedPullRequestsChanged.listen((event) => handler(event.payload.projectIds)),
  /** How much is recorded, where the spool and the inbox are, and what went wrong recording. */
  activityDiagnostics: () => unwrap(commands.activityDiagnostics()),
  /** Forget recorded activity: one workspace's, or all of it for `null`. */
  activityClear: (workspaceId: string | null) => done(commands.activityClear(workspaceId)),
  /** The handoff packet for a workspace: what Yardsort recorded, as the next agent's prompt. */
  workspaceHandoff: (workspaceId: string) => unwrap(commands.workspaceHandoff(workspaceId)),
  /** The fork packet for a workspace: the same record, for an agent in a new workspace on a
   *  new branch made from its branch. */
  workspaceFork: (workspaceId: string) => unwrap(commands.workspaceFork(workspaceId)),
  /**
   * Which of a workspace's files its agents reported writing, which runs could have, and for
   * each of `paths` (the change list's files) whether its last write fell inside a tool call.
   */
  workspaceProvenance: (workspaceId: string, paths: string[]) =>
    unwrap(commands.workspaceProvenance(workspaceId, paths)),

  /** Is there a newer release? Looks only; nothing is downloaded. */
  updateCheck: () => unwrap(commands.updateCheck()),
  /** Download, verify and install the update found by the last check, then restart. */
  updateInstall(onProgress: (progress: DownloadProgress) => void): Promise<void> {
    const channel = new Channel<DownloadProgress>();
    channel.onmessage = onProgress;
    return done(commands.updateInstall(channel));
  },

  /** A workspace's harness conversations, newest first. */
  sessionsList: (workspaceId: string) => unwrap(commands.sessionsList(workspaceId)),
  /**
   * How full a conversation's context is, as its agent last reported, and the command that
   * compacts it; `null` when nothing was reported since it started or last compacted.
   */
  sessionContext: (id: string): Promise<ContextUsage | null> => unwrap(commands.sessionContext(id)),
  /** Continue an ended conversation in a new terminal. */
  sessionResume: (id: string, size: TermSize) => unwrap(commands.sessionResume(id, size)),
  /** Start a copy of a conversation that goes its own way. */
  sessionFork: (id: string, size: TermSize) => unwrap(commands.sessionFork(id, size)),
  sessionForget: (id: string) => done(commands.sessionForget(id)),

  /** Remove the worktree but keep branch and history. Rejects with `worktree_dirty` unless forced. */
  workspaceArchive: (id: string, force = false) => done(commands.workspaceArchive(id, force)),
  /** Bring back an archived or vanished workspace at its old path. */
  workspaceRestore: (id: string) => unwrap(commands.workspaceRestore(id)),
  workspaceRename: (id: string, name: string) => unwrap(commands.workspaceRename(id, name)),
  /** Worktrees git knows about in a project that are not workspaces: what Import offers. */
  projectAutomationGet: (projectId: string) => unwrap(commands.projectAutomationGet(projectId)),
  projectAutomationSave: (projectId: string, config: ProjectAutomation) =>
    done(commands.projectAutomationSave(projectId, config)),
  workspaceRun: (workspaceId: string, size: TermSize) =>
    unwrap(commands.workspaceRun(workspaceId, size)),
  projectUntrackedWorktrees: (projectId: string) =>
    unwrap(commands.projectUntrackedWorktrees(projectId)),
  /** Make workspaces of those worktrees, by path. Nothing on disk is touched. */
  workspacesImport: (projectId: string, paths: string[]) =>
    unwrap(commands.workspacesImport(projectId, paths)),
  /** Stop showing a workspace; folder and branch stay, and so does its history when asked. */
  workspaceForget: (id: string, keepHistory: boolean) =>
    done(commands.workspaceForget(id, keepHistory)),

  workspaceChanges: (workspaceId: string) => unwrap(commands.workspaceChanges(workspaceId)),
  /** Both versions of a file; the viewer computes the diff. */
  workspaceDiff: (
    workspaceId: string,
    change: Pick<FileChange, "path" | "oldPath">,
    scope: Scope,
  ) => unwrap(commands.workspaceDiff(workspaceId, change.path, change.oldPath, scope)),
  /** One folder of the file tree; `dir` is relative, empty for the root. */
  workspaceFiles: (workspaceId: string, dir: string, showIgnored = false) =>
    unwrap(commands.workspaceFiles(workspaceId, dir, showIgnored)),
  workspaceSaveFile: (workspaceId: string, path: string, expected: string, text: string) =>
    unwrap(commands.workspaceSaveFile(workspaceId, path, expected, text)),
  workspaceRevealFile: (workspaceId: string, path: string) =>
    done(commands.workspaceRevealFile(workspaceId, path)),
  workspaceFile: (workspaceId: string, path: string) =>
    unwrap(commands.workspaceFile(workspaceId, path)),
  /** Watch one workspace's files (replacing any earlier watch); `null` stops. */
  workspaceWatch: (workspaceId: string | null) => done(commands.workspaceWatch(workspaceId)),
  onWorkspaceFilesChanged: (handler: (workspaceId: string) => void) =>
    events.workspaceFilesChanged.listen((event) => handler(event.payload.workspaceId)),
  /** Open a file — or the workspace folder, for `null` — in the user's editor. */
  openInEditor: (workspaceId: string, path: string | null) =>
    done(commands.openInEditor(workspaceId, path)),

  /**
   * Where a workspace stands with its remote: branch, base, what is unpushed, and the pull
   * request if there is one. `refresh` goes back to `gh` instead of reusing its last answer.
   */
  workspacePublishState: (workspaceId: string, refresh = false) =>
    unwrap(commands.workspacePublishState(workspaceId, refresh)),
  /** Commit everything the workspace has changed. Resolves to where it stands afterwards. */
  workspaceCommit: (workspaceId: string, message: string) =>
    unwrap(commands.workspaceCommit(workspaceId, message)),
  /** Push the branch, setting its upstream the first time. */
  workspacePush: (workspaceId: string) => unwrap(commands.workspacePush(workspaceId)),
  /**
   * Open a pull request, pushing first if it needs it. Without `gh` — or logged out of it — the
   * URL that comes back is the forge's own form rather than a pull request that now exists.
   */
  workspaceOpenPullRequest: (
    workspaceId: string,
    pr: { title: string; body: string; draft: boolean },
  ) => unwrap(commands.workspaceOpenPullRequest(workspaceId, pr.title, pr.body, pr.draft)),
  /** Merge a confirmed PR only if it is still the open PR for this workspace and head. */
  workspaceMergePullRequest: (
    workspaceId: string,
    number: number,
    headOid: string,
    method: MergeMethod,
  ) => unwrap(commands.workspaceMergePullRequest(workspaceId, number, headOid, method)),
  workspaceConflictHelper: (workspaceId: string, number: number) =>
    unwrap(commands.workspaceConflictHelper(workspaceId, number)),
  workspaceResolveConflicts: (
    workspaceId: string,
    number: number,
    sessionId: string,
    size: TermSize,
  ) => unwrap(commands.workspaceResolveConflicts(workspaceId, number, sessionId, size)),

  /** Every pull request `gh` knows for a project, so each workspace row can show its own. */
  /** `full` also reads every open pull request, not only the newest fifty: the Pull requests
   *  view's question, and several round trips on a busy repository. */
  projectPullRequests: (projectId: string, refresh = false, full = false) =>
    unwrap(commands.projectPullRequests(projectId, refresh, full)),
  /** Follow a pull request in another repository with a workspace. Resolves to whether it is
   *  new: tracking one twice changes nothing. Refused for a link that is not a GitHub pull
   *  request, and for one of the project's own repository. */
  workspaceTrackPullRequest: (workspaceId: string, url: string) =>
    unwrap(commands.workspaceTrackPullRequest(workspaceId, url)),
  /** Stop following one. Only Yardsort's note of it goes; the pull request is left as it is. */
  workspaceUntrackPullRequest: (workspaceId: string, id: string) =>
    unwrap(commands.workspaceUntrackPullRequest(workspaceId, id)),
  /** A tracked pull request in full, read by its link. */
  trackedPullRequestSummary: (workspaceId: string, id: string, refresh = false) =>
    unwrap(commands.trackedPullRequestSummary(workspaceId, id, refresh)),
  /** One pull request in full: description, checks with links, reviewers, conversation. */
  pullRequestSummary: (projectId: string, number: number, refresh = false) =>
    unwrap(commands.pullRequestSummary(projectId, number, refresh)),
  /** The files a pull request changes. Fetches its commits first if they are not here. */
  pullRequestChanges: (projectId: string, number: number) =>
    unwrap(commands.pullRequestChanges(projectId, number)),
  /** Both sides of one of those files, between the two commits `pullRequestChanges` named. */
  pullRequestDiff: (
    projectId: string,
    commits: Pick<PullRequestChanges, "baseOid" | "headOid">,
    change: Pick<FileChange, "path" | "oldPath">,
  ) =>
    unwrap(
      commands.pullRequestDiff(
        projectId,
        commits.baseOid,
        commits.headOid,
        change.path,
        change.oldPath,
      ),
    ),
  /** Post a comment on a pull request's conversation. */
  pullRequestComment: (projectId: string, number: number, body: string) =>
    done(commands.pullRequestComment(projectId, number, body)),
  /** Every comment made on lines of a pull request's diff. */
  pullRequestLineComments: (projectId: string, number: number) =>
    unwrap(commands.pullRequestLineComments(projectId, number)),
  /** Post a comment on particular lines of a pull request's diff. */
  pullRequestLineComment: (projectId: string, number: number, place: LinePlace, body: string) =>
    done(commands.pullRequestLineComment(projectId, number, place, body)),
  /** Who in a workspace would be given a note about its pull request's lines, and how. */
  pullRequestNoteHelper: (workspaceId: string, number: number) =>
    unwrap(commands.pullRequestNoteHelper(workspaceId, number)),
  /** Give that agent the note: typed in, resumed with it, or as a new conversation's first message. */
  pullRequestSendNote: (
    workspaceId: string,
    number: number,
    sessionId: string,
    excerpt: Excerpt,
    note: string,
    size: TermSize,
  ) => unwrap(commands.pullRequestSendNote(workspaceId, number, sessionId, excerpt, note, size)),
  /** The note as a composer's first message, for a pull request with no workspace yet. */
  pullRequestNoteText: (projectId: string, number: number, excerpt: Excerpt, note: string) =>
    unwrap(commands.pullRequestNoteText(projectId, number, excerpt, note)),
  /** Merge any pull request of a project, at the head commit the user confirmed. */
  pullRequestMerge: (projectId: string, number: number, headOid: string, method: MergeMethod) =>
    done(commands.pullRequestMerge(projectId, number, headOid, method)),
  /** Close without merging. The branch stays. */
  pullRequestClose: (projectId: string, number: number) =>
    done(commands.pullRequestClose(projectId, number)),
  pullRequestReopen: (projectId: string, number: number) =>
    done(commands.pullRequestReopen(projectId, number)),
  /** Fetch a pull request's branch so the composer can open a workspace on it. */
  pullRequestPrepareBranch: (projectId: string, number: number) =>
    unwrap(commands.pullRequestPrepareBranch(projectId, number)),

  /** A project's tasks: every open one, and with `closed` the most recently closed too. */
  projectTasks: (projectId: string, refresh = false, closed = false) =>
    unwrap(commands.projectTasks(projectId, refresh, closed)),
  /** One task in full: its description and its conversation. */
  taskDetail: (projectId: string, key: string, refresh = false) =>
    unwrap(commands.taskDetail(projectId, key, refresh)),
  /** A task made ready to hand to an agent: the first message, and what to record. */
  taskPrompt: (projectId: string, key: string) => unwrap(commands.taskPrompt(projectId, key)),
  /** Open a new task on the project's source. Public at once. */
  taskCreate: (projectId: string, task: NewTask) => unwrap(commands.taskCreate(projectId, task)),
  /** Post a comment on a task. */
  taskComment: (projectId: string, key: string, body: string) =>
    done(commands.taskComment(projectId, key, body)),
  taskClose: (projectId: string, key: string, reason: CloseReason) =>
    done(commands.taskClose(projectId, key, reason)),
  taskReopen: (projectId: string, key: string) => done(commands.taskReopen(projectId, key)),
  /** Change a task's labels, assignees or title. */
  taskEdit: (projectId: string, key: string, change: TaskEdit) =>
    done(commands.taskEdit(projectId, key, change)),
  /** The labels and people a project's tasks can be given. */
  projectTaskChoices: (projectId: string) => unwrap(commands.projectTaskChoices(projectId)),

  /**
   * Whether a model can write a commit message or a pull request here, and which one would.
   * `harnessId` is the agent the workspace is using, which gets first refusal.
   */
  workflowWriterStatus: () => unwrap(commands.workflowWriterStatus()),
  workflowSaveWriter: (harnessId: string | null) => unwrap(commands.workflowSaveWriter(harnessId)),
  draftStatus: (harnessId: string | null = null) => unwrap(commands.draftStatus(harnessId)),
  /** Have a model write a commit message for what is uncommitted. Sends that diff. */
  draftCommitMessage: (workspaceId: string, harnessId: string | null = null) =>
    unwrap(commands.draftCommitMessage(workspaceId, harnessId)),
  /** Have a model write the pull request. Sends the branch's diff against its base. */
  draftPullRequest: (workspaceId: string, harnessId: string | null = null) =>
    unwrap(commands.draftPullRequest(workspaceId, harnessId)),
  /** Keep an Anthropic API key in the OS credential store. Never read back. */
  draftSaveKey: (key: string) => unwrap(commands.draftSaveKey(key)),
  draftForgetKey: () => unwrap(commands.draftForgetKey()),
  draftSaveSettings: (enabled: boolean, model: string) =>
    unwrap(commands.draftSaveSettings(enabled, model)),

  /**
   * Remote, still under development: this computer as a host on a Yardsort server. The webview never sees a
   * credential, only whether Remote is on, a pairing code while one is shown, and the devices
   * let in. See `src-tauri/src/remote.rs`.
   */
  remoteStatus: () => unwrap(commands.remoteStatus()),
  remoteReplaceDaemon: () => unwrap(commands.remoteReplaceDaemon()),
  /** Sign in through the browser and register this computer. Waits for the browser. */
  remoteEnable: (serverUrl: string, name: string) => unwrap(commands.remoteEnable(serverUrl, name)),
  remoteCancel: () => unwrap(commands.remoteCancel()),
  remoteDisable: () => unwrap(commands.remoteDisable()),
  /** A code to type and a QR code, good once for five minutes. */
  remotePair: () => unwrap(commands.remotePair()),
  /** Devices waiting for an answer, and the ones let in. */
  remoteRequests: () => unwrap(commands.remoteRequests()),
  remoteAnswer: (grantId: string, answer: RemoteAnswer) =>
    unwrap(commands.remoteAnswer(grantId, answer)),
  remoteRevoke: (grantId: string) => unwrap(commands.remoteRevoke(grantId)),
  /** The relay told this host a device asks to be let in, and the server has it waiting. */
  onRemotePairingRequested: (handler: (request: RemoteRequest) => void) =>
    events.remotePairingRequested.listen((event) => handler(event.payload.request)),

  /**
   * Remote the other way round: this computer as a client of other hosts, a device of its own at
   * the server, apart from this computer as a host. What a host shows is read-only and carries no
   * paths. See `src-tauri/src/computers.rs`.
   */
  otherComputers: () => unwrap(commands.otherComputers()),
  otherComputersSignIn: (serverUrl: string, name: string) =>
    unwrap(commands.otherComputersSignIn(serverUrl, name)),
  otherComputersCancel: () => unwrap(commands.otherComputersCancel()),
  otherComputersSignOut: () => unwrap(commands.otherComputersSignOut()),
  otherComputersPair: (code: string) => unwrap(commands.otherComputersPair(code)),
  otherComputersForget: (grantId: string) => unwrap(commands.otherComputersForget(grantId)),
  otherHost: (hostId: string) => unwrap(commands.otherHost(hostId)),
  /**
   * Watch a terminal on another computer. Its bytes and what else it shows come in order on one
   * channel, told apart by their first byte (`src-tauri/src/remote_terminal.rs`). Resolves to the
   * number the other calls take, until `remoteTerminalClose`.
   */
  remoteTerminalOpen(
    hostId: string,
    terminalId: string,
    onOutput: (bytes: Uint8Array) => void,
    onSeen: (seen: RemoteTerminalSeen) => void,
  ): Promise<number> {
    const channel = new Channel<ArrayBuffer>();
    const text = new TextDecoder();
    channel.onmessage = (buffer) => {
      const message = new Uint8Array(buffer);
      if (message[0] === 0) onOutput(message.subarray(1));
      else onSeen(JSON.parse(text.decode(message.subarray(1))) as RemoteTerminalSeen);
    };
    return unwrap(
      commands.remoteTerminalOpen(hostId, terminalId, channel as unknown as Channel<number[]>),
    );
  },
  /** Stop watching; control, if held, goes back to that computer. */
  remoteTerminalClose: (id: number) => done(commands.remoteTerminalClose(id)),
  /** Ask for control; the answer comes as `held` or `notHeld` on its channel. */
  remoteTerminalTakeControl: (id: number) => done(commands.remoteTerminalTakeControl(id)),
  remoteTerminalRelease: (id: number) => done(commands.remoteTerminalRelease(id)),
  /** Type, while control is held; nothing is sent otherwise. */
  remoteTerminalWrite: (id: number, data: string) => done(commands.remoteTerminalWrite(id, data)),
  /** Fit it to the view, while control is held. */
  remoteTerminalResize: (id: number, size: TermSize) =>
    done(commands.remoteTerminalResize(id, size)),
  /** The list, a host's link or what one shows changed. */
  onOtherComputersChanged: (handler: () => void) =>
    events.otherComputersChanged.listen(() => handler()),

  /** Assist: whether a TypeSafe key is in force and which features are on. */
  assistStatus: () => unwrap(commands.assistStatus()),
  /** Check a key with TypeSafe, then keep it in the OS credential store. Never read back. */
  assistSaveKey: (key: string) => unwrap(commands.assistSaveKey(key)),
  assistForgetKey: () => unwrap(commands.assistForgetKey()),
  /** Ask TypeSafe whether the key in force still works. */
  assistTestKey: () => done(commands.assistTestKey()),
  assistSaveSettings: (
    reviewChanges: boolean,
    suggestInComposer: boolean,
    sendProvenance: boolean,
    checkMemory: boolean,
    thresholds: ThresholdsDto,
  ) =>
    unwrap(
      commands.assistSaveSettings(
        reviewChanges,
        suggestInComposer,
        sendProvenance,
        checkMemory,
        thresholds,
      ),
    ),
  /** A project's memory: its entries with their history, and whether its agents are given it. */
  memoryGet: (projectId: string) => unwrap(commands.memoryGet(projectId)),
  /** The user writes an entry; it is approved as written. */
  memoryWrite: (projectId: string, text: string) => unwrap(commands.memoryWrite(projectId, text)),
  memoryEdit: (id: string, text: string) => unwrap(commands.memoryEdit(id, text)),
  memoryDecide: (id: string, decision: MemoryDecision) =>
    unwrap(commands.memoryDecide(id, decision)),
  /** Whether a project's approved entries go into its agents' first messages. */
  memoryShare: (projectId: string, shared: boolean) =>
    unwrap(commands.memoryShare(projectId, shared)),
  /** Let a project's agents read each other's conversations with `ys workspace context`, or not. */
  conversationsShare: (projectId: string, shared: boolean) =>
    unwrap(commands.conversationsShare(projectId, shared)),
  /**
   * The note an agent started in this workspace now would be given about its other
   * conversations, and the digest it would read — `except` the conversation it is, for one
   * already running. `null` when the project does not share them or none has said anything.
   */
  conversationContext: (workspaceId: string, except: string | null) =>
    unwrap(commands.conversationContext(workspaceId, except)),
  /** How many proposals wait, per project that has any. */
  memoryWaiting: () => unwrap(commands.memoryWaiting()),
  /** Jev's word on each waiting proposal: repeats or contradicts an approved entry. */
  memoryCheck: (projectId: string) => unwrap(commands.memoryCheck(projectId)),
  /** A project's attempts with their outcome and its evidence, brought up to date first. */
  outcomesGet: (projectId: string) => unwrap(commands.outcomesGet(projectId)),
  /** The user's word on an attempt — kept, partly, discarded — or null to take it back. */
  outcomeLabel: (id: string, label: string | null) => unwrap(commands.outcomeLabel(id, label)),
  /** Per-agent history across every project, from outcomes only. */
  outcomesAgents: () => unwrap(commands.outcomesAgents()),
  /** Judge a workspace.s changed files against its task. Sends those diffs to TypeSafe. */
  assistReview: (workspaceId: string) => unwrap(commands.assistReview(workspaceId)),
  /** A harness and an effort for a message being typed; empty fields mean "nothing to offer". */
  assistSuggest: (message: string) => unwrap(commands.assistSuggest(message)),

  // --- the Floor Manager -------------------------------------------------------------------

  /** The Floor Manager the daemon holds, running or the last to end, and where it lives. */
  coordinatorStatus: () => unwrap(commands.coordinatorStatus()),
  /** Start it with this harness, or get the one already running. */
  coordinatorStart: (harness: string, size: TermSize) =>
    unwrap(commands.coordinatorStart(harness, size)),
  /** End the running one. Its last screen stays until it is started again. */
  coordinatorStop: () => unwrap(commands.coordinatorStop()),

  ptySpawn: (request: SpawnRequest) => unwrap(commands.ptySpawn(request)),
  ptyList: () => unwrap(commands.ptyList()),
  ptyWrite: (id: SessionId, data: string) => done(commands.ptyWrite(id, data)),
  ptyResize: (id: SessionId, size: TermSize) => done(commands.ptyResize(id, size)),
  ptyKill: (id: SessionId) => done(commands.ptyKill(id)),
  ptyClose: (id: SessionId) => done(commands.ptyClose(id)),
  ptyDetach: (id: SessionId, attachment: number) => done(commands.ptyDetach(id, attachment)),

  /**
   * Stream a session's output: one snapshot that repaints the terminal, then live bytes.
   * Resolves to the attachment id to pass to `ptyDetach`.
   *
   * The core sends raw bytes, which arrive as an `ArrayBuffer`; the generated binding believes
   * they are `number[]` (see `RawBytes` in `terminal.rs`), hence the one cast below.
   */
  ptyAttach(id: SessionId, onOutput: (bytes: Uint8Array) => void): Promise<number> {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buffer) => onOutput(new Uint8Array(buffer));
    return unwrap(commands.ptyAttach(id, channel as unknown as Channel<number[]>));
  },

  // --- workflows ----------------------------------------------------------------------------

  /** Every workflow, built in and the user's, with how many runs of each are going now. */
  workflowList: () => unwrap(commands.workflowList()),
  /** Check a file's text as it is written: problems for the editor, the workflow for the chart. */
  workflowCheck: (text: string): Promise<WorkflowCheck> => commands.workflowCheck(text),
  /** Save a file: `path` is the user's file being edited, `null` for a new one. Its id back. */
  workflowSave: (path: string | null, text: string) => unwrap(commands.workflowSave(path, text)),
  /** Copy into the user's folder; with no `asId`, a built-in's copy replaces it. The copy's id. */
  workflowCopy: (id: string, asId: string | null = null) => unwrap(commands.workflowCopy(id, asId)),
  /** Delete one of the user's files. Ask first. */
  workflowRemove: (path: string) => done(commands.workflowRemove(path)),
  workflowRuns: (workflowId: string | null, limit = 50) =>
    unwrap(commands.workflowRuns(workflowId, limit)),
  workflowRunSteps: (runId: string) => unwrap(commands.workflowRunSteps(runId)),
  /** What a run would find: the pull request, and whether it needs one. */
  workflowPreview: (workflowId: string, workspaceId: string) =>
    unwrap(commands.workflowPreview(workflowId, workspaceId)),
  /** Start a run; its id back. Rejects with what is wrong, as `ys workflow run` does. */
  workflowStart: (workflowId: string, workspaceId: string, inputs: Record<string, string>) =>
    unwrap(commands.workflowStart(workflowId, workspaceId, inputs)),
  workflowCancel: (runId: string) => unwrap(commands.workflowCancel(runId)),
  /** A model writes a workflow from a description, through Drafting; checked, never saved. */
  workflowDescribe: (description: string): Promise<Described> =>
    unwrap(commands.workflowDescribe(description)),
  /** A run was queued, moved on, finished or cancelled. */
  onWorkflowRunsChanged: (handler: () => void) =>
    events.workflowRunsChanged.listen(() => handler()),

  // --- smoke tests ---------------------------------------------------------------------
  /** A workspace's smoke tests: every file in its worktree's `.yardsort/tests`. */
  testList: (workspaceId: string) => unwrap(commands.testList(workspaceId)),
  /** A project's smoke tests as its own checkout has them, for a workspace not made yet. */
  testListInProject: (projectId: string) => unwrap(commands.testListInProject(projectId)),
  /** Every test in every workspace on show, with where it lives and how it last ran. */
  testCatalog: (): Promise<TestListing[]> =>
    unwrap(commands.testCatalog()) as Promise<TestListing[]>,
  /** A model writes a smoke test from a description, through Drafting; checked, never saved. */
  testDescribe: (description: string): Promise<TestDescribed> =>
    unwrap(commands.testDescribe(description)) as Promise<TestDescribed>,
  /** Save a described test into a workspace's `.yardsort/tests`; returns the file's path. */
  testSave: (workspaceId: string, text: string): Promise<string> =>
    unwrap(commands.testSave(workspaceId, text)) as Promise<string>,
  /** Runs, newest first: of one workspace, or of all. */
  testRuns: (workspaceId: string | null, limit = 100): Promise<TestRun[]> =>
    unwrap(commands.testRuns(workspaceId, limit)) as Promise<TestRun[]>,
  /** Every run not ended yet, so a waiting or running one is never crowded out of the list. */
  testActiveRuns: (): Promise<TestRun[]> => unwrap(commands.testActiveRuns()) as Promise<TestRun[]>,
  /** Particular runs by id, skipping any that are gone: for the window's open run tabs. */
  testRunsById: (ids: string[]): Promise<TestRun[]> =>
    unwrap(commands.testRunsById(ids)) as Promise<TestRun[]>,
  testRunSteps: (runId: string): Promise<TestStepRun[]> =>
    unwrap(commands.testRunSteps(runId)) as Promise<TestStepRun[]>,
  /**
   * Start a run from the Run dialog, which showed the steps: pressing Run approves the file the
   * dialog showed. `contentHash` is that file's hash (from `testReview`); if the file changed
   * since, the run waits for the approval card instead of running unseen.
   */
  testRunStart: (
    testId: string,
    workspaceId: string,
    inputs: Record<string, string>,
    contentHash: string,
  ) => unwrap(commands.testRunStart(testId, workspaceId, inputs, contentHash)),
  /** A test as the Run dialog shows it: steps in words, environment names, what changed. */
  testReview: (workspaceId: string, testId: string) =>
    unwrap(commands.testReview(workspaceId, testId)),
  /** A waiting run's file the same way, as it was when the run was asked for. */
  testRunReview: (runId: string) => unwrap(commands.testRunReview(runId)),
  /** The user saw the file and lets it run. False when the run was not waiting. */
  testRunApprove: (runId: string) => unwrap(commands.testRunApprove(runId)),
  /** The user chose not to run the file. */
  testRunRefuse: (runId: string) => unwrap(commands.testRunRefuse(runId)),
  /** Stop a run: at once when it has not started, after its teardown when it is running. */
  testRunCancel: (runId: string) => unwrap(commands.testRunCancel(runId)),
  /** Pass or fail a `manual` step the run waits on. */
  testStepAnswer: (runId: string, stepId: string, passed: boolean, note: string | null) =>
    unwrap(commands.testStepAnswer(runId, stepId, passed, note)),
  /** A piece of a step's `stdout` or `stderr` from `from`; the tab tails with this. */
  testStepOutput: (
    runId: string,
    stepId: string,
    stream: "stdout" | "stderr",
    from: number,
  ): Promise<OutputChunk> =>
    unwrap(commands.testStepOutput(runId, stepId, stream, from)) as Promise<OutputChunk>,
  /** Type the run's result into an agent's terminal, now or once it is quiet. */
  testRunSend: (runId: string, sessionId: string) => unwrap(commands.testRunSend(runId, sessionId)),
  testEnvGet: (projectId: string) => unwrap(commands.testEnvGet(projectId)),
  testEnvSave: (projectId: string, entries: TestEnvEntry[]) =>
    done(commands.testEnvSave(projectId, entries)),
  testSettingsGet: (projectId: string) => unwrap(commands.testSettingsGet(projectId)),
  testSettingsSave: (projectId: string, settings: TestSettings) =>
    done(commands.testSettingsSave(projectId, settings)),
  /** A run was queued, approved, moved on, finished or cancelled, or a step changed. */
  onTestRunsChanged: (handler: () => void) => events.testRunsChanged.listen(() => handler()),
  /** A step's output file grew. */
  onTestRunOutput: (handler: (runId: string, stepId: string) => void) =>
    events.testRunOutput.listen((event) => handler(event.payload.runId, event.payload.stepId)),
  /** A result sent to an agent went in, or never will. */
  onTestResultSent: (handler: (sent: TestResultSent) => void) =>
    events.testResultSent.listen((event) => handler(event.payload)),

  // --- schedules -----------------------------------------------------------------------
  /** Every schedule of every project, by name, each with the newest line of its history. */
  scheduleList: (): Promise<Schedule[]> => unwrap(commands.scheduleList()) as Promise<Schedule[]>,
  /** A schedule's history, newest first: every time it came due and what became of it. */
  scheduleRuns: (scheduleId: string, limit = 50): Promise<ScheduleRun[]> =>
    unwrap(commands.scheduleRuns(scheduleId, limit)) as Promise<ScheduleRun[]>,
  /** The runs schedules started, for a run list to mark. */
  scheduleStartedRuns: () => unwrap(commands.scheduleStartedRuns()),
  /** What a spec comes to as it is filled in: what is wrong, how it reads, its next times. */
  schedulePreview: (spec: ScheduleSpec): Promise<SchedulePreview> =>
    unwrap(commands.schedulePreview(spec)) as Promise<SchedulePreview>,
  /** Add a schedule, or change the one `draft.id` names. Rejects with what is wrong with it. */
  scheduleSave: (draft: ScheduleDraft): Promise<Schedule> =>
    unwrap(commands.scheduleSave(draft)) as Promise<Schedule>,
  /** Switch a schedule on, from now, or off. */
  scheduleSetEnabled: (scheduleId: string, enabled: boolean): Promise<Schedule> =>
    unwrap(commands.scheduleSetEnabled(scheduleId, enabled)) as Promise<Schedule>,
  /** Remove a schedule and its history. The runs it started stay. */
  scheduleRemove: (scheduleId: string) => done(commands.scheduleRemove(scheduleId)),
  /** Start a schedule's run now, leaving its next time where it is. */
  scheduleRunNow: (scheduleId: string): Promise<ScheduleRun> =>
    unwrap(commands.scheduleRunNow(scheduleId)) as Promise<ScheduleRun>,
  /** A schedule was added, changed, switched, removed or fired, or a run it started ended. */
  onSchedulesChanged: (handler: () => void) => events.schedulesChanged.listen(() => handler()),

  // --- triggers ------------------------------------------------------------------------
  /** Every trigger of every project, by name, each with the newest decided line of its history. */
  triggerList: (): Promise<TriggerItem[]> =>
    unwrap(commands.triggerList()) as Promise<TriggerItem[]>,
  /** A trigger's history, newest first: every fire and what became of it. */
  triggerFires: (triggerId: string, limit = 50): Promise<TriggerFire[]> =>
    unwrap(commands.triggerFires(triggerId, limit)) as Promise<TriggerFire[]>,
  /** The runs triggers started, for a run list to mark. */
  triggerStartedRuns: () => unwrap(commands.triggerStartedRuns()),
  /** The ready-made triggers, with the one question each asks. */
  triggerPresets: (): Promise<TriggerPreset[]> => commands.triggerPresets(),
  /** Add a trigger, or change the one `draft.id` names. Rejects with what is wrong with it. */
  triggerSave: (draft: TriggerDraft): Promise<SavedTrigger> =>
    unwrap(commands.triggerSave(draft)) as Promise<SavedTrigger>,
  /** Tell these running agents about `ys workspace done`, each once it is quiet. */
  triggerTellAgents: (sessionIds: string[]) => unwrap(commands.triggerTellAgents(sessionIds)),
  /** Switch a trigger on, from now, or off. */
  triggerSetEnabled: (triggerId: string, enabled: boolean): Promise<TriggerItem> =>
    unwrap(commands.triggerSetEnabled(triggerId, enabled)) as Promise<TriggerItem>,
  /** Lift a trigger's cap, so it starts runs again. */
  triggerReset: (triggerId: string): Promise<TriggerItem> =>
    unwrap(commands.triggerReset(triggerId)) as Promise<TriggerItem>,
  /** Remove a trigger and its history. The runs it started stay. */
  triggerRemove: (triggerId: string) => done(commands.triggerRemove(triggerId)),
  /** A trigger was added, changed, switched or removed, gathered an event or fired, or a run it
   *  started ended. */
  onTriggersChanged: (handler: () => void) => events.triggersChanged.listen(() => handler()),

  onHostEvent: (handler: (event: HostEvent) => void) =>
    events.ptyHostEvent.listen((event) => handler(event.payload)),
  /** A workflow run started a session the window did not ask for; it should get a tab. */
  onSessionStarted: (handler: (workspaceId: string) => void) =>
    events.sessionStarted.listen((event) => handler(event.payload.workspaceId)),

  /** Where this app's terminals actually live: the daemon, or this process. */
  daemonStatus: () => unwrap(commands.daemonStatus()),
  /**
   * Closing the window asks the core first, which answers with this when agents are still
   * working. The ids are PTY sessions, matching the terminal tabs.
   */
  onQuitRequested: (handler: (agents: string[]) => void) =>
    events.quitRequested.listen((event) => handler(event.payload.agents)),
  /** Quit for real. `stopAgents` stops everything the daemon runs; otherwise it carries on. */
  appQuit: (stopAgents: boolean) => done(commands.appQuit(stopAgents)),
  /** The user changed their mind, so the next close should ask again. */
  quitCancelled: () => done(commands.quitCancelled()),
};
