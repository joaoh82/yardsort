// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The remote protocol: what a device asks a computer through the relay, and what it answers. The
 * types follow `crates/remote-proto` (`lib.rs`, `model.rs`, `terminal.rs`, `grant.rs`, `ids.rs`,
 * `start.rs`, `manage.rs`, `look.rs`, `publish.rs`), which is the source; the fixtures in
 * `crates/remote-proto/fixtures/v1/` hold both to the same wire.
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
  flag,
  hasControl,
  i64,
  isObject,
  nonEmpty,
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
  unlessEmpty,
  utf8Length,
  variant,
} from "./decode.ts";
import {
  MAX_ARTIFACT_PREVIEW,
  MAX_ASK_ID,
  MAX_ASKS,
  MAX_BRANCH,
  MAX_BRANCHES,
  MAX_CHOICES,
  MAX_CLIENT_NAME,
  MAX_COMMAND,
  MAX_CHECK_NAME,
  MAX_CODE_LEN,
  MAX_COMMIT_MESSAGE,
  MAX_CURSOR_LEN,
  MAX_FILE,
  MAX_FILE_CHUNK,
  MAX_ID_LEN,
  MAX_INPUT_BYTES,
  MAX_LABEL,
  MAX_LINK,
  MAX_LISTED_BYTES,
  MAX_NOTE,
  MAX_NOTIFICATION_LINE,
  MAX_NOTIFICATIONS,
  MAX_NOTIFICATIONS_SEEN,
  MAX_OID,
  MAX_PAGE,
  MAX_PATH,
  MAX_PROJECT_NAME,
  MAX_PROMPT,
  MAX_PULL_CHECKS,
  MAX_PULL_WORKSPACES,
  MAX_REVIEWERS,
  MAX_PULL_REQUEST_BODY,
  MAX_PULL_REQUEST_TITLE,
  MAX_REMOTE,
  MAX_SAID,
  MAX_STEP_WORDS,
  MAX_TASKS,
  MAX_TERMINALS,
  MAX_TEST_DEFINITION,
  MAX_TEST_HASH,
  MAX_TEST_INPUT,
  MAX_TEST_NAMES,
  MAX_TEST_STEPS,
  MAX_TEXT,
  MAX_TURN_TEXT,
  MAX_UNPUSHED,
  MAX_UPLOAD,
  MAX_UPLOAD_CHUNK,
  MAX_UPLOAD_NAME,
  MAX_URL,
  MAX_WORKSPACE_NAME,
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
  /**
   * Archiving or deleting the workspace would lose uncommitted work. Ask the person, and send the
   * request again with `force` if they say so. Minor 12.
   */
  "uncommitted_work",
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
  /**
   * Where its repository is fetched from, when that is an address another computer could clone,
   * with no credentials in it. To tell whether two computers' projects are the same repository.
   * At most {@link MAX_REMOTE} bytes. Minor 12.
   */
  remote?: string;
}

const remoteAddress: Decoder<string> = (value, path) => {
  const address = text()(value, path);
  if (utf8Length(address) > MAX_REMOTE || hasControl(address)) {
    throw new DecodeError(path, "is not an address the protocol carries");
  }
  return address;
};

