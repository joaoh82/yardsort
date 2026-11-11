// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The remote protocol: what a device asks a computer through the relay, and what it answers. The
 * types follow `crates/remote-proto` (`lib.rs`, `model.rs`, `terminal.rs`, `grant.rs`, `ids.rs`),
 * which is the source; the fixtures in `crates/remote-proto/fixtures/v1/` hold both to the same
 * wire.
 *
 * Every message from a computer is decoded as untrusted, though it comes from one the person
 * paired: ids are checked, and names, labels, notes, links and lists are held to the protocol's
 * limits. Strings the protocol puts no limit on are bounded by the message's own,
 * {@link MAX_CONTROL_FRAME}.
 */

import {
  array,
  bool,
  byOrder,
  chars,
  DecodeError,
  type Decoder,
  hasControl,
  i64,
  isObject,
  nullable,
  object,
  oneOf,
  optional,
  orElse,
  set,
  tagged,
  text,
  u16,
  u32,
  u64,
  utf8Length,
  variant,
} from "./decode.ts";
import {
  MAX_ARTIFACT_PREVIEW,
  MAX_CLIENT_NAME,
  MAX_CODE_LEN,
  MAX_CURSOR_LEN,
  MAX_ID_LEN,
  MAX_INPUT_BYTES,
  MAX_LABEL,
  MAX_LINK,
  MAX_NOTE,
  MAX_PAGE,
  MAX_PROMPT,
  MAX_TASKS,
  MAX_TERMINALS,
  MAX_TEXT,
  MAX_UPLOAD,
  MAX_UPLOAD_CHUNK,
  MAX_UPLOAD_NAME,
  PROTOCOL_MAJOR,
  PROTOCOL_MINOR,
} from "./limits.ts";

// --- ids -----------------------------------------------------------------------------------

/**
 * Each kind of id is its own type, so a workspace's cannot be handed to something expecting a
 * project's. None of them is authorization. On the wire each is a string of at most
 * {@link MAX_ID_LEN} ASCII letters, digits and `-_.:`, which keeps paths, whitespace and control
 * characters out.
 */
type Id<K extends string> = string & { readonly __id: K };
export type ServerId = Id<"server">;
export type DeviceId = Id<"device">;
export type HostId = Id<"host">;
export type ProjectId = Id<"project">;
export type WorkspaceId = Id<"workspace">;
export type SessionRecordId = Id<"session">;
export type RunId = Id<"run">;
export type TerminalId = Id<"terminal">;
export type MemoryId = Id<"memory">;
export type EventId = Id<"event">;
export type LeaseId = Id<"lease">;
export type GrantId = Id<"grant">;
export type RequestId = Id<"request">;

const ID = /^[A-Za-z0-9._:-]+$/;

/** Whether `text` is acceptable as an id of any kind. */
export function isId(text: string): boolean {
  return text.length > 0 && text.length <= MAX_ID_LEN && ID.test(text);
}

/** `text` as an id of kind `K`, after checking it is one. */
export function asId<T extends Id<string>>(text: string): T {
  if (!isId(text)) throw new DecodeError("", "is not an id");
  return text as T;
}

export function id<T extends Id<string>>(): Decoder<T> {
  return (value, path) => {
    if (typeof value !== "string") throw new DecodeError(path, "is not a string");
    if (!isId(value)) throw new DecodeError(path, "is not an id");
    return value as T;
  };
}

/**
 * An opaque position in a list, from one page's reply, for the next page's request. Kept and sent
 * back, never built or read.
 */
export type Cursor = string & { readonly __cursor: true };

const CURSOR = /^[\x21-\x7e]+$/;

export const cursor: Decoder<Cursor> = (value, path) => {
  if (typeof value !== "string") throw new DecodeError(path, "is not a string");
  if (value.length > MAX_CURSOR_LEN || !CURSOR.test(value)) {
    throw new DecodeError(path, "is not a cursor");
  }
  return value as Cursor;
};

// --- versions, errors, greetings ---------------------------------------------------------------

export interface ProtocolVersion {
  major: number;
  minor: number;
}

export const CURRENT: ProtocolVersion = { major: PROTOCOL_MAJOR, minor: PROTOCOL_MINOR };

export const protocolVersion: Decoder<ProtocolVersion> = object({ major: u16, minor: u16 });

/** A stable reason a request failed. Branch on this, never on the message. */
export const ERROR_CODES = [
  "unauthenticated",
  "forbidden",
  "not_found",
  "host_offline",
  "session_ended",
  "control_conflict",
  "lease_lost",
  "resnapshot_required",
  "cursor_expired",
  "rate_limited",
  "protocol_incompatible",
  "invalid_request",
  "stale",
  "unsupported",
  "internal",
  "unknown",
] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

/** Whether the same request might succeed later without anything else changing. */
export function retryable(code: ErrorCode): boolean {
  return code === "host_offline" || code === "rate_limited" || code === "internal";
}

/**
 * A failed request: a code to branch on, a message safe to show a person, and whether trying
 * again could help. A code from a newer peer reads as `unknown`.
 */
export interface RemoteError {
  code: ErrorCode;
  message: string;
  retryable: boolean;
  /** With `protocol_incompatible`: the majors that side speaks. */
  supportedProtocols?: number[];
}

export const remoteError: Decoder<RemoteError> = object({
  code: oneOf(ERROR_CODES, "unknown"),
  message: text(),
  retryable: bool,
  supportedProtocols: optional(array(u16)),
});

