// Yardsort's extension for the pi family — pi, and OMP, its fork: what the agent does, reported
// to the activity timeline.
//
// The agent loads this for one launch (`-e <file>`), beside whatever extensions the user has,
// when capture is on for it. It subscribes to a few of the agent's extension events, keeps the
// metadata of each — a tool's name, a file's path, ids, token counts — and hands it to the
// Yardsort executable in hook mode on stdin, which writes one file to the activity inbox. No
// prompt, command, file content or reply leaves this process: `reduce` copies only the fields the
// Yardsort side records.
//
// Written from a template in the Yardsort binary before each launch, so the executable it names
// is the one that launched. Only notification events are used — never `tool_call`, whose
// handler failing would block the tool — and every handler swallows its own failures.

import { spawn } from "node:child_process";

const EXE = "__YARDSORT_EXE__";
const INBOX = "__YARDSORT_INBOX__";
const HARNESS = "__YARDSORT_HARNESS__";

type Session = { id?: string; cwd?: string; model?: { id?: string; provider?: string } };

/**
 * The part of an event Yardsort records, or null when the event is not one of ours. Pure, and
 * the shape is the raw event's with fields left out, so the Yardsort side reads a recorded
 * fixture and a live delivery the same way.
 */
export function reduce(event: string, payload: any, session: Session) {
  const reduced = reduceEvent(event, payload ?? {});
  if (!reduced) return null;
  return { event, payload: reduced, session };
}

function reduceEvent(event: string, p: any): Record<string, unknown> | null {
  switch (event) {
    case "session_start":
      return pick(p, ["type", "reason"]);
    case "session_shutdown":
      return pick(p, ["type", "reason"]);
    case "input":
      return {
        type: p.type,
        source: p.source,
        chars: typeof p.text === "string" ? p.text.length : undefined,
      };
    case "turn_start":
      return pick(p, ["type", "turnIndex", "timestamp"]);
    case "turn_end": {
      const m = p.message ?? {};
      return {
        type: p.type,
        turnIndex: p.turnIndex,
        message: {
          ...pick(m, ["role", "model", "provider", "stopReason", "timestamp", "duration", "ttft"]),
          usage: pick(m.usage, [
            "input",
            "output",
            "cacheRead",
            "cacheWrite",
            "reasoningTokens",
            "reasoning",
            "totalTokens",
          ]),
          cost: pick(m.usage?.cost, ["total"]),
        },
      };
    }
    case "tool_execution_start":
      return {
        ...pick(p, ["type", "toolCallId", "toolName"]),
        args: pick(p.args, ["path", "cwd"]),
      };
    case "tool_execution_end": {
      const details = p.result?.details ?? {};
      return {
        ...pick(p, ["type", "toolCallId", "toolName", "isError"]),
        result: {
          details: {
            ...pick(details, ["resolvedPath", "wallTimeMs"]),
            meta: details.meta?.source
              ? { source: pick(details.meta.source, ["type", "value"]) }
              : undefined,
          },
        },
      };
    }
    case "tool_approval_requested":
      return pick(p, ["type", "toolCallId", "toolName", "approvalMode"]);
    case "tool_approval_resolved":
      return pick(p, ["type", "toolCallId", "toolName", "approved"]);
    case "model_select":
      return {
        type: p.type,
        model: pick(p.model, ["id", "provider"]),
        previousModel: pick(p.previousModel, ["id", "provider"]),
        source: p.source,
      };
    default:
      return null;
  }
}

function pick(object: any, keys: string[]): Record<string, unknown> {
  if (!object || typeof object !== "object") return {};
  const out: Record<string, unknown> = {};
  for (const key of keys) {
    if (object[key] !== undefined) out[key] = object[key];
  }
  return out;
}

/** Hand one reduced event to the Yardsort executable. Fire and forget; never waits. */
function deliver(entry: unknown) {
  const child = spawn(EXE, ["--yardsort-hook", HARNESS, INBOX], {
    stdio: ["pipe", "ignore", "ignore"],
    windowsHide: true,
  });
  child.on("error", () => {});
  child.stdin.on("error", () => {});
  child.stdin.end(JSON.stringify(entry));
}

const EVENTS = [
  "session_start",
  "session_shutdown",
  "input",
  "turn_start",
  "turn_end",
  "tool_execution_start",
  "tool_execution_end",
  "tool_approval_requested",
  "tool_approval_resolved",
  "model_select",
];

export default function (pi: any) {
  for (const name of EVENTS) {
    try {
      pi.on(name, (event: any, ctx: any) => {
        try {
          const session: Session = {
            id: ctx?.sessionManager?.getSessionId?.(),
            cwd: ctx?.cwd,
            model: ctx?.model ? { id: ctx.model.id, provider: ctx.model.provider } : undefined,
          };
          const entry = reduce(name, event, session);
          if (entry) deliver(entry);
        } catch {
          // Reporting is never allowed to get in the agent's way.
        }
      });
    } catch {
      // An event this agent does not have: pi lacks OMP's approvals, OMP lacks pi's model_select.
    }
  }
}
