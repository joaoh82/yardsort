// Yardsort's OpenCode plugin: what the agent does, reported to the activity timeline.
//
// OpenCode loads this for one launch, through `OPENCODE_CONFIG_CONTENT`, when "Capture what
// OpenCode reports" is on. It listens to a few of OpenCode's plugin hooks and bus events, keeps
// the metadata of each — a tool's name, a file's path, an exit code, token counts, ids — and
// hands it to the Yardsort executable in hook mode, which writes one file to the activity inbox.
// Nothing here reads a prompt, a command, a file's contents or a reply: `reduce` copies only the
// fields the Yardsort side records, and everything else stops at this process's memory.
//
// Written from a template in the Yardsort binary before each launch, so the executable it names
// is the one that launched. Never throws into OpenCode: every hook swallows its own failures.

import { spawn } from "node:child_process";

const EXE = "__YARDSORT_EXE__";
const INBOX = "__YARDSORT_INBOX__";
const HARNESS = "opencode";

/** Bus event types worth a file. Everything else — streaming deltas, catalogues — is not. */
const EVENTS = new Set([
  "session.created",
  "session.idle",
  "session.error",
  "file.edited",
  "permission.asked",
  "permission.replied",
  "message.updated",
  "message.part.updated",
]);

/**
 * The part of a hook's payload that Yardsort records, or null when the call is not one of
 * ours. Pure, and the shape is the raw payload's with fields left out, so the Yardsort side
 * reads a recorded fixture and a live delivery the same way. Not an export of its own:
 * OpenCode calls every export as a plugin, and a function that answers null to that is a
 * crash on start-up. It hangs off the plugin instead, for the tests.
 */
function reduce(hook, payload, directory) {
  const reduced = reduceHook(hook, payload ?? {});
  if (!reduced) return null;
  if (typeof directory === "string") reduced.directory = directory;
  return reduced;
}

function reduceHook(hook, p) {
  switch (hook) {
    case "event": {
      if (!EVENTS.has(p.type)) return null;
      const props = p.properties ?? {};
      switch (p.type) {
        case "session.created":
          return {
            hook,
            payload: {
              type: p.type,
              properties: {
                sessionID: props.sessionID,
                info: pick(props.info, ["id", "version", "parentID", "time"]),
              },
            },
          };
        case "message.updated": {
          const info = props.info ?? {};
          // Only an assistant message that finished carries the turn's usage.
          if (info.role !== "assistant" || !info.time?.completed) return null;
          return {
            hook,
            payload: {
              type: p.type,
              properties: {
                sessionID: props.sessionID,
                info: pick(info, [
                  "id",
                  "role",
                  "time",
                  "tokens",
                  "cost",
                  "modelID",
                  "providerID",
                  "finish",
                  "agent",
                ]),
              },
            },
          };
        }
        case "message.part.updated": {
          const part = props.part ?? {};
          // A tool that failed is the one state the hooks do not report.
          if (part.type !== "tool" || part.state?.status !== "error") return null;
          return {
            hook,
            payload: {
              type: p.type,
              properties: {
                sessionID: props.sessionID,
                part: {
                  type: part.type,
                  tool: part.tool,
                  callID: part.callID,
                  id: part.id,
                  messageID: part.messageID,
                  state: {
                    status: part.state.status,
                    time: part.state.time,
                    input: pick(part.state.input, ["filePath", "workdir"]),
                  },
                },
              },
            },
          };
        }
        default: {
          const properties = pick(props, [
            "sessionID",
            "file",
            "callID",
            "messageID",
            "permission",
            "permissionID",
            "response",
            "id",
          ]);
          // A permission names its tool call under `tool`; an error is kept to its name.
          if (props.tool && typeof props.tool === "object") {
            properties.tool = pick(props.tool, ["messageID", "callID"]);
          }
          if (props.error && typeof props.error === "object") {
            properties.error = pick(props.error, ["name"]);
          }
          return { hook, payload: { type: p.type, properties } };
        }
      }
    }
    case "chat.message": {
      const message = p.output?.message ?? {};
      const parts = Array.isArray(p.output?.parts) ? p.output.parts : [];
      const chars = parts
        .filter((part) => part && part.type === "text" && typeof part.text === "string")
        .reduce((sum, part) => sum + part.text.length, 0);
      return {
        hook,
        payload: {
          input: pick(p.input, ["sessionID"]),
          output: {
            message: pick(message, ["id", "role", "sessionID", "time", "agent", "model"]),
            chars,
          },
        },
      };
    }
    case "tool.execute.before":
      return {
        hook,
        payload: {
          input: pick(p.input, ["tool", "sessionID", "callID"]),
          output: { args: pick(p.output?.args, ["filePath", "workdir"]) },
        },
      };
    case "tool.execute.after":
      return {
        hook,
        payload: {
          input: {
            ...pick(p.input, ["tool", "sessionID", "callID"]),
            args: pick(p.input?.args, ["filePath", "workdir"]),
          },
          output: { metadata: pick(p.output?.metadata, ["exit", "truncated", "exists"]) },
        },
      };
    case "permission.ask":
      return {
        hook,
        payload: {
          input: pick(p.input, ["id", "type", "permission", "sessionID", "messageID", "callID"]),
          output: pick(p.output, ["status"]),
        },
      };
    default:
      return null;
  }
}

function pick(object, keys) {
  if (!object || typeof object !== "object") return {};
  const out = {};
  for (const key of keys) {
    if (object[key] !== undefined) out[key] = object[key];
  }
  return out;
}

/** Hand one reduced payload to the Yardsort executable. Fire and forget; never waits. */
function deliver(entry, spawnImpl = spawn) {
  const child = spawnImpl(EXE, ["--yardsort-hook", HARNESS, INBOX], {
    stdio: ["pipe", "ignore", "ignore"],
    windowsHide: true,
  });
  child.on("error", () => {});
  child.stdin.on("error", () => {});
  child.stdin.end(JSON.stringify(entry));
}

function send(hook, payload, directory) {
  try {
    const entry = reduce(hook, payload, directory);
    if (entry) deliver(entry);
  } catch {
    // Reporting is never allowed to get in the agent's way.
  }
}

export const YardsortActivity = async (ctx) => {
  // The directory the launch was given: what a file's path is measured against.
  const directory = typeof ctx?.directory === "string" ? ctx.directory : undefined;
  const on = (hook) => async (input, output) => send(hook, { input, output }, directory);
  return {
    event: async ({ event }) => send("event", event, directory),
    "chat.message": on("chat.message"),
    "tool.execute.before": on("tool.execute.before"),
    "tool.execute.after": on("tool.execute.after"),
    "permission.ask": on("permission.ask"),
  };
};

YardsortActivity.reduce = reduce;