/** The first thing a client says on a connection: who it is and which majors it speaks. */
export interface Hello {
  /** For a host's list of connected devices and for support: `yardsort-mobile/0.1.0 (ios)`. */
  client: string;
  protocols: number[];
}

export const hello: Decoder<Hello> = object({ client: text(), protocols: array(u16) });

/** The protocol to speak with a client, or why there is none, as a host decides it. */
export function negotiate(greeting: Hello): ProtocolVersion | RemoteError {
  if (utf8Length(greeting.client) > MAX_CLIENT_NAME || greeting.protocols.length > 16) {
    return {
      code: "invalid_request",
      message: "The greeting is larger than any client sends.",
      retryable: false,
    };
  }
  return greeting.protocols.includes(PROTOCOL_MAJOR) ? CURRENT : incompatible();
}

export function incompatible(): RemoteError {
  return {
    code: "protocol_incompatible",
    message:
      "This host speaks a different version of the remote protocol. Update the app that is older.",
    retryable: false,
    supportedProtocols: [PROTOCOL_MAJOR],
  };
}

// --- the host and its work ---------------------------------------------------------------------

export const PLATFORMS = ["linux", "macos", "windows", "ios", "android", "other"] as const;
/** What a host or a device runs, to label it. One from a newer peer reads as `other`. */
export type Platform = (typeof PLATFORMS)[number];
export const platform = oneOf(PLATFORMS, "other");

/** `host.describe`. */
export interface HostDescription {
  host: HostId;
  name: string;
  platform: Platform;
  /** The Yardsort release there. For display; compatibility is `protocol`'s. */
  appVersion: string;
  protocol: ProtocolVersion;
  /** The operations it answers. */
  capabilities: string[];
  /** Present when it lists terminals it cannot share yet: why. Minor 4. */
  terminalsLater?: TerminalsLater;
}

export interface TerminalsLater {
  reason: "earlierDaemon" | "unknown";
  message: string;
}

const terminalsLater: Decoder<TerminalsLater> = object({
  reason: oneOf(["earlierDaemon", "unknown"], "unknown"),
  message: text(),
});

const hostFields = {
  host: id<HostId>(),
  name: text(MAX_TEXT),
  platform,
  appVersion: text(),
  protocol: protocolVersion,
  capabilities: array(text()),
  terminalsLater: optional(terminalsLater),
};

export const hostDescription: Decoder<HostDescription> = object(hostFields);

export interface Welcome {
  protocol: ProtocolVersion;
  host: HostDescription;
}

/** The host's answer to a `Hello`. After an error, it closes the connection. */
export type HelloReply = { welcome: Welcome } | { error: RemoteError };

export const helloReply: Decoder<HelloReply> = (value, path) => {
  if (isObject(value) && "welcome" in value) {
    return {
      welcome: object({ protocol: protocolVersion, host: hostDescription })(
        value.welcome,
        path ? `${path}.welcome` : "welcome",
      ),
    };
  }
  if (isObject(value) && "error" in value) {
    return { error: remoteError(value.error, path ? `${path}.error` : "error") };
  }
  throw new DecodeError(path, "is neither a welcome nor an error");
};

export interface ProjectSummary {
  id: ProjectId;
  name: string;
  /** Its folder is gone from the host. */
  missing: boolean;
}

const projectSummary: Decoder<ProjectSummary> = object({
  id: id<ProjectId>(),
  name: text(MAX_TEXT),
  missing: bool,
});

export interface BranchRef {
  /** A branch, or an abbreviated commit when detached. */
  label: string;
  detached: boolean;
  unborn: boolean;
}

export interface TaskLink {
  source: string;
  key: string;
  title: string;
  url: string;
}

const link: Decoder<string> = (value, path) => {
  const url = text()(value, path);
  if (utf8Length(url) > MAX_LINK || hasControl(url)) {
    throw new DecodeError(path, "is not a link the protocol carries");
  }
  return url;
};

export interface WorkspaceSummary {
  id: WorkspaceId;
  project: ProjectId;
  name: string;
  kind: "local" | "worktree";
  /** `null` when the folder is not on disk, or git would not say. */
  branch: BranchRef | null;
  archived: boolean;
  missing: boolean;
  tasks: TaskLink[];
  /** Conversations whose record says running, when the list was made. */
  runningSessions: number;
}

const workspaceSummary: Decoder<WorkspaceSummary> = object({
  id: id<WorkspaceId>(),
  project: id<ProjectId>(),
  name: text(MAX_TEXT),
  kind: oneOf(["local", "worktree"]),
  branch: nullable(object({ label: text(MAX_TEXT), detached: bool, unborn: bool })),
  archived: bool,
  missing: bool,
  tasks: array(
    object({
      source: text(MAX_LABEL),
      key: text(MAX_TEXT),
      title: text(MAX_TEXT),
      url: link,
    }),
    MAX_TASKS,
  ),
  runningSessions: u32,
});

export interface WorkspaceDetail {
  workspace: WorkspaceSummary;
  project: ProjectSummary;
}

const workspaceDetailFields = { workspace: workspaceSummary, project: projectSummary };

export interface TermSize {
  cols: number;
  rows: number;
}

export const termSize: Decoder<TermSize> = object({ cols: u16, rows: u16 });

/**
 * What a terminal has been doing, from when it last printed, never from what it printed. `quiet`
 * means only that nothing printed for a while: not that a program waits for an answer.
 */
