// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The wire, pinned by the same fixtures as the Rust side (`crates/remote-proto/src/tests.rs`).
// Each one decodes, and writes back as exactly itself; each also meets what the Rust test of it
// asserts. A fixture this table does not read fails the run, as one the Rust tests do not read
// fails theirs: a file only one side reads is a wire format only one side is held to.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import {
  CURRENT,
  type Decoder,
  decodeFrame,
  devicePage,
  encodeFrame,
  event,
  FrameError,
  grantDecision,
  grantPage,
  hello,
  helloReply,
  hostNotice,
  hostPage,
  hostRegistration,
  incompatible,
  negotiate,
  normalizeCode,
  offer,
  pairingRequest,
  presented,
  relayTicket,
  remoteError,
  request,
  response,
  retryable,
  scopeOf,
  serverDescription,
  TICKET_TTL_MS,
  ticketRequest,
  tokenRequest,
  validateGrantDecision,
  validateHostRegistration,
  validatePairingRequest,
  validateTokenRequest,
  validRoute,
  credentials,
  greeting,
  decode,
  type Request,
  type Response,
} from "../src/index.ts";

const DIR = join(import.meta.dir, "../../../crates/remote-proto/fixtures/v1");

function raw(name: string): unknown {
  return JSON.parse(readFileSync(join(DIR, name), "utf8"));
}

/** Decoded, and written back as exactly the fixture. */
function pinned<T>(decoder: Decoder<T>, name: string): T {
  const json = raw(name);
  const value = decode(decoder, json);
  expect(JSON.parse(JSON.stringify(value))).toEqual(json);
  return value;
}

function result(name: string) {
  const answer: Response = pinned(response, name);
  if (!("result" in answer)) throw new Error(`${name} is not a result`);
  return answer.result;
}

function hex(text: string): Uint8Array {
  return Uint8Array.from(text.match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));
}