const projectSummary: Decoder<ProjectSummary> = object({
  id: id<ProjectId>(),
  name: text(MAX_TEXT),
  missing: bool,
  remote: optional(remoteAddress),
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

// --- a computer's Notifications list (minor 15) -------------------------------------------------

/**
 * What a row is about, by the computer's own names for its list. One from a newer computer reads
 * as `other`, and a device shows nothing for it.
 */
export const NOTIFICATION_KINDS = [
  "agent.asks",
  "agent.finished",
  "agent.failed",
  "agent.stopped",
  "workflow.approval",
  "workflow.question",
  "workflow.succeeded",
  "workflow.failed",
  "test.waits",
  "schedule.refused",
  "schedule.noWorkspace",
  "trigger.refused",
  "trigger.capped",
  "trigger.noWorkspace",
  "other",
] as const;
export type NotificationKind = (typeof NOTIFICATION_KINDS)[number];

/**
 * Where a row is in the list. `needs`: waiting for an answer, or to be put right. `finished`:
 * finished, and not seen yet. `earlier`: answered or seen, and kept a week.
 */
export type NotificationPart = "needs" | "finished" | "earlier" | "other";

/** One row of a computer's Notifications list. */
export interface NotificationSummary {
  /** The computer's id for the row: what `notification.seen` names. */
  id: string;
  kind: NotificationKind;
  part: NotificationPart;
  project: ProjectId;
  projectName: string;
  /** The workspace it happened in. Only a schedule or a trigger can be without one. */
  workspace?: WorkspaceId;
  /** What that workspace is called, or was. Empty for a schedule that makes its own. */
  workspaceName: string;
  branch?: string;
  /** The agent's conversation, for a row about an agent. */
  session?: SessionRecordId;
  /** What the row is about within its workspace: the conversation's name, or the workflow's,
   *  the test's, the schedule's or the trigger's. */
  title: string;
  /** Which agent: `claude`, `codex`… */
  harness?: string;
  /** The conversation's terminal, while its process runs. */
  terminal?: TerminalId;
  /** The workflow's run, for a row about one: what `workflow.run.get` names. */
  run?: string;
  /** The step that waits, for a workflow's question or approval. */
  step?: string;
  /** When it happened, in epoch milliseconds. */
  at: number;
  /** When it was answered or seen. */
  settledAt?: number;
  /** How long the agent worked before it finished, when that is known. */
  workedMs?: number;
  /** What its process exited with, for one that stopped. */
  exitCode?: number;
  /** The command an agent wants to run, while its question is open. */
  command?: string;
  /** The start of what the agent said last, or of what a workflow asks. */
  said?: string;
  /** Why, for something that went wrong. */
  note?: string;
}

/**
 * `notification.list`: what still waits, the newest first, then the newest of what left. The
 * counts are of everything this device may see, not only of the rows in this reply.
 */
export interface NotificationPage {
  notifications: NotificationSummary[];
  needs: number;
  finished: number;
  /** The list was cut to what one reply carries: what still waits went first. */
  more: boolean;
}

/**
 * `notification.seen`: these finished rows were looked at, or every one this device may see. A
 * row that waits for an answer is not marked: it leaves when it gets one.
 */
export interface NotificationSeenParams {
  ids: string[];
  all: boolean;
}

const notificationLine = optional(text(MAX_NOTIFICATION_LINE));
const exitCode = (value: unknown, path: string) => {
  const code = i64(value, path);
  if (code < -0x8000_0000 || code > 0x7fff_ffff) throw new DecodeError(path, "is not an exit code");
  return code;
};

const notificationSummary = object({
  id: text(MAX_ID_LEN),
  kind: oneOf(NOTIFICATION_KINDS, "other"),
  part: oneOf(["needs", "finished", "earlier", "other"] as const, "other"),
  project: id<ProjectId>(),
  projectName: text(MAX_TEXT),
  workspace: optional(id<WorkspaceId>()),
  workspaceName: text(MAX_TEXT),
  branch: optional(text(MAX_TEXT)),
  session: optional(id<SessionRecordId>()),
  title: text(MAX_TEXT),
  harness: optional(text(MAX_LABEL)),
  terminal: optional(id<TerminalId>()),
  run: optional(text(MAX_ID_LEN)),
  step: optional(text(MAX_ID_LEN)),
  at: i64,
  settledAt: optional(i64),
  workedMs: optional(i64),
  exitCode: optional(exitCode),
  command: notificationLine,
  said: notificationLine,
  note: notificationLine,
});

const notificationPageFields = {
  notifications: array(notificationSummary, MAX_NOTIFICATIONS),
  needs: u32,
  finished: u32,
  more: bool,
};

/** Something a host tells a client without being asked. */
export type Event =
  | { event: "terminal.resized"; stream: StreamId; position: StreamPosition; size: TermSize }
  | { event: "terminal.detached"; stream: StreamId; error: RemoteError }
  | { event: "terminal.control"; stream: StreamId; controller: Controller }
  | { event: "terminal.exited"; stream: StreamId; exit: { code: number; success: boolean } }
  | ({ event: "activity.events" } & ActivityBatch)
  | ({ event: "activity.ended" } & SubscriptionEnded)
  /** The Notifications list this client follows changed: it reads the list again. Minor 15. */
  | { event: "notifications.changed" };

export const EVENTS = [
  "terminal.resized",
  "terminal.detached",
  "terminal.control",
  "terminal.exited",
  "activity.events",
  "activity.ended",
  "notifications.changed",
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
  "notifications.changed": variant("event", "notifications.changed", {}),
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

/**
 * A new workspace, named after its first message, with an agent given it. Minor 5.
 *
 * Since minor 12 it may say what the computer's own composer says: `name`, `model`, `effort`, and
 * either `baseBranch` or `existingBranch`. An earlier computer does not read them, so they are
 * sent only to one whose capabilities list `project.branches`.
 */
export interface WorkspaceStartParams {
  project: ProjectId;
  /** Not empty, at most {@link MAX_PROMPT} bytes. */
  prompt: string;
  /** A harness from `harness.list`. Without one, the computer's default. */
  harness?: string;
  /**
   * The workspace's folder, and its branch after the computer's prefix. At most
   * {@link MAX_WORKSPACE_NAME} characters. Not with `existingBranch`, which names its own.
   */
  name?: string;
  /** The model the agent is told to use. At most {@link MAX_LABEL} characters. */
  model?: string;
  /** One of the harness's `efforts`. */
  effort?: string;
  /** The branch the new one starts from: one of `project.branches`. Without it, the default. */
  baseBranch?: string;
  /**
   * A branch to open as it is, with no new one made: one of `project.branches` that is not
   * checked out. Not with `baseBranch` or `name`.
   */
  existingBranch?: string;
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

// --- conversations (minor 8) ------------------------------------------------------------------

/** `conversation.read`: without `from`, the newest turns that fit; with it, from that turn on. */
export interface ConversationReadParams {
  workspace: WorkspaceId;
  session: SessionRecordId;
  /** A turn's number, from 1. */
  from?: number;
}

/** One turn: the user's message and every text the agent wrote back, its reply last. */
export interface ConversationTurn {
  n: number;
  at: number;
  user: string;
  said: string[];
  /** Some of it was left out; the rest is on the computer. */
  cut: boolean;
}

/** A permission the agent asked for that nobody has answered yet. */
export interface Ask {
  id: string;
  askedAt: number;
  tool: string;
  command?: string;
  path?: string;
}

export interface ConversationPage {
  session: SessionRecordId;
  total: number;
  /** Oldest first. */
  turns: ConversationTurn[];
  unreadable?: string;
  asks: Ask[];
  /** Its questions can be answered with `approval.answer`. */
  answersAsks: boolean;
}

export type ApprovalDecision = "allow" | "deny";

export interface ApprovalAnswerParams {
  workspace: WorkspaceId;
  session: SessionRecordId;
  ask: string;
  decision: ApprovalDecision;
}

const ASK_ID = /^[A-Za-z0-9_-]+$/;

const conversationPageFields = {
  session: id<SessionRecordId>(),
  total: u32,
  turns: array(
    object({
      n: u32,
      at: i64,
      user: text(MAX_TURN_TEXT),
      said: array(text(MAX_TURN_TEXT), MAX_SAID),
      cut: orElse(bool, () => false),
    }),
  ),
  unreadable: optional(text(MAX_TEXT)),
  asks: orElse(
    array(
      object({
        id: text(MAX_ASK_ID),
        askedAt: i64,
        tool: text(MAX_TEXT),
        command: optional(text(MAX_COMMAND)),
        path: optional(text(MAX_TEXT)),
      }),
      MAX_ASKS,
    ),
    () => [],
  ),
  answersAsks: orElse(bool, () => false),
};

// --- smoke tests waiting for a person (minor 10) ------------------------------------------------

/** Who asked for a run. One from a newer host is `other`. */
export type TestAskedBy = "agent" | "app" | "workflow" | "other";

/** One step of a smoke test, in a sentence. */
export interface TestStep {
  id: string;
  /** `setup`, `steps` or `teardown`. */
  phase: string;
  words: string;
}

/**
 * A smoke test run waiting for a person, as the computer's approval card shows it: every step in
 * words, the names of the project's test environment it can read (never a value), and the file
 * before and now when it changed since it was approved.
 */
export interface WaitingTest {
  run: string;
  name: string;
  askedBy: TestAskedBy;
  /** The conversation whose agent asked, when one did. */
  session?: SessionRecordId;
  askedAt: number;
  inputs: { name: string; value: string }[];
  steps: TestStep[];
  env: { name: string; set: boolean }[];
  /** A step runs a program, which is handed the project's whole test environment. */
  runsAProgram: boolean;
  environment: string[];
  /** This exact file was approved before, and the project asks to see every run. */
  approvedBefore: boolean;
  previous?: string;
  definition?: string;
  contentHash: string;
  /** Everything the card says is here, uncut. When false, it can only be refused from here. */
  whole: boolean;
  whyNot?: string;
}

/** The oldest run waiting in a workspace, and how many wait behind it. */
export interface TestWaitingPage {
  first?: WaitingTest;
  behind: number;
}

export type TestDecision = "run" | "refuse";

/** `test.decide`: an answer to the card that showed the file at `contentHash`. */
export interface TestDecideParams {
  workspace: WorkspaceId;
  run: string;
  contentHash: string;
  decision: TestDecision;
}

const TEST_HASH = /^[0-9A-Fa-f]+$/;
const TEST_RUN_ID = /^[A-Za-z0-9_.:-]+$/;

const testWaitingPageFields = {
  first: optional(
    object({
      run: text(MAX_ID_LEN),
      name: text(MAX_TEXT),
      askedBy: oneOf(["agent", "app", "workflow", "other"] as const, "other"),
      session: optional(id<SessionRecordId>()),
      askedAt: i64,
      inputs: array(object({ name: text(MAX_LABEL), value: text(MAX_TEST_INPUT) }), MAX_TEST_NAMES),
      steps: array(
        object({ id: text(MAX_LABEL), phase: text(MAX_LABEL), words: text(MAX_STEP_WORDS) }),
        MAX_TEST_STEPS,
      ),
      env: array(object({ name: text(MAX_LABEL), set: bool }), MAX_TEST_NAMES),
      runsAProgram: bool,
      environment: array(text(MAX_LABEL), MAX_TEST_NAMES),
      approvedBefore: bool,
      previous: optional(text(MAX_TEST_DEFINITION)),
      definition: optional(text(MAX_TEST_DEFINITION)),
      contentHash: text(MAX_TEST_HASH),
      whole: bool,
      whyNot: optional(text(MAX_TEXT)),
    }),
  ),
  behind: u32,
};

// --- pull requests, as a host last read them (minor 11) -----------------------------------------

/**
 * Which pull requests `pull.list` gives. `workspaces`: those a workspace opened or follows,
 * whatever their state, each workspace's own in the host's order for it and before what it
 * follows. The others: every one of that state the host has read, most recently changed first.
 */
export type PullsOf = "workspaces" | "open" | "merged" | "closed";

export interface PullListParams {
  of: PullsOf;
  cursor: Cursor | null;
  limit: number | null;
}

/**
 * Which pull request: a project's own by its number, or one a workspace follows in another
 * repository, by the host's id for that.
 */
export interface PullRef {
  project: ProjectId;
  number: number;
  followed?: string;
}

/** One from a newer host is `other`. */
export type PullState = "open" | "merged" | "closed" | "other";
export type PullReview = "approved" | "changesRequested" | "required" | "none" | "other";

/** A pull request, as much of it as a row and a badge need. */
export interface PullSummary {
  project: ProjectId;
  number: number;
  /** The host's id for it, when a workspace follows it in another repository. */
  followed?: string;
  /** The repository it is in, as `owner/name`, when that is not the project's own. */
  repo?: string;
  title: string;
  /** Its page on the forge: an `https` link, or absent. */
  url?: string;
  branch: string;
  base?: string;
  state: PullState;
  draft: boolean;
  checks: { passed: number; failed: number; running: number };
  review: PullReview;
  /** The forge says it does not merge cleanly into its base. */
  conflicts: boolean;
  additions?: number;
  deletions?: number;
  author?: string;
  /** When it last changed, in epoch milliseconds. */
  updatedAt?: number;
  workspaces: WorkspaceId[];
}

/** `pull.list`. A page without `next` is the last. */
export interface PullPage {
  pulls: PullSummary[];
  next: Cursor | null;
  /** How many open ones the host has read, over every project this device may see. */
  open: number;
  /** The host is reading some of these from the forge now. Asking again shortly gives them. */
  reading: boolean;
}

export type CheckState = "passed" | "failed" | "running" | "other";
export type ReviewerState = "approved" | "changesRequested" | "commented" | "dismissed" | "other";

/** `pull.describe`: one pull request, with its checks and reviewers by name. */
export interface PullDetail {
  pull: PullSummary;
  /** The failed first, then those running. Empty when the host has only their counts. */
  checks: { name: string; state: CheckState }[];
  /** How many checks are not in `checks`. */
  moreChecks: number;
  reviews: { login: string; state: ReviewerState }[];
  /** Who a review was asked of and has not answered. */
  waitingOn: string[];
}

const HTTPS = /^https:\/\//;

const pullSummaryFields = object({
  project: id<ProjectId>(),
  number: u32,
  followed: optional(text(MAX_ID_LEN)),
  repo: optional(text(MAX_TEXT)),
  title: text(MAX_TEXT),
  url: optional(text(MAX_LINK)),
  branch: text(MAX_TEXT),
  base: optional(text(MAX_TEXT)),
  state: oneOf(["open", "merged", "closed", "other"] as const, "other"),
  draft: bool,
  checks: object({ passed: u32, failed: u32, running: u32 }),
  review: oneOf(["approved", "changesRequested", "required", "none", "other"] as const, "other"),
  conflicts: bool,
  additions: optional(u32),
  deletions: optional(u32),
  author: optional(text(MAX_LABEL)),
  updatedAt: optional(i64),
  workspaces: array(id<WorkspaceId>(), MAX_PULL_WORKSPACES),
});

/** A link is kept only when it is to a web page: what a host says is opened in a browser. */
const pullSummary: Decoder<PullSummary> = (value, path) => {
  const { url, ...pull } = pullSummaryFields(value, path);
  return url !== undefined && HTTPS.test(url) ? { ...pull, url } : pull;
};

const pullPageFields = {
  pulls: array(pullSummary, MAX_PAGE),
  next: nullable(cursor),
  open: u32,
  reading: bool,
};

const pullDetailFields = {
  pull: pullSummary,
  checks: array(
    object({
      name: text(MAX_CHECK_NAME),
      state: oneOf(["passed", "failed", "running", "other"] as const, "other"),
    }),
    MAX_PULL_CHECKS,
  ),
  moreChecks: u32,
  reviews: array(
    object({
      login: text(MAX_LABEL),
      state: oneOf(
        ["approved", "changesRequested", "commented", "dismissed", "other"] as const,
        "other",
      ),
    }),
    MAX_REVIEWERS,
  ),
  waitingOn: array(text(MAX_LABEL), MAX_REVIEWERS),
};

// --- looking after projects and workspaces (minor 12) -----------------------------------------

/** `project.branches`: the branches a new workspace in a project can start from or open. */
export interface ProjectBranches {
  /** Local branches, the default first when there is one, then by name. */
  branches: string[];
  /** The branch a new workspace starts from when a start names none. */
  default?: string;
  /** Those of `branches` a worktree has out, which cannot be opened in another. Absent when none. */
  checkedOut?: string[];
  /** The project has more branches than are listed. Absent when it has not. */
  more?: boolean;
}

/**
 * `project.clone`: clone a repository into the computer's projects folder and add it as a project,
 * with the computer's own Git credentials. Needs a grant that reaches every project.
 */
export interface ProjectCloneParams {
  /** The repository's address, with no credentials in it. At most {@link MAX_REMOTE} bytes. */
  remote: string;
  /**
   * What to call the folder and the project. Without one, the last part of the address. At most
   * {@link MAX_PROJECT_NAME} bytes.
   */
  name?: string;
}

/** The project a clone was added as. */
export interface ProjectCloned {
  project: ProjectId;
  name: string;
}

/** `workspace.rename`: its folder and its branch keep their names. Not for `local`. */
export interface WorkspaceRenameParams {
  workspace: WorkspaceId;
  /** From 1 to {@link MAX_WORKSPACE_NAME} characters. */
  name: string;
}

/**
 * `workspace.archive` and `workspace.delete`: take a workspace's folder off the computer's disk.
 * Archiving keeps its record and its conversations; deleting forgets it. Both keep the branch.
 * Not for `local`.
 */
export interface WorkspaceRemoveParams {
  workspace: WorkspaceId;
  /**
   * Go ahead although the folder has uncommitted work, which is then lost. Without it, such a
   * workspace is refused with `uncommitted_work`: send it only after the person was asked again.
   */
  force?: boolean;
}

/** A branch's name in a reply: one a request could give back. */
const branchName: Decoder<string> = (value, path) => {
  const name = text()(value, path);
  if (branchInvalid(name)) throw new DecodeError(path, "is not a branch's name");
  return name;
};

const projectBranchesFields = {
  branches: array(branchName, MAX_BRANCHES),
  default: optional(branchName),
  checkedOut: unlessEmpty(branchName, MAX_BRANCHES),
  more: flag,
};

// --- reading a workspace's changes and files (minor 13) ---------------------------------------

// A file is named by its path inside the workspace, with `/` between its parts, never by a path on
// the computer. All of it needs `terminal:control`: a device that only watches reads none of it.

export const CHANGE_KINDS = [
  "added",
  "modified",
  "deleted",
  "renamed",
  /** New, and not yet known to Git. */
  "untracked",
  "conflicted",
  "unknown",
] as const;
/** How a file changed. A kind from a newer computer reads as `unknown`. */
export type ChangeKind = (typeof CHANGE_KINDS)[number];

/** One changed file. */
export interface ChangedFile {
  /** Inside the workspace. At most {@link MAX_PATH} bytes. */
  path: string;
  /** Where a renamed file was. */
  oldPath?: string;
  kind: ChangeKind;
  /** Lines added and removed. Absent for a binary or an untracked file. */
  additions?: number;
  deletions?: number;
}

/** `workspace.changes`: which files changed in a workspace. */
export interface WorkspaceChanges {
  /** Changed and not committed. */
  uncommitted: ChangedFile[];
  /** Committed on the workspace's branch since `base`. */
  committed: ChangedFile[];
  /** The branch `committed` is measured against. Absent when there is nothing to compare with. */
  base?: string;
  /** There are more changed files than are listed. Absent when there are not. */
  more?: boolean;
}

/** `workspace.files`: one folder of a workspace's file tree. */
export interface WorkspaceFilesParams {
  workspace: WorkspaceId;
  /** The folder, inside the workspace. Absent for its root. */
  dir?: string;
}

/** One entry of a folder. */
export interface FileEntry {
  /** Its name within the folder. */
  name: string;
  /** It is a folder. Absent for a file. */
  dir?: boolean;
}

/** The reply to `workspace.files`: folders first, then files, each by name. */
export interface FileList {
  entries: FileEntry[];
  /** The folder holds more than is listed. Absent when it does not. */
  more?: boolean;
}

export const FILE_SIDES = ["working", "uncommittedOld", "committedOld", "committedNew"] as const;
/**
 * Which contents of a file `file.read` asks for. `working`: as it is on disk now, a file of the
 * tree and the new side of an uncommitted change. `uncommittedOld`: before an uncommitted change,
 * as the last commit has it. `committedOld`: before the workspace's commits, as the base branch
 * has it. `committedNew`: after them, as the last commit has it.
 */
export type FileOf = (typeof FILE_SIDES)[number];

/** `file.read`: a piece of a file in a workspace. */
export interface FileReadParams {
  workspace: WorkspaceId;
  /** Inside the workspace. At most {@link MAX_PATH} bytes. */
  path: string;
  /** For one side of a renamed file's change: where it was. */
  oldPath?: string;
  of: FileOf;
  /** Where the piece starts: 0, then each reply's `offset` plus what it carried. */
  offset: number;
}

export const FILE_STATES = [
  /** Text, carried in `data`. */
  "text",
  /** There is no such file on that side: it was added, or deleted. */
  "absent",
  /** Not text. Nothing is carried. */
  "binary",
  /** More than {@link MAX_FILE} bytes: `size` says how many. Nothing is carried. */
  "tooLarge",
  /** Something the computer does not show: a folder, a link, a device. */
  "notShown",
  "unknown",
] as const;
/** What a file's contents are, as a reply can say. One from a newer computer reads as `unknown`. */
export type FileState = (typeof FILE_STATES)[number];

/** The reply to `file.read`. */
export interface FileChunk {
  state: FileState;
  /**
   * The whole file's size in bytes. It is the same in every piece of one reading: when it differs,
   * the file changed meanwhile, and the client reads it again from the start.
   */
  size: number;
  /** Where this piece starts. */
  offset: number;
  /**
   * Standard base64, padded, of at most {@link MAX_FILE_CHUNK} bytes of the file. Absent unless
   * `state` is `text`, and for a piece of no bytes. A piece may end inside a character: join the
   * pieces before reading them as UTF-8.
   */
  data?: string;
  /** Nothing comes after this piece. */
  done: boolean;
}

/** A path in a reply: one a request could give back. */
const insidePath: Decoder<string> = (value, path) => {
  const inside = text()(value, path);
  if (pathInvalid(inside)) throw new DecodeError(path, "is not a path inside the workspace");
  return inside;
};

/** A list of files held to {@link MAX_LISTED_BYTES} of path, as a computer cuts one. */
function listed<T>(item: Decoder<T>, bytes: (item: T) => number): Decoder<T[]> {
  const list = array(item);
  return (value, path) => {
    const items = list(value, path);
    let total = 0;
    for (const one of items) total += bytes(one);
    if (total > MAX_LISTED_BYTES) throw new DecodeError(path, "lists more than a reply carries");
    return items;
  };
}

const changedFiles = listed(
  object({
    path: insidePath,
    // A computer may say an empty one, which is none.
    oldPath: optional((value, path) => (value === "" ? "" : insidePath(value, path))),
    kind: oneOf(CHANGE_KINDS, "unknown"),
    additions: optional(u32),
    deletions: optional(u32),
  }),
  (file) => utf8Length(file.path) + utf8Length(file.oldPath ?? ""),
);

/** A branch a computer names as it stands, held only to a branch's length. */
const branchAsItIs: Decoder<string> = (value, path) => {
  const branch = text()(value, path);
  if (utf8Length(branch) > MAX_BRANCH) throw new DecodeError(path, "is not a branch's name");
  return branch;
};

const workspaceChangesFields = {
  uncommitted: changedFiles,
  committed: changedFiles,
  base: optional(branchAsItIs),
  more: flag,
};

/** A folder's entry's name: of a length a file's can be, and nothing that redraws a screen. */
const entryName: Decoder<string> = (value, path) => {
  const name = text()(value, path);
  if (name === "" || utf8Length(name) > 255 || hasControl(name)) {
    throw new DecodeError(path, "is not a file's name");
  }
  return name;
};

const fileListFields = {
  entries: listed(object({ name: entryName, dir: flag }), (entry) => utf8Length(entry.name)),
  more: flag,
};

const fileChunkFields = {
  state: oneOf(FILE_STATES, "unknown"),
  size: u64,
  offset: u64,
  data: nonEmpty(text(Math.ceil(MAX_FILE_CHUNK / 3) * 4)),
  done: bool,
};

// --- sending a workspace's work on (minor 14) -------------------------------------------------

// What the computer's own commit box and publish bar do, asked of it by the workspace's id. It
// commits with its own Git identity, pushes with its own credentials and asks the forge with its
// own sign-in: no request carries a credential, a remote, a branch to push to, or a path. Each of
// these but `workspace.publish` changes a repository, so one is sent only at its person's press.
// All of it needs `terminal:control`.

/** `workspace.publish`: where a workspace stands with its remote. */
export interface WorkspacePublishParams {
  workspace: WorkspaceId;
  /**
   * Ask the forge again about its pull request, which takes the computer a network round trip.
   * Without it, only what Git knows is answered, and `pullRequest` is absent.
   */
  forge?: boolean;
}

export const CHECKS = ["none", "running", "passing", "failing", "unknown"] as const;
/**
 * What the forge's checks say about a pull request's head commit. One from a newer computer reads
 * as `unknown`.
 */
export type Checks = (typeof CHECKS)[number];

export const PULL_REQUEST_STATES = ["open", "merged", "closed", "unknown"] as const;
/** Whether a pull request is open still. A state from a newer computer reads as `unknown`. */
export type PullRequestState = (typeof PULL_REQUEST_STATES)[number];

/** The pull request of a workspace's branch. */
export interface PullRequest {
  number: number;
  /** Its page on the forge. At most {@link MAX_URL} bytes. */
  url: string;
  /** At most {@link MAX_PULL_REQUEST_TITLE} characters. */
  title: string;
  state: PullRequestState;
  /** Absent when it is not a draft. */
  draft?: boolean;
  checks: Checks;
  /** The commit at its head, to name when merging it. Absent when the forge did not say. */
  headOid?: string;
  /** The forge says it does not merge cleanly into its base. Absent when it does not say so. */
  conflicting?: boolean;
}

/** The reply to `workspace.publish`, `workspace.commit` and `workspace.push`. */
export interface PublishState {
  /** Absent when the workspace is not on a branch: there is nothing to push or open. */
  branch?: string;
  /** What a pull request would merge into. */
  base?: string;
  /** Who commits there would be by. Absent when Git has no name and email on the computer. */
  identity?: string;
  /** The project has a remote to push to. Absent when it has none. */
  remote?: boolean;
  /** The branch has been pushed, and tracks its remote branch. Absent when it has not. */
  pushed?: boolean;
  /** Commits the remote does not have. */
  ahead: number;
  behind: number;
  /** The subjects of those commits, newest first: at most {@link MAX_UNPUSHED}. Absent when none. */
  unpushed?: string[];
  /** Absent when there is none, and when the forge was not asked. */
  pullRequest?: PullRequest;
  /** Opening one makes sense: a branch that is not its own base, on a forge, with none open. */
  canOpen?: boolean;
  /** The computer has the forge's own program and can open and merge without a browser. */
  gh?: boolean;
  /** Why the forge gave no answer, when it was asked. */
  problem?: string;
}

/** `workspace.commit`: commit everything changed in a workspace. */
export interface WorkspaceCommitParams {
  workspace: WorkspaceId;
  /** Not empty, at most {@link MAX_COMMIT_MESSAGE} bytes. */
  message: string;
}

/**
 * `pull_request.open`: open a pull request for a workspace's branch, pushing it first when the
 * remote does not have it all.
 */
export interface PullRequestOpenParams {
  workspace: WorkspaceId;
  /** Not empty, at most {@link MAX_PULL_REQUEST_TITLE} characters, on one line. */
  title: string;
  /** At most {@link MAX_PULL_REQUEST_BODY} bytes. */
  body?: string;
  draft?: boolean;
}

/** The reply to `pull_request.open`. */
export interface PullRequestOpened {
  /** The pull request, or the forge's own form to finish in a browser. At most {@link MAX_URL}. */
  url: string;
  /** The computer opened it. When false, `url` is the form, with both branches filled in. */
  created: boolean;
}

export const MERGE_METHODS = ["squash", "merge", "rebase"] as const;
export type MergeMethod = (typeof MERGE_METHODS)[number];

/**
 * `pull_request.merge`: merge the pull request of a workspace's branch, if it is still open, not a
 * draft, and at the commit named.
 */
export interface PullRequestMergeParams {
  workspace: WorkspaceId;
  number: number;
  /** The commit the person saw at its head. At most {@link MAX_OID} characters, all hexadecimal. */
  headOid: string;
  method: MergeMethod;
}

/** A link to the forge in a reply: within its length, and nothing that redraws a screen. */
const forgeLink: Decoder<string> = (value, path) => {
  const url = text()(value, path);
  if (utf8Length(url) > MAX_URL || hasControl(url)) {
    throw new DecodeError(path, "is not a link the protocol carries");
  }
  return url;
};

/** A commit's id as a request may give one: hexadecimal, of 7 to {@link MAX_OID} characters. */
const OID = new RegExp(`^[0-9A-Fa-f]{7,${MAX_OID}}$`);

/** A commit's id in a reply: one a request could give back. */
const commitId: Decoder<string> = (value, path) => {
  const oid = text()(value, path);
  if (!OID.test(oid)) throw new DecodeError(path, "is not a commit's id");
  return oid;
};

const pullRequest = object({
  number: u32,
  url: forgeLink,
  title: text(MAX_PULL_REQUEST_TITLE),
  state: oneOf(PULL_REQUEST_STATES, "unknown"),
  draft: flag,
  checks: oneOf(CHECKS, "unknown"),
  headOid: optional(commitId),
  conflicting: flag,
});

const publishStateFields = {
  branch: optional(branchAsItIs),
  base: optional(branchAsItIs),
  identity: optional(text(MAX_TEXT)),
  remote: flag,
  pushed: flag,
  ahead: u32,
  behind: u32,
  unpushed: unlessEmpty(text(MAX_TEXT), MAX_UNPUSHED),
  pullRequest: optional(pullRequest),
  canOpen: flag,
  gh: flag,
  problem: optional(text(MAX_TEXT)),
};

const pullRequestOpenedFields = { url: forgeLink, created: bool };

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
  | { op: "workflow.decide"; params: WorkflowDecideParams }
  | { op: "conversation.read"; params: ConversationReadParams }
  | { op: "approval.answer"; params: ApprovalAnswerParams }
  | { op: "test.waiting"; params: { workspace: WorkspaceId } }
  | { op: "test.decide"; params: TestDecideParams }
  | { op: "pull.list"; params: PullListParams }
  | { op: "pull.describe"; params: PullRef }
  | { op: "project.branches"; params: { project: ProjectId } }
  | { op: "project.clone"; params: ProjectCloneParams }
  | { op: "shell.start"; params: { workspace: WorkspaceId } }
  | { op: "workspace.rename"; params: WorkspaceRenameParams }
  | { op: "workspace.archive"; params: WorkspaceRemoveParams }
  | { op: "workspace.delete"; params: WorkspaceRemoveParams }
  | { op: "workspace.changes"; params: { workspace: WorkspaceId } }
  | { op: "workspace.files"; params: WorkspaceFilesParams }
  | { op: "file.read"; params: FileReadParams }
  | { op: "workspace.publish"; params: WorkspacePublishParams }
  | { op: "workspace.commit"; params: WorkspaceCommitParams }
  | { op: "workspace.push"; params: { workspace: WorkspaceId } }
  | { op: "pull_request.open"; params: PullRequestOpenParams }
  | { op: "pull_request.merge"; params: PullRequestMergeParams }
  | { op: "notification.list"; params: Record<string, never> }
  | { op: "notification.seen"; params: NotificationSeenParams }
  | { op: "notification.follow" }
  | { op: "notification.unfollow" };

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
  "conversation.read",
  "approval.answer",
  "test.waiting",
  "test.decide",
  "pull.list",
  "pull.describe",
  "project.branches",
  "project.clone",
  "shell.start",
  "workspace.rename",
  "workspace.archive",
  "workspace.delete",
  "workspace.changes",
  "workspace.files",
  "file.read",
  "workspace.publish",
  "workspace.commit",
  "workspace.push",
  "pull_request.open",
  "pull_request.merge",
  "notification.list",
  "notification.seen",
  "notification.follow",
  "notification.unfollow",
];

const leaseRefFields = { terminal: id<TerminalId>(), lease: id<LeaseId>(), generation: u64 };
const workspaceRemoveFields = { workspace: id<WorkspaceId>(), force: flag };

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
    name: optional(text()),
    model: optional(text()),
    effort: optional(text()),
    baseBranch: optional(text()),
    existingBranch: optional(text()),
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
  "conversation.read": object({
    workspace: id<WorkspaceId>(),
    session: id<SessionRecordId>(),
    from: optional(u32),
  }),
  "approval.answer": object({
    workspace: id<WorkspaceId>(),
    session: id<SessionRecordId>(),
    ask: text(MAX_ASK_ID),
    decision: oneOf(["allow", "deny"] as const),
  }),
  "test.waiting": object({ workspace: id<WorkspaceId>() }),
  "test.decide": object({
    workspace: id<WorkspaceId>(),
    run: text(MAX_ID_LEN),
    contentHash: text(MAX_TEST_HASH),
    decision: oneOf(["run", "refuse"] as const),
  }),
  "pull.list": object({
    of: orElse(oneOf(["workspaces", "open", "merged", "closed"] as const), () => "workspaces"),
    cursor: nullable(cursor),
    limit,
  }),
  "pull.describe": object({
    project: id<ProjectId>(),
    number: u32,
    followed: optional(text(MAX_ID_LEN)),
  }),
  "project.branches": object({ project: id<ProjectId>() }),
  "project.clone": object({ remote: text(), name: optional(text()) }),
  "shell.start": object({ workspace: id<WorkspaceId>() }),
  "workspace.rename": object({ workspace: id<WorkspaceId>(), name: text() }),
  "workspace.archive": object(workspaceRemoveFields),
  "workspace.delete": object(workspaceRemoveFields),
  "workspace.changes": object({ workspace: id<WorkspaceId>() }),
  "workspace.files": object({ workspace: id<WorkspaceId>(), dir: nonEmpty(text()) }),
  "file.read": object({
    workspace: id<WorkspaceId>(),
    path: text(),
    oldPath: optional(text()),
    of: oneOf(FILE_SIDES),
    offset: orElse(u64, () => 0),
  }),
  "workspace.publish": object({ workspace: id<WorkspaceId>(), forge: flag }),
  "workspace.commit": object({ workspace: id<WorkspaceId>(), message: text() }),
  "workspace.push": object({ workspace: id<WorkspaceId>() }),
  "pull_request.open": object({
    workspace: id<WorkspaceId>(),
    title: text(),
    body: nonEmpty(text()),
    draft: flag,
  }),
  "pull_request.merge": object({
    workspace: id<WorkspaceId>(),
    number: u32,
    headOid: text(),
    method: oneOf(MERGE_METHODS),
  }),
  "notification.list": object({}),
  "notification.seen": object({
    ids: orElse(array(text()), () => []),
    all: orElse(bool, () => false),
  }),
};

/** The operations that carry no `params` at all. */
const BARE: readonly OperationName[] = [
  "host.describe",
  "harness.list",
  "notification.follow",
  "notification.unfollow",
];

/** The scope a caller needs for an operation. `null` for `host.describe`, which any grant may ask. */
export function scopeOf(op: OperationName): Scope | null {
  if (op === "host.describe") return null;
  if (
    op === "workspace.list" ||
    op === "workspace.describe" ||
    op === "session.list" ||
    op === "harness.list" ||
    op === "project.branches" ||
    op === "pull.list" ||
    op === "pull.describe"
  ) {
    return "workspace:read";
  }
  if (op.startsWith("activity.")) return "activity:read";
  if (op === "memory.list") return "memory:read";
  if (
    op === "terminal.attach" ||
    op === "terminal.detach" ||
    op === "conversation.read" ||
    op === "test.waiting" ||
    op.startsWith("notification.")
  ) {
    return "terminal:read";
  }
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
  if (!BARE.includes(op)) {
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
    case "workspace.start": {
      const start = operation.params;
      if (start.prompt.trim() === "") return "starts a workspace with no message";
      const invalid =
        promptTooLong(start.prompt) ??
        harnessInvalid(start.harness) ??
        labelInvalid(start.model, "names no model") ??
        labelInvalid(start.effort, "names no effort") ??
        (start.name === undefined ? null : workspaceNameInvalid(start.name)) ??
        (start.baseBranch === undefined ? null : branchInvalid(start.baseBranch)) ??
        (start.existingBranch === undefined ? null : branchInvalid(start.existingBranch));
      if (invalid) return invalid;
      if (
        start.existingBranch !== undefined &&
        (start.baseBranch !== undefined || start.name !== undefined)
      ) {
        return "opens an existing branch as it is, which takes no name and no base";
      }
      return null;
    }
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
    case "conversation.read":
      return operation.params.from === 0 ? "numbers turns from 0, not 1" : null;
    case "approval.answer":
      return ASK_ID.test(operation.params.ask) && operation.params.ask.length <= MAX_ASK_ID
        ? null
        : "names a question Yardsort did not ask";
    case "test.decide":
      if (!TEST_RUN_ID.test(operation.params.run)) return "names a run Yardsort did not queue";
      return TEST_HASH.test(operation.params.contentHash) ? null : "names no test file's hash";
    case "pull.describe":
      if (operation.params.number === 0) return "numbers pull requests from 0, not 1";
      return operation.params.followed === undefined || isId(operation.params.followed)
        ? null
        : "names a followed pull request by no id of Yardsort's";
    case "project.clone":
      return cloneInvalid(operation.params);
    case "workspace.rename":
      return workspaceNameInvalid(operation.params.name);
    case "workspace.files": {
      const { dir } = operation.params;
      // No folder, or an empty one, is the workspace's root.
      return dir === undefined || dir === "" ? null : pathInvalid(dir);
    }
    case "file.read": {
      const { path, oldPath, offset } = operation.params;
      const invalid = pathInvalid(path) ?? (oldPath === undefined ? null : pathInvalid(oldPath));
      if (invalid) return invalid;
      return offset > MAX_FILE ? "starts past the end of any file a computer sends" : null;
    }
    case "workspace.commit": {
      const { message } = operation.params;
      if (BLANK.test(message)) return "commits with no message";
      return utf8Length(message) > MAX_COMMIT_MESSAGE || message.includes("\0")
        ? "carries a commit message no request may"
        : null;
    }
    case "pull_request.open":
      return openInvalid(operation.params);
    case "pull_request.merge":
      return operation.params.number === 0 || !OID.test(operation.params.headOid)
        ? "names no pull request and the commit at its head"
        : null;
    case "notification.seen": {
      const { ids, all } = operation.params;
      if (ids.length > MAX_NOTIFICATIONS_SEEN) return "names more rows than one request may";
      if (!ids.every(isId)) return "names a row by no id of Yardsort's";
      return ids.length === 0 && !all ? "names no rows and does not say all of them" : null;
    }
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

/**
 * White space as Rust's `char::is_whitespace` has it, which `str::trim` takes off: not quite
 * JavaScript's, which leaves U+0085 and takes U+FEFF.
 */
const SPACE = "\\t-\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000";
const WHITESPACE = new RegExp(`[${SPACE}]`);
const BLANK = new RegExp(`^[${SPACE}]*$`);
const ENDS = new RegExp(`^[${SPACE}]+|[${SPACE}]+$`, "g");

/** A branch's name as a request may give one: something git could have, and no option. */
function branchInvalid(branch: string): string | null {
  return branch === "" ||
    utf8Length(branch) > MAX_BRANCH ||
    branch.startsWith("-") ||
    branch.includes(" ") ||
    hasControl(branch)
    ? "names no branch"
    : null;
}

/** A workspace's name as a request may give one. */
function workspaceNameInvalid(name: string): string | null {
  return BLANK.test(name) || chars(name) > MAX_WORKSPACE_NAME || hasControl(name)
    ? "gives a workspace a name of no characters, or over 80"
    : null;
}

/** A model or an effort: a label, and nothing that could be read as an option. */
function labelInvalid(label: string | undefined, why: string): string | null {
  if (label === undefined) return null;
  return BLANK.test(label) || chars(label) > MAX_LABEL || label.startsWith("-") || hasControl(label)
    ? why
    : null;
}

/** Shape only. Whether it is an address the computer will clone from is the computer's to say. */
function cloneInvalid(params: ProjectCloneParams): string | null {
  const remote = params.remote.replace(ENDS, "");
  if (
    remote === "" ||
    utf8Length(remote) > MAX_REMOTE ||
    remote.startsWith("-") ||
    hasControl(remote) ||
    WHITESPACE.test(remote)
  ) {
    return "names no repository's address";
  }
  const { name } = params;
  if (
    name !== undefined &&
    (BLANK.test(name) ||
      utf8Length(name) > MAX_PROJECT_NAME ||
      hasControl(name) ||
      name.includes("/") ||
      name.includes("\\"))
  ) {
    return "gives a project a name none can have";
  }
  return null;
}

/**
 * A path as a request may give one: relative, within bounds, and never one that climbs out.
 * Whether it stays inside the workspace on disk, links included, is the computer's to check.
 */
function pathInvalid(path: string): string | null {
  return path === "" ||
    utf8Length(path) > MAX_PATH ||
    /^[/\\-]/.test(path) ||
    hasControl(path) ||
    // A drive's letter, on a computer that has drives: its second byte, as Rust reads it.
    (path.charCodeAt(0) < 0x80 && path.charAt(1) === ":") ||
    path.split(/[/\\]/).some((part) => part === ".." || part === "")
    ? "names no path inside the workspace"
    : null;
}

/** A title on one line that is no option, as the host reads it with what is around it trimmed. */
function openInvalid(params: PullRequestOpenParams): string | null {
  const title = params.title.replace(ENDS, "");
  if (
    title === "" ||
    chars(title) > MAX_PULL_REQUEST_TITLE ||
    title.startsWith("-") ||
    hasControl(title)
  ) {
    return "gives a pull request a title that is not 1 to 256 characters on one line";
  }
  const body = params.body ?? "";
  return utf8Length(body) > MAX_PULL_REQUEST_BODY || body.includes("\0")
    ? "carries a longer description than one request may"
    : null;
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
  /**
   * Models to suggest for it, as the computer's settings list them: any other may be named too.
   * Absent when it lists none, as from a computer before minor 12.
   */
  models?: string[];
  /** The efforts it takes. Absent when it has no such thing. Minor 12. */
  efforts?: string[];
}

/** `harness.list`: what the computer can start, its default first. Minor 5. */
export interface HarnessList {
  harnesses: HarnessSummary[];
}

const harnessListFields = {
  harnesses: array(
    object({
      id: text(MAX_LABEL),
      name: text(MAX_TEXT),
      models: unlessEmpty(text(MAX_LABEL), MAX_CHOICES),
      efforts: unlessEmpty(text(MAX_LABEL), MAX_CHOICES),
    }),
  ),
};

/** Where an agent or a shell a device started runs, to watch it. Minor 5. */
export interface Started {
  workspace: WorkspaceId;
  terminal: TerminalId;
  /** The harness started, by its id. Empty for a shell. */
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
  | ({ kind: "conversation" } & ConversationPage)
  | ({ kind: "testsWaiting" } & TestWaitingPage)
  | ({ kind: "pulls" } & PullPage)
  | ({ kind: "pull" } & PullDetail)
  | ({ kind: "branches" } & ProjectBranches)
  | ({ kind: "cloned" } & ProjectCloned)
  | ({ kind: "changes" } & WorkspaceChanges)
  | ({ kind: "files" } & FileList)
  | ({ kind: "file" } & FileChunk)
  | ({ kind: "publish" } & PublishState)
  | ({ kind: "pullRequestOpened" } & PullRequestOpened)
  | ({ kind: "notifications" } & NotificationPage)
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
  conversation: variant("kind", "conversation", conversationPageFields),
  testsWaiting: variant("kind", "testsWaiting", testWaitingPageFields),
  pulls: variant("kind", "pulls", pullPageFields),
  pull: variant("kind", "pull", pullDetailFields),
  branches: variant("kind", "branches", projectBranchesFields),
  cloned: variant("kind", "cloned", { project: id<ProjectId>(), name: text(MAX_TEXT) }),
  changes: variant("kind", "changes", workspaceChangesFields),
  files: variant("kind", "files", fileListFields),
  file: variant("kind", "file", fileChunkFields),
  publish: variant("kind", "publish", publishStateFields),
  pullRequestOpened: variant("kind", "pullRequestOpened", pullRequestOpenedFields),
  notifications: variant("kind", "notifications", notificationPageFields),
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