export type TerminalActivity = "starting" | "busy" | "quiet" | "exited";

export interface TerminalSummary {
  id: TerminalId;
  activity: TerminalActivity;
  idleMs: number;
  /** The size the program draws at. A viewer draws at this size. */
  size: TermSize;
  exitCode: number | null;
}

const terminalSummary: Decoder<TerminalSummary> = object({
  id: id<TerminalId>(),
  activity: oneOf(["starting", "busy", "quiet", "exited"]),
  idleMs: u32,
  size: termSize,
  exitCode: nullable(u32),
});

/** Whether a conversation's process is there, as far as its host can tell. */
export type ProcessState = "running" | "ended" | "gone" | "unknown";

/** One agent conversation. */
export interface SessionSummary {
  id: SessionRecordId;
  workspace: WorkspaceId;
  harness: string;
  model: string | null;
  effort: string | null;
  /**
   * The tab title the desktop shows. It may quote the person's first message: shown, and never
   * logged, cached or sent anywhere but the screen.
   */
  title: string;
  forkedFrom: SessionRecordId | null;
  startedAt: number;
  endedAt: number | null;
  exitCode: number | null;
  process: ProcessState;
  terminal: TerminalSummary | null;
  /**
   * `session.resume` would start it again: it has ended, and the computer can resume its agent's
   * conversation. Minor 6; an earlier computer says nothing, which reads as `false`.
   */
  resumable: boolean;
}

const sessionSummary: Decoder<SessionSummary> = object({
  id: id<SessionRecordId>(),
  workspace: id<WorkspaceId>(),
  harness: text(MAX_LABEL),
  model: nullable(text(MAX_LABEL)),
  effort: nullable(text(MAX_LABEL)),
  title: text(MAX_TEXT),
  forkedFrom: nullable(id<SessionRecordId>()),
  startedAt: i64,
  endedAt: nullable(i64),
  exitCode: nullable(i64),
  process: oneOf(["running", "ended", "gone", "unknown"]),
  terminal: nullable(terminalSummary),
  resumable: orElse(bool, () => false),
});

/** What a terminal outside a recorded conversation runs. One from a newer host is `other`. */
export type TerminalPurpose = "shell" | "runCommand" | "program" | "agent" | "other";

export interface WorkspaceTerminal {
  purpose: TerminalPurpose;
  /** The program's name, without its folder or arguments. */
  program: string;
  harness: string | null;
  terminal: TerminalSummary;
}

const workspaceTerminal: Decoder<WorkspaceTerminal> = object({
  purpose: oneOf(["shell", "runCommand", "program", "agent", "other"], "other"),
  program: text(MAX_LABEL),
  harness: nullable(text(MAX_LABEL)),
  terminal: terminalSummary,
});

/** An approved memory entry: the only kind a device is shown. */
export interface MemoryNote {
  id: MemoryId;
  project: ProjectId;
  text: string;
  author: string;
  harness: string | null;
  createdAt: number;
  updatedAt: number;
}

const memoryNote: Decoder<MemoryNote> = object({
  id: id<MemoryId>(),
  project: id<ProjectId>(),
  text: text(MAX_NOTE),
  author: text(MAX_LABEL),
  harness: nullable(text(MAX_LABEL)),
  createdAt: i64,
  updatedAt: i64,
});

/** `workspace.list`. A page without `next` is the last. */
export interface WorkspacePage {
  projects: ProjectSummary[];
  workspaces: WorkspaceSummary[];
  next: Cursor | null;
}

const workspacePageFields = {
  projects: array(projectSummary, MAX_PAGE),
  workspaces: array(workspaceSummary, MAX_PAGE),
  next: nullable(cursor),
};

/** `session.list`, most recently started first. */
export interface SessionPage {
  sessions: SessionSummary[];
  /** The workspace's other terminals, on the first page only. */
  terminals: WorkspaceTerminal[];
  next: Cursor | null;
}

const sessionPageFields = {
  sessions: array(sessionSummary, MAX_PAGE),
  terminals: orElse(array(workspaceTerminal, MAX_TERMINALS), () => []),
  next: nullable(cursor),
};

/** `memory.list`, most recently written first. */
export interface MemoryPage {
  notes: MemoryNote[];
  next: Cursor | null;
}

const memoryPageFields = { notes: array(memoryNote, MAX_PAGE), next: nullable(cursor) };

/** Who recorded an event, how, and how far to trust it. */
export interface Provenance {
  producer: string;
  method: string;
  fidelity: string;
}

/**
 * What an event says, field by field: each optional, and which ones an event has depends on its
 * kind and on who recorded it. Never anything read from a terminal.
 */
export interface ActivityDetail {
  harness?: string;
  runKind?: string;
  program?: string;
  model?: string;
  continuation?: string;
  launchedBy?: string;
  capture?: string;
  forkedFrom?: SessionRecordId;
  exitCode?: number;
  success?: boolean;
  signal?: string;
  via?: string;
  reason?: string;
  tool?: string;
  subagentType?: string;
  /** A file, relative to the workspace's folder. */
  path?: string;
  pathOutsideWorkspace?: boolean;
  change?: string;
  status?: string;
  durationMs?: number;
  chars?: number;
  decision?: string;
  waitMs?: number;
  notification?: string;
  errorType?: string;
  agentType?: string;
  trigger?: string;
  outcome?: string;
  turnNumber?: number;
  contextTokens?: number;
  contextWindow?: number;
  totalTokens?: number;
  outputTokens?: number;
  note?: string;
}