const toHex = (bytes: Uint8Array) =>
  [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");

/** Each fixture, and what reading it has to show. */
const FIXTURES: Record<string, (name: string) => void> = {
  "request-workspace-list.json": (name) => {
    expect(pinned<Request>(request, name).op).toBe("workspace.list");
  },
  "request-host-describe.json": (name) => {
    expect(pinned<Request>(request, name)).toEqual({
      requestId: "r-0",
      protocol: { major: 1, minor: 0 },
      op: "host.describe",
    } as Request);
  },
  "request-activity-list.json": (name) => {
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("activity:read");
  },
  "request-terminal-input.json": (name) => {
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("terminal:control");
  },
  "request-activity-subscribe.json": (name) => {
    // The request decoder also makes the checks a host makes, so reading it is validating it.
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("activity:read");
  },
  "request-workspace-start.json": (name) => {
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("terminal:control");
  },
  "request-workspace-start-full.json": (name) => {
    const start = pinned<Request>(request, name);
    if (start.op !== "workspace.start") throw new Error("not a start");
    expect(scopeOf(start.op)).toBe("terminal:control");
    expect(start.params.name).toBe("fix-login-spacing");
    expect(start.params.model).toBe("opus");
    expect(start.params.effort).toBe("high");
    expect(start.params.baseBranch).toBe("develop");
  },
  "request-workspace-start-branch.json": (name) => {
    const start = pinned<Request>(request, name);
    if (start.op !== "workspace.start") throw new Error("not a start");
    expect(scopeOf(start.op)).toBe("terminal:control");
    expect(start.params.existingBranch).toBe("ys/kept-earlier");
    expect(start.params.baseBranch).toBeUndefined();
  },
  "request-project-branches.json": (name) => {
    const branches = pinned<Request>(request, name);
    expect(branches.op).toBe("project.branches");
    expect(scopeOf(branches.op)).toBe("workspace:read");
  },
  "request-project-clone.json": (name) => {
    const clone = pinned<Request>(request, name);
    expect(clone.op).toBe("project.clone");
    expect(scopeOf(clone.op)).toBe("terminal:control");
  },
  "request-shell-start.json": (name) => {
    const shell = pinned<Request>(request, name);
    expect(shell.op).toBe("shell.start");
    expect(scopeOf(shell.op)).toBe("terminal:control");
  },
  "request-workspace-rename.json": (name) => {
    const rename = pinned<Request>(request, name);
    expect(rename.op).toBe("workspace.rename");
    expect(scopeOf(rename.op)).toBe("terminal:control");
  },
  "request-workspace-archive.json": (name) => {
    const archive = pinned<Request>(request, name);
    if (archive.op !== "workspace.archive") throw new Error("not an archive");
    expect(scopeOf(archive.op)).toBe("terminal:control");
    // Archiving says nothing of force unless it is asked for; deleting here asks.
    expect(archive.params.force).toBeUndefined();
    const unforced = raw(name) as { params: Record<string, unknown> };
    unforced.params.force = false;
    expect(JSON.parse(JSON.stringify(decode(request, unforced)))).toEqual(raw(name));
  },
  "request-workspace-delete.json": (name) => {
    const remove = pinned<Request>(request, name);
    if (remove.op !== "workspace.delete") throw new Error("not a delete");
    expect(scopeOf(remove.op)).toBe("terminal:control");
    expect(remove.params.force).toBe(true);
  },
  "request-session-resume.json": (name) => {
    const resume = pinned<Request>(request, name);
    expect(resume.op).toBe("session.resume");
    expect(scopeOf(resume.op)).toBe("terminal:control");
  },
  "request-upload-chunk.json": (name) => {
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("terminal:control");
  },
  "request-workflow-run-list.json": (name) => {
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("workflow:read");
  },
  "request-workflow-decide.json": (name) => {
    const decide = pinned<Request>(request, name);
    expect(decide.op).toBe("workflow.decide");
    expect(scopeOf(decide.op)).toBe("workflow:decide");
  },
  "request-conversation-read.json": (name) => {
    const read = pinned<Request>(request, name);
    expect(read.op).toBe("conversation.read");
    expect(scopeOf(read.op)).toBe("terminal:read");
  },
  "request-approval-answer.json": (name) => {
    const answer = pinned<Request>(request, name);
    expect(answer.op).toBe("approval.answer");
    expect(scopeOf(answer.op)).toBe("terminal:control");
  },
  "response-conversation.json": (name) => {
    const page = result(name);
    if (page.kind !== "conversation") throw new Error("not a conversation");
    expect(page.turns.map((turn) => turn.n)).toEqual([12, 13]);
    expect(page.asks[0]?.command).toBe("npm test -- --run render");
    expect(page.answersAsks).toBe(true);
  },
  "request-workflow-challenge.json": (name) => {
    expect(scopeOf(pinned<Request>(request, name).op)).toBe("workflow:decide");
  },
  "response-harnesses.json": (name) => {
    const listed = result(name);
    if (listed.kind !== "harnesses") throw new Error("not harnesses");
    expect(listed.harnesses[0]).toEqual({ id: "claude", name: "Claude Code" });
    // A harness before minor 10 lists no models and no efforts, and reads as having none.
    expect(listed.harnesses.every((h) => h.models === undefined && h.efforts === undefined)).toBe(
      true,
    );
  },
  "response-harnesses-choices.json": (name) => {
    const listed = result(name);
    if (listed.kind !== "harnesses") throw new Error("not harnesses");
    expect(listed.harnesses[0]?.models).toEqual(["opus", "sonnet"]);
    expect(listed.harnesses[0]?.efforts).toEqual(["low", "high"]);
    expect(listed.harnesses[1]?.models).toBeUndefined();
    // An empty list is one left out, as Rust writes it.
    const json = raw(name) as { result: { harnesses: Record<string, unknown>[] } };
    (json.result.harnesses[1] as Record<string, unknown>).efforts = [];
    expect(JSON.parse(JSON.stringify(decode(response, json)))).toEqual(raw(name));
  },
  "response-started.json": (name) => {
    const started = result(name);
    expect(started.kind === "started" && started.harness).toBe("claude");
  },
  "response-started-shell.json": (name) => {
    const started = result(name);
    expect(started.kind === "started" && started.harness).toBe("");
  },
  "response-branches.json": (name) => {
    const listed = result(name);
    if (listed.kind !== "branches") throw new Error("not branches");
    expect(listed.branches[0]).toBe("main");
    expect(listed.default).toBe("main");
    expect(listed.checkedOut).toEqual(["main", "ys/fix-login"]);
    expect(listed.more).toBeUndefined();
  },
  "response-cloned.json": (name) => {
    const cloned = result(name);
    if (cloned.kind !== "cloned") throw new Error("not cloned");
    expect(cloned.name).toBe("example-app");
    expect(cloned.project as string).toBe("7a1e3c2d-4b5f-4a6b-8c9d-0e1f2a3b4c5d");
  },
  "response-done.json": (name) => {
    expect(result(name)).toEqual({ kind: "done" });
  },
  "response-uploaded.json": (name) => {
    const uploaded = result(name);
    expect(uploaded.kind === "uploaded" && uploaded.path).toEndWith("-photo.jpg");
  },
  "response-workflow-runs.json": (name) => {
    const page = result(name);
    if (page.kind !== "workflowRuns") throw new Error("not workflow runs");
    expect(page.runs[0]?.waitingOn).toBe("sign-off");
  },
  "response-workflow-run.json": (name) => {
    const run = result(name);
    if (run.kind !== "workflowRun") throw new Error("not a workflow run");
    expect(run.waitingKind).toBe("approval");
    expect(run.artifacts[0]?.body).toBe("Looks good");
  },
  "response-workflow-decision.json": (name) => {
    const decision = result(name);
    if (decision.kind !== "workflowDecision") throw new Error("not a decision");
    expect(decision.outcome).toBe("recorded");
  },
  "response-workflow-challenge.json": (name) => {
    const challenge = result(name);
    if (challenge.kind !== "workflowChallenge") throw new Error("not a challenge");
    expect(challenge.code).toBe("ABCD2345");
  },
  "hello.json": (name) => {
    expect(negotiate(pinned(hello, name))).toEqual(CURRENT);
  },
  "hello-welcome.json": (name) => {
    expect("welcome" in pinned(helloReply, name)).toBe(true);
  },
  "response-host.json": (name) => {
    const host = result(name);
    if (host.kind !== "host") throw new Error("not a host");
    // A host before minor 4 says nothing of it.
    expect(host.terminalsLater).toBeUndefined();
  },
  "response-host-terminals-later.json": (name) => {
    const host = result(name);
    if (host.kind !== "host") throw new Error("not a host");
    expect(host.terminalsLater?.reason).toBe("earlierDaemon");
    expect(host.capabilities.some((c) => c.startsWith("terminal."))).toBe(false);
    // A newer host's reason reads as unknown, not as a failure.
    const json = raw(name) as { result: { terminalsLater: { reason: string } } };
    json.result.terminalsLater.reason = "somethingNew";
    const newer = decode(response, json);
    if (!("result" in newer) || newer.result.kind !== "host") throw new Error("not a host");
    expect(newer.result.terminalsLater?.reason).toBe("unknown");
  },
  "response-workspaces.json": (name) => {
    const page = result(name);
    if (page.kind !== "workspaces") throw new Error("not workspaces");
    expect(page.projects[0]?.remote).toBe("https://github.com/example/example-app.git");
    // A host before minor 10 says nothing of it.
    const json = raw(name) as { result: { projects: Record<string, unknown>[] } };
    for (const project of json.result.projects) delete project.remote;
    const older = decode(response, json);
    if (!("result" in older) || older.result.kind !== "workspaces") {
      throw new Error("not workspaces");
    }
    expect(older.result.projects[0]).toEqual({
      id: "6f0d2b1c-3a4e-4f5a-9b8c-7d6e5f4a3b2c",
      name: "example-app",
      missing: false,
    } as never);
  },
  "response-sessions.json": (name) => {
    expect(result(name).kind).toBe("sessions");
    // What a host from before terminals were listed sends: still 1.x, and still understood.
    const json = raw(name) as { result: Record<string, unknown> };
    delete json.result.terminals;
    const older = decode(response, json);
    if (!("result" in older) || older.result.kind !== "sessions") throw new Error("not sessions");
    expect(older.result.sessions).toHaveLength(2);
    expect(older.result.terminals).toEqual([]);
    // One from before minor 6 says nothing of resuming, which reads as not resumable.
    const listed = result(name);
    if (listed.kind !== "sessions") throw new Error("not sessions");
    expect(listed.sessions.map((s) => s.resumable)).toEqual([false, true]);
    const before = raw(name) as { result: { sessions: Record<string, unknown>[] } };
    for (const session of before.result.sessions) delete session.resumable;
    const earlier = decode(response, before);
    if (!("result" in earlier) || earlier.result.kind !== "sessions") {
      throw new Error("not sessions");
    }
    expect(earlier.result.sessions.map((s) => s.resumable)).toEqual([false, false]);
  },
  "response-activity.json": (name) => {
    expect(result(name).kind).toBe("activity");
  },
  "response-error-incompatible.json": (name) => {
    const answer = pinned(response, name);
    expect("error" in answer && answer.error).toEqual(incompatible());
  },
  "response-error-uncommitted.json": (name) => {
    // Since minor 10: why archiving or deleting was refused, for a client to ask again.
    const answer = pinned(response, name);
    if (!("error" in answer)) throw new Error("not an error");
    expect(answer.error.code).toBe("uncommitted_work");
    expect(answer.error.retryable).toBe(false);
    expect(retryable(answer.error.code)).toBe(false);
  },
  "response-attached.json": (name) => {
    expect(result(name).kind).toBe("attached");
  },
  "response-attached-resumed.json": (name) => {
    const attached = result(name);
    expect(attached.kind === "attached" && attached.start).toBe("resumed");
  },
  "response-subscribed.json": (name) => {
    const subscribed = result(name);
    if (subscribed.kind !== "subscribed") throw new Error("not subscribed");
    expect(subscribed.subscription).toBe(1);
    expect(subscribed.newer).not.toBeNull();
  },
  "account-server.json": (name) => {
    expect(pinned(serverDescription, name).issuer).toBe("https://auth.example.invalid");
  },
  "account-token-sign-in.json": (name) => {
    expect(validateTokenRequest(pinned(tokenRequest, name))).toBeNull();
  },
  "account-token-refresh.json": (name) => {
    expect(validateTokenRequest(pinned(tokenRequest, name))).toBeNull();
  },
  "account-credentials.json": (name) => {
    const issued = pinned(credentials, name);
    // A credential is never printed whole.
    expect(String(issued.refresh)).toBe("Secret(…)");
    expect(Bun.inspect(issued)).not.toContain("ys1.");
  },
  "account-devices.json": (name) => {
    expect(pinned(devicePage, name).devices[1]?.kind).toBe("host");
  },
  "pairing-host-registration.json": (name) => {
    expect(validateHostRegistration(pinned(hostRegistration, name))).toBeNull();
  },
  "pairing-hosts.json": (name) => {
    pinned(hostPage, name);
  },
  "pairing-offer.json": (name) => {
    expect(normalizeCode(pinned(offer, name).code.expose())).not.toBeNull();
  },
  "pairing-request.json": (name) => {
    expect(validatePairingRequest(pinned(pairingRequest, name))).toBeNull();
  },
  "pairing-grants.json": (name) => {
    const page = pinned(grantPage, name);
    expect(page.grants.map((g) => g.state)).toEqual(["approved", "pending"]);
    // A newer server's state reads as unknown, not as a failure to read the page.
    const json = raw(name) as { grants: { state: string }[] };
    (json.grants[0] as { state: string }).state = "suspended";
    expect(decode(grantPage, json).grants[0]?.state).toBe("unknown");
  },
  "pairing-decision.json": (name) => {
    const asked = decode(pairingRequest, raw("pairing-request.json"));
    expect(validateGrantDecision(pinned(grantDecision, name), asked.scopes)).toBeNull();
  },
  "relay-ticket-request.json": (name) => {
    expect(pinned(ticketRequest, name).role).toBe("device");
  },
  "relay-ticket.json": (name) => {
    const ticket = pinned(relayTicket, name);
    expect(ticket.expiresInMs).toBe(TICKET_TTL_MS);
    expect(ticket.route).toBeUndefined();
  },
  "relay-ticket-routed.json": (name) => {
    expect(validRoute(pinned(relayTicket, name).route ?? "")).toBe(true);
  },
  "relay-presented.json": (name) => {
    pinned(presented, name);
  },
  "relay-notice-pairing.json": (name) => {
    expect(pinned(hostNotice, name).kind).toBe("pairingRequested");
    // A newer server's notice reads as one to ignore, not as a broken link.
    expect(decode(hostNotice, { kind: "hostRenamed", name: "x" })).toEqual({ kind: "unknown" });
  },
  "relay-notice-sync.json": (name) => {
    expect(pinned(hostNotice, name)).toEqual({ kind: "syncChanged" });
  },
  "relay-greeting.json": (name) => {
    expect("connected" in pinned(greeting, name)).toBe(true);
  },
  "event-terminal-resized.json": (name) => {
    expect(pinned(event, name).event).toBe("terminal.resized");
  },
  "event-terminal-detached.json": (name) => {
    const detached = pinned(event, name);
    expect(detached.event === "terminal.detached" && detached.error.code).toBe(
      "resnapshot_required",
    );
  },
  "event-terminal-control.json": (name) => {
    const control = pinned(event, name);
    expect(control.event === "terminal.control" && control.controller.kind).toBe("device");
  },
  "event-terminal-exited.json": (name) => {
    const exited = pinned(event, name);
    expect(exited.event === "terminal.exited" && exited.exit).toEqual({ code: 0, success: true });
  },
  "event-activity-events.json": (name) => {
    const batch = pinned(event, name);
    if (batch.event !== "activity.events") throw new Error("not a batch");
    expect(batch.events).toHaveLength(1);
    expect(batch.newer as string).toBe("a1.n4223");
  },
  "event-activity-ended.json": (name) => {
    const ended = pinned(event, name);
    expect(ended.event === "activity.ended" && ended.error).toEqual({
      code: "not_found",
      message: "There is nothing by that id here.",
      retryable: false,
    });
    // An event is told from a response by its `event` field.
    expect(() => decode(event, raw("response-host.json"))).toThrow();
  },
  "frame-output.json": (name) => {
    const { frames } = raw(name) as {
      frames: {
        hex: string;
        stream: number;
        epoch: number;
        seq: number;
        bytes: string;
        reencoded?: string;
      }[];
    };
    expect(frames.length).toBeGreaterThan(0);
    for (const frame of frames) {
      const read = decodeFrame(hex(frame.hex));
      expect(read.stream).toBe(frame.stream);
      expect(read.position).toEqual({ epoch: frame.epoch, seq: frame.seq });
      expect(toHex(read.bytes)).toBe(frame.bytes);
      expect(toHex(encodeFrame(read))).toBe(frame.reencoded ?? frame.hex);
    }
  },
  "frame-refused.json": (name) => {
    const { frames } = raw(name) as { frames: { hex: string; padding?: number; error: string }[] };
    for (const frame of frames) {
      const head = hex(frame.hex);
      const wire = new Uint8Array(head.length + (frame.padding ?? 0));
      wire.set(head);
      expect(() => decodeFrame(wire)).toThrow(new FrameError(frame.error as FrameError["kind"]));
    }
  },
};

describe("the fixtures", () => {
  test("are each read here, and none is left out", () => {
    const names = readdirSync(DIR).sort();
    expect(names.length).toBeGreaterThanOrEqual(72);
    expect(names).toEqual(Object.keys(FIXTURES).sort());
  });

  for (const [name, check] of Object.entries(FIXTURES)) {
    test(name, () => check(name));
  }
});

describe("what a newer peer sends", () => {
  test("an error code this build does not know reads as unknown, and not retryable", () => {
    const error = decode(remoteError, { code: "quota_exceeded", message: "m", retryable: true });
    expect(error.code).toBe("unknown");
    expect(retryable("unknown")).toBe(false);
  });

  test("only transient failures are retryable", () => {
    for (const code of ["host_offline", "rate_limited", "internal"] as const) {
      expect(retryable(code)).toBe(true);
    }
    for (const code of [
      "forbidden",
      "protocol_incompatible",
      "control_conflict",
      "lease_lost",
      "invalid_request",
      "stale",
      "uncommitted_work",
    ] as const) {
      expect(retryable(code)).toBe(false);
    }
  });

  test("a newer minor's fields are dropped, not refused", () => {
    const read = decode(request, {
      requestId: "r",
      protocol: { major: 1, minor: 7 },
      op: "workspace.list",
      params: { includeArchived: true, sortBy: "recent" },
      trace: "x",
    });
    expect(read).toEqual({
      requestId: "r",
      protocol: { major: 1, minor: 7 },
      op: "workspace.list",
      params: { project: null, includeArchived: true, cursor: null, limit: null },
    } as Request);
  });
});