const word = optional(text());
const count = optional(u64);

const activityDetail: Decoder<ActivityDetail> = object({
  harness: word,
  runKind: word,
  program: word,
  model: word,
  continuation: word,
  launchedBy: word,
  capture: word,
  forkedFrom: optional(id<SessionRecordId>()),
  exitCode: optional(i64),
  success: optional(bool),
  signal: word,
  via: word,
  reason: word,
  tool: word,
  subagentType: word,
  path: word,
  pathOutsideWorkspace: optional(bool),
  change: word,
  status: word,
  durationMs: count,
  chars: count,
  decision: word,
  waitMs: count,
  notification: word,
  errorType: word,
  agentType: word,
  trigger: word,
  outcome: word,
  turnNumber: count,
  contextTokens: count,
  contextWindow: count,
  totalTokens: count,
  outputTokens: count,
  note: word,
});

/** One recorded fact about a workspace's processes and agents. Show a kind you do not know by its name. */
export interface ActivityEvent {
  id: EventId;
  workspace: WorkspaceId;
  session: SessionRecordId | null;
  run: RunId | null;
  /** The producer's clock. Order by the page, not this. */
  occurredAt: number;
  kind: string;
  source: Provenance;
  detail: ActivityDetail;
}

const activityEvent: Decoder<ActivityEvent> = object({
  id: id<EventId>(),
  workspace: id<WorkspaceId>(),
  session: nullable(id<SessionRecordId>()),
  run: nullable(id<RunId>()),
  occurredAt: i64,
  kind: text(),
  source: object({ producer: text(), method: text(), fidelity: text() }),
  detail: activityDetail,
});

/** `activity.list`: events oldest first, whichever way the request walked. */
export interface ActivityPage {
  events: ActivityEvent[];
  older: Cursor | null;
  newer: Cursor | null;
}

const activityPageFields = {
  events: array(activityEvent, MAX_PAGE),
  older: nullable(cursor),
  newer: nullable(cursor),
};

/** One activity subscription on one connection. Chosen by the host. */
export type SubscriptionId = number;

export interface Subscribed {
  subscription: SubscriptionId;
  newer: Cursor | null;
}

const subscribedFields = { subscription: u32, newer: nullable(cursor) };

/** `activity.events`: what a subscription's scope recorded next, oldest first. */
export interface ActivityBatch {
  subscription: SubscriptionId;
  events: ActivityEvent[];
  /** Where this batch got to: subscribe after it again, or read on from it. */
  newer: Cursor;
}

/** `activity.ended`: the host stopped a subscription, and why. */
export interface SubscriptionEnded {
  subscription: SubscriptionId;
  error: RemoteError;
}

// --- terminals ---------------------------------------------------------------------------------

/** One attachment's output stream on one connection. Chosen by the host. */
export type StreamId = number;

/** A point in a terminal's output: the `seq`th item of `epoch`. */
export interface StreamPosition {
  epoch: number;
  seq: number;
}

export const streamPosition: Decoder<StreamPosition> = object({ epoch: u32, seq: u64 });

/** Who may type into a terminal right now. */
export type Controller =
  { kind: "nobody" } | { kind: "local" } | { kind: "device"; device: DeviceId; name: string };

const controller: Decoder<Controller> = tagged<Controller>("kind", {
  nobody: variant("kind", "nobody", {}),
  local: variant("kind", "local", {}),
  device: variant("kind", "device", { device: id<DeviceId>(), name: text(MAX_TEXT) }),
});

/** The reply to `terminal.attach`. */
export interface Attached {
  stream: StreamId;
  /** `snapshot`: clear the screen and draw what follows. `resumed`: it goes on from `position`. */
  start: "snapshot" | "resumed";
  /**
   * What the snapshot or the resumed stream covers, up to and including. A snapshot's frames are
   * numbered `position.seq`; a resumed stream's next item is `position.seq + 1`.
   */
  position: StreamPosition;
  size: TermSize;
  controller: Controller;
}

const attachedFields = {
  stream: u32,
  start: oneOf(["snapshot", "resumed"]),
  position: streamPosition,
  size: termSize,
  controller,
};

/** A control lease. Renew it every `renewEveryMs`, or it lapses after `ttlMs`. */
export interface Lease {
  id: LeaseId;
  /** Increases every time control changes hands on this terminal. */
  generation: number;
  ttlMs: number;
  renewEveryMs: number;
}

const leaseFields = { id: id<LeaseId>(), generation: u64, ttlMs: u32, renewEveryMs: u32 };

/** Something a host tells a client without being asked. */
export type Event =
  | { event: "terminal.resized"; stream: StreamId; position: StreamPosition; size: TermSize }
  | { event: "terminal.detached"; stream: StreamId; error: RemoteError }
  | { event: "terminal.control"; stream: StreamId; controller: Controller }
  | { event: "terminal.exited"; stream: StreamId; exit: { code: number; success: boolean } }
  | ({ event: "activity.events" } & ActivityBatch)
  | ({ event: "activity.ended" } & SubscriptionEnded);

export const EVENTS = [
  "terminal.resized",
  "terminal.detached",
  "terminal.control",
  "terminal.exited",
  "activity.events",
  "activity.ended",
] as const;

export const event: Decoder<Event> = tagged<Event>("event", {
  "terminal.resized": variant("event", "terminal.resized", {
    stream: u32,
    position: streamPosition,
    size: termSize,
  }),
  "terminal.detached": variant("event", "terminal.detached", { stream: u32, error: remoteError }),
  "terminal.control": variant("event", "terminal.control", { stream: u32, controller }),
  "terminal.exited": variant("event", "terminal.exited", {
    stream: u32,
    exit: object({ code: u32, success: bool }),
  }),
  "activity.events": variant("event", "activity.events", {
    subscription: u32,
    events: array(activityEvent),
    newer: cursor,
  }),
  "activity.ended": variant("event", "activity.ended", { subscription: u32, error: remoteError }),
});

// --- requests ----------------------------------------------------------------------------------

const limit = nullable(u32);

export interface WorkspaceListParams {
  project: ProjectId | null;
  includeArchived: boolean;
  cursor: Cursor | null;
  /** At most {@link MAX_PAGE}; 50 when `null`. */
  limit: number | null;
}

export interface SessionListParams {
  workspace: WorkspaceId;
  includeEnded: boolean;
  cursor: Cursor | null;
  limit: number | null;
}

export interface ActivityListParams {
  workspace: WorkspaceId;
  session: SessionRecordId | null;
  cursor: Cursor | null;
  limit: number | null;
}

/** Follow activity as it is recorded: all the grant reaches when nothing is named. */
export interface ActivitySubscribeParams {
  workspace?: WorkspaceId;
  /** Needs `workspace`. */
  session?: SessionRecordId;
  /** A `newer` cursor of the same scope. Without one, only what is recorded from now on. */
  after?: Cursor;
}

export interface MemoryListParams {
  project: ProjectId;
  cursor: Cursor | null;
  limit: number | null;
}

/** A new workspace, named after its first message, with an agent given it. Minor 5. */
export interface WorkspaceStartParams {
  project: ProjectId;
  /** Not empty, at most {@link MAX_PROMPT} bytes. */
  prompt: string;
  /** A harness from `harness.list`. Without one, the computer's default. */
  harness?: string;
}

/** Another agent in a workspace, with a first message or none. Minor 5. */
export interface SessionStartParams {
  workspace: WorkspaceId;
  harness?: string;
  prompt?: string;
}

/** An ended conversation again, under its own record, with a message or none. Minor 6. */
export interface SessionResumeParams {
  session: SessionRecordId;
  prompt?: string;
}

/** An upload under way, numbered by the computer within its channel. */
export type UploadId = number;

export interface LeaseRef {
  terminal: TerminalId;
  lease: LeaseId;
  generation: number;
}

/** What a phone can ask a workflow to do. Minor 7. */
export type WorkflowAction = "approve" | "reject" | "changes" | "answer" | "resume";

const workflowAction: Decoder<WorkflowAction> = (value, path) => {
  const name = text(16)(value, path);
  if (
    name === "approve" ||
    name === "reject" ||
    name === "changes" ||
    name === "answer" ||
    name === "resume"
  ) {
    return name;
  }
  throw new DecodeError(path, "is not a workflow action");
};

export interface WorkflowRunListParams {
  cursor: Cursor | null;
  limit: number | null;
}

export interface WorkflowDecideParams {
  runId: RunId;
  stepId: RunId | null;
  action: WorkflowAction;
  note: string | null;
  code: string | null;
  decisionId: RunId;
  /** The waiting step's attempt, as the run detail showed it. */
  attempt: number;
  /** The open clarification round, or 0. */
  round: number;
}

export interface WorkflowRunSummary {
  runId: RunId;
  name: string;
  status: string;
  workspace: string;
  waitingOn: string | null;
  blocking: string | null;
  createdAt: number;
}

export interface WorkflowRunPage {
  runs: WorkflowRunSummary[];
  next: Cursor | null;
}

export interface WorkflowStepView {
  stepId: string;
  action: string;
  status: string;
  attempts: number;
  note: string | null;
}

export interface WorkflowArtifactView {
  name: string;
  source: string;
  truncated: boolean;
  body?: string;
}

export interface WorkflowRunDetail {
  runId: RunId;
  name: string;
  status: string;
  workspace: string;
  error: string | null;
  waitingOn: string | null;
  blocking: string | null;
  waitingKind?: string;
  attempts: number;
  locked: boolean;
  /** The waiting step's attempt. 0 when none is waiting. */
  attempt: number;
  /** The open clarification round, or 0. */
  round: number;
  resumeEligible: boolean;
  steps: WorkflowStepView[];
  artifacts: WorkflowArtifactView[];
}

export interface WorkflowChallenge {
  code: string;
  attempt: number;
  round: number;
}

export interface WorkflowDecision {
  outcome: string;
  message: string;
}

const workflowRunSummaryFields = {
  runId: id<RunId>(),
  name: text(MAX_TEXT),
  status: text(32),
  workspace: text(MAX_TEXT),
  waitingOn: nullable(text(MAX_TEXT)),
  blocking: nullable(text(MAX_TEXT)),
  createdAt: i64,
};

const workflowRunPageFields = {
  runs: array(object(workflowRunSummaryFields)),
  next: nullable(cursor),
};

const workflowStepFields = {
  stepId: text(MAX_TEXT),
  action: text(MAX_TEXT),
  status: text(32),
  attempts: u32,
  note: nullable(text(MAX_NOTE)),
};

const workflowArtifactFields = {
  name: text(MAX_TEXT),
  source: text(32),
  truncated: bool,
  body: optional(text(MAX_ARTIFACT_PREVIEW)),
};

const workflowRunDetailFields = {
  runId: id<RunId>(),
  name: text(MAX_TEXT),
  status: text(32),
  workspace: text(MAX_TEXT),
  error: nullable(text(MAX_TEXT)),
  waitingOn: nullable(text(MAX_TEXT)),
  blocking: nullable(text(MAX_TEXT)),
  waitingKind: optional(text(32)),
  attempts: u32,
  locked: bool,
  attempt: u32,
  round: u32,
  resumeEligible: bool,
  steps: array(object(workflowStepFields)),
  artifacts: array(object(workflowArtifactFields)),
};

/** Everything a client can ask a host: `op` names it, `params` carries its parameters. */
export type Operation =
  | { op: "host.describe" }
  | { op: "workspace.list"; params: WorkspaceListParams }
  | { op: "workspace.describe"; params: { workspace: WorkspaceId } }
  | { op: "session.list"; params: SessionListParams }
  | { op: "activity.list"; params: ActivityListParams }
  | { op: "memory.list"; params: MemoryListParams }
  | {
      op: "terminal.attach";
      params: { terminal: TerminalId; resumeAfter: StreamPosition | null };
    }
  | { op: "terminal.detach"; params: { terminal: TerminalId; stream: StreamId } }
  | { op: "terminal.acquire"; params: { terminal: TerminalId; takeOver: boolean } }
  | { op: "terminal.renew"; params: LeaseRef }
  | { op: "terminal.release"; params: LeaseRef }
  | {
      op: "terminal.input";
      params: LeaseRef & { seq: number; data: string; paste: boolean };
    }
  | { op: "terminal.resize"; params: LeaseRef & { size: TermSize } }
  | { op: "activity.subscribe"; params: ActivitySubscribeParams }
  | { op: "activity.unsubscribe"; params: { subscription: SubscriptionId } }
  | { op: "harness.list" }
  | { op: "workspace.start"; params: WorkspaceStartParams }
  | { op: "session.start"; params: SessionStartParams }
  | { op: "upload.begin"; params: { workspace: WorkspaceId; name: string; size: number } }
  | { op: "upload.chunk"; params: { upload: UploadId; offset: number; data: string } }
  | { op: "session.resume"; params: SessionResumeParams }
  | { op: "workflow.run.list"; params: WorkflowRunListParams }
  | { op: "workflow.run.get"; params: { runId: RunId } }
  | {
      op: "workflow.challenge";
      params: { runId: RunId; stepId: RunId; attempt: number; round: number };
    }
  | { op: "workflow.decide"; params: WorkflowDecideParams };

export type OperationName = Operation["op"];

/** Every operation's wire name, in the Rust `Operation`'s order. */
export const OPERATIONS: readonly OperationName[] = [
  "host.describe",
  "workspace.list",
  "workspace.describe",
  "session.list",
  "activity.list",
  "memory.list",
  "terminal.attach",
  "terminal.detach",
  "terminal.acquire",
  "terminal.renew",
  "terminal.release",
  "terminal.input",
  "terminal.resize",
  "activity.subscribe",
  "activity.unsubscribe",
  "harness.list",
  "workspace.start",
  "session.start",
  "upload.begin",
  "upload.chunk",
  "session.resume",
  "workflow.run.list",
  "workflow.run.get",
  "workflow.challenge",
  "workflow.decide",
];

const leaseRefFields = { terminal: id<TerminalId>(), lease: id<LeaseId>(), generation: u64 };

const params: Record<string, Decoder<unknown>> = {
  "workspace.list": object({
    project: nullable(id<ProjectId>()),
    includeArchived: orElse(bool, () => false),
    cursor: nullable(cursor),
    limit,
  }),
  "workspace.describe": object({ workspace: id<WorkspaceId>() }),
  "session.list": object({
    workspace: id<WorkspaceId>(),
    includeEnded: orElse(bool, () => false),
    cursor: nullable(cursor),
    limit,
  }),
  "activity.list": object({
    workspace: id<WorkspaceId>(),
    session: nullable(id<SessionRecordId>()),
    cursor: nullable(cursor),
    limit,
  }),
  "memory.list": object({ project: id<ProjectId>(), cursor: nullable(cursor), limit }),
  "terminal.attach": object({
    terminal: id<TerminalId>(),
    resumeAfter: nullable(streamPosition),
  }),
  "terminal.detach": object({ terminal: id<TerminalId>(), stream: u32 }),
  "terminal.acquire": object({ terminal: id<TerminalId>(), takeOver: orElse(bool, () => false) }),
  "terminal.renew": object(leaseRefFields),
  "terminal.release": object(leaseRefFields),
  "terminal.input": object({
    ...leaseRefFields,
    seq: u64,
    data: text(),
    paste: orElse(bool, () => false),
  }),
  "terminal.resize": object({ ...leaseRefFields, size: termSize }),
  "activity.subscribe": object({
    workspace: optional(id<WorkspaceId>()),
    session: optional(id<SessionRecordId>()),
    after: optional(cursor),
  }),
  "activity.unsubscribe": object({ subscription: u32 }),
  "workspace.start": object({
    project: id<ProjectId>(),
    prompt: text(),
    harness: optional(text()),
  }),
  "session.start": object({
    workspace: id<WorkspaceId>(),
    harness: optional(text()),
    prompt: optional(text()),
  }),
  "upload.begin": object({ workspace: id<WorkspaceId>(), name: text(), size: u64 }),
  "upload.chunk": object({ upload: u32, offset: u64, data: text() }),
  "session.resume": object({ session: id<SessionRecordId>(), prompt: optional(text()) }),
  "workflow.run.list": object({ cursor: nullable(cursor), limit }),
  "workflow.run.get": object({ runId: id<RunId>() }),
  "workflow.challenge": object({
    runId: id<RunId>(),
    stepId: id<RunId>(),
    attempt: u32,
    round: u32,
  }),
  "workflow.decide": object({
    runId: id<RunId>(),
    stepId: nullable(id<RunId>()),
    action: workflowAction,
    note: nullable(text(MAX_NOTE)),
    code: nullable(text(MAX_CODE_LEN)),
    decisionId: id<RunId>(),
    attempt: u32,
    round: u32,
  }),
};

/** The scope a caller needs for an operation. `null` for `host.describe`, which any grant may ask. */
export function scopeOf(op: OperationName): Scope | null {
  if (op === "host.describe") return null;
  if (
    op === "workspace.list" ||
    op === "workspace.describe" ||
    op === "session.list" ||
    op === "harness.list"
  ) {
    return "workspace:read";
  }
  if (op.startsWith("activity.")) return "activity:read";
  if (op === "memory.list") return "memory:read";
  if (op === "terminal.attach" || op === "terminal.detach") return "terminal:read";
  if (op === "workflow.run.list" || op === "workflow.run.get") return "workflow:read";
  if (op === "workflow.challenge" || op === "workflow.decide") return "workflow:decide";
  return "terminal:control";
}

export type Request = { requestId: RequestId; protocol: ProtocolVersion } & Operation;

/**
 * A request, as the Rust side reads it: shape, ids and bounds, then the checks a well-formed
 * request must also meet. The client runs what it sends through this, so it never sends what a
 * host would refuse as malformed.
 */
export const request: Decoder<Request> = (value, path) => {
  const head = object({ requestId: id<RequestId>(), protocol: protocolVersion, op: text() })(
    value,
    path,
  );
  const op = head.op as OperationName;
  if (!OPERATIONS.includes(op)) throw new DecodeError("op", "is not an operation this build knows");
  const out: Record<string, unknown> = { requestId: head.requestId, protocol: head.protocol, op };
  if (op !== "host.describe" && op !== "harness.list") {
    out.params = (params[op] as Decoder<unknown>)(
      (value as Record<string, unknown>).params,
      "params",
    );
  }
  const decoded = out as Request;
  const invalid = validate(decoded);
  if (invalid) throw new DecodeError("params", invalid);
  return decoded;
};

/** Why a well-formed request is still refused, or `null`. */
export function validate(operation: Operation): string | null {
  if ("params" in operation && "limit" in operation.params && operation.params.limit === 0) {
    return "asks for a page of no items";
  }
  switch (operation.op) {
    case "terminal.input":
      if (utf8Length(operation.params.data) > MAX_INPUT_BYTES) {
        return "carries more input than one request may";
      }
      if (operation.params.seq === 0) return "numbers input from 0, not 1";
      return null;
    case "activity.subscribe":
      if (operation.params.session !== undefined && operation.params.workspace === undefined) {
        return "follows a conversation without its workspace";
      }
      return null;
    case "terminal.resize":
      if (operation.params.size.cols === 0 || operation.params.size.rows === 0) {
        return "asks for a terminal without a row or a column";
      }
      return null;
    case "workspace.start":
      if (operation.params.prompt.trim() === "") return "starts a workspace with no message";
      return promptTooLong(operation.params.prompt) ?? harnessInvalid(operation.params.harness);
    case "session.start":
      return (
        (operation.params.prompt === undefined ? null : promptTooLong(operation.params.prompt)) ??
        harnessInvalid(operation.params.harness)
      );
    case "upload.begin": {
      const { name, size } = operation.params;
      if (size === 0 || size > MAX_UPLOAD) return "uploads a file of no bytes, or over 20 MB";
      if (name === "" || chars(name) > MAX_UPLOAD_NAME) return "names an upload no file is";
      return null;
    }
    case "upload.chunk":
      return chunkInvalid(operation.params.data);
    case "session.resume":
      return operation.params.prompt === undefined ? null : promptTooLong(operation.params.prompt);
    case "workflow.decide":
      return decideInvalid(operation.params);
    default:
      return null;
  }
}

const CODE = /^[A-Za-z0-9]+$/;

function decideInvalid(params: WorkflowDecideParams): string | null {
  if (params.action === "resume") return null;
  if (params.stepId === null) return "Name the step this decision is for.";
  if (params.code === null || !CODE.test(params.code) || params.code.length > MAX_CODE_LEN) {
    return "A decision needs the one-time code.";
  }
  if (
    (params.action === "changes" || params.action === "answer") &&
    (params.note === null || params.note.trim() === "")
  ) {
    return "Say what should change, or give the answer.";
  }
  return null;
}

function promptTooLong(prompt: string): string | null {
  return utf8Length(prompt) > MAX_PROMPT ? "carries a longer message than one request may" : null;
}

function harnessInvalid(harness: string | undefined): string | null {
  if (harness === undefined) return null;
  return harness === "" || chars(harness) > MAX_LABEL ? "names no harness" : null;
}

const BASE64 = /^[A-Za-z0-9+/]*={0,2}$/;

/** Whether a piece of an upload is padded standard base64 of 1 byte to {@link MAX_UPLOAD_CHUNK}. */
function chunkInvalid(data: string): string | null {
  if (data === "" || data.length > Math.ceil(MAX_UPLOAD_CHUNK / 3) * 4) {
    return "carries a piece of no bytes, or over 32 KB";
  }
  if (data.length % 4 !== 0 || !BASE64.test(data)) return "carries a piece that is not base64";
  return null;
}

// --- replies -----------------------------------------------------------------------------------

/** An agent the computer can start. */
export interface HarnessSummary {
  /** What `workspace.start` and `session.start` name it by. */
  id: string;
  name: string;
}

/** `harness.list`: what the computer can start, its default first. Minor 5. */
export interface HarnessList {
  harnesses: HarnessSummary[];
}

const harnessListFields = {
  harnesses: array(object({ id: text(MAX_LABEL), name: text(MAX_TEXT) })),
};

/** Where an agent a device started runs, to watch it. Minor 5. */
export interface Started {
  workspace: WorkspaceId;
  terminal: TerminalId;
  harness: string;
}

const startedFields = {
  workspace: id<WorkspaceId>(),
  terminal: id<TerminalId>(),
  harness: text(MAX_LABEL),
};

/** What a successful request returns, tagged with `kind`. */
export type Reply =
  | ({ kind: "host" } & HostDescription)
  | ({ kind: "workspaces" } & WorkspacePage)
  | ({ kind: "workspace" } & WorkspaceDetail)
  | ({ kind: "sessions" } & SessionPage)
  | ({ kind: "activity" } & ActivityPage)
  | ({ kind: "memory" } & MemoryPage)
  | ({ kind: "attached" } & Attached)
  | ({ kind: "lease" } & Lease)
  | ({ kind: "subscribed" } & Subscribed)
  | ({ kind: "harnesses" } & HarnessList)
  | ({ kind: "started" } & Started)
  | { kind: "upload"; upload: UploadId }
  | { kind: "uploaded"; upload: UploadId; path: string }
  | ({ kind: "workflowRuns" } & WorkflowRunPage)
  | ({ kind: "workflowRun" } & WorkflowRunDetail)
  | ({ kind: "workflowChallenge" } & WorkflowChallenge)
  | ({ kind: "workflowDecision" } & WorkflowDecision)
  | { kind: "done" };

export const reply: Decoder<Reply> = tagged<Reply>("kind", {
  host: variant("kind", "host", hostFields),
  workspaces: variant("kind", "workspaces", workspacePageFields),
  workspace: variant("kind", "workspace", workspaceDetailFields),
  sessions: variant("kind", "sessions", sessionPageFields),
  activity: variant("kind", "activity", activityPageFields),
  memory: variant("kind", "memory", memoryPageFields),
  attached: variant("kind", "attached", attachedFields),
  lease: variant("kind", "lease", leaseFields),
  subscribed: variant("kind", "subscribed", subscribedFields),
  harnesses: variant("kind", "harnesses", harnessListFields),
  started: variant("kind", "started", startedFields),
  upload: variant("kind", "upload", { upload: u32 }),
  uploaded: variant("kind", "uploaded", { upload: u32, path: text() }),
  workflowRuns: variant("kind", "workflowRuns", workflowRunPageFields),
  workflowRun: variant("kind", "workflowRun", workflowRunDetailFields),
  workflowChallenge: variant("kind", "workflowChallenge", {
    code: text(MAX_CODE_LEN),
    attempt: u32,
    round: u32,
  }),
  workflowDecision: variant("kind", "workflowDecision", {
    outcome: text(64),
    message: text(MAX_TEXT),
  }),
  done: variant("kind", "done", {}),
});

/** A host's answer to one request. `requestId` is `null` only for one too broken to read. */
export type Response = { requestId: RequestId | null } & (
  { result: Reply } | { error: RemoteError }
);

export const response: Decoder<Response> = (value, path) => {
  const { requestId } = object({ requestId: nullable(id<RequestId>()) })(value, path);
  const fields = value as Record<string, unknown>;
  if (fields.result !== undefined) return { requestId, result: reply(fields.result, "result") };
  if (fields.error !== undefined) return { requestId, error: remoteError(fields.error, "error") };
  throw new DecodeError(path, "has neither a result nor an error");
};

// --- grants ------------------------------------------------------------------------------------

/** One permission. A device can watch a terminal without being able to type into it. */
export const SCOPES = [
  "workspace:read",
  "activity:read",
  "memory:read",
  "terminal:read",
  "terminal:control",
  "device:manage",
  "workflow:read",
  "workflow:decide",
] as const;
export type Scope = (typeof SCOPES)[number];

/** Scopes as a set, in the order Rust writes one. A name this build does not know is dropped. */
export const scopes: Decoder<Scope[]> = (value, path) => {
  if (!Array.isArray(value)) throw new DecodeError(path, "is not a list");
  const known = new Set<Scope>();
  for (const item of value) {
    if (typeof item !== "string") throw new DecodeError(path, "is not a string");
    if ((SCOPES as readonly string[]).includes(item)) known.add(item as Scope);
  }
  return SCOPES.filter((scope) => known.has(scope));
};

/** Which projects a grant reaches. */
export type ProjectGrant = { kind: "all" } | { kind: "only"; projects: ProjectId[] };

export const projectGrant: Decoder<ProjectGrant> = tagged<ProjectGrant>("kind", {
  all: variant("kind", "all", {}),
  only: variant("kind", "only", { projects: set(id<ProjectId>(), byOrder) }),
});
