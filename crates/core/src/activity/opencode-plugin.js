// Yardsort's OpenCode plugin: what the agent does, reported to the activity timeline.
//
// OpenCode loads this for one launch, through `OPENCODE_CONFIG_CONTENT`, when "Capture what
// OpenCode reports" is on. One module for both of OpenCode's plugin APIs: its default export is
// an object that OpenCode 1 (1.18.29 and later) starts through `server`, and OpenCode 2 through
// `setup`, with an `id`. The two report different shapes; each delivery says which API it came
// from (`api: 2` for OpenCode 2), and the Yardsort side reads each by its own rules. It listens
// to a few of OpenCode's plugin hooks and bus events, keeps
// the metadata of each — a tool's name, a file's path, an exit code, token counts, ids — and
// hands it to the Yardsort executable in hook mode, which writes one file to the activity inbox.
// Nothing here reads a prompt, a command, a file's contents or a reply: `reduce` copies only the
// fields the Yardsort side records, and everything else stops at this process's memory. One
// exception: a call that may have opened a pull request — a `bash` command naming `gh`, `pr` and
// `create`, or a GitHub server's create-pull-request tool — hands its command and its output to
// the executable, which keeps the new pull request's link and nothing else of either.
//
// Written from a template in the Yardsort binary before each launch, so the executable it names
// is the one that launched. Never throws into OpenCode: every hook swallows its own failures.
//
// OpenCode 2 runs sessions in a background service that every launch shares unless it is told
// `--standalone`, which Yardsort adds. A copy of this plugin that finds itself in the shared
// service reports nothing: that service's environment is whichever launch started it, so what
// it saw would be credited to the wrong run, or to a run that has ended.

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
 * The part of an OpenCode 1 hook's payload that Yardsort records, or null when the call is not
 * one of ours. Pure, and the shape is the raw payload's with fields left out, so the Yardsort
 * side reads a recorded fixture and a live delivery the same way. Not an export of its own:
 * OpenCode 1 calls every export as a plugin, and a function that answers null to that is a
 * crash on start-up. It hangs off the plugin's definition instead, for the tests.
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
    case "tool.execute.after": {
      const opened = mayOpenPullRequest(p.input?.tool, p.input?.args);
      return {
        hook,
        payload: {
          input: {
            ...pick(p.input, ["tool", "sessionID", "callID"]),
            args: pick(p.input?.args, ["filePath", "workdir", ...(opened ? ["command"] : [])]),
          },
          output: {
            metadata: pick(p.output?.metadata, ["exit", "truncated", "exists"]),
            ...(opened ? pick(p.output, ["output", "content"]) : {}),
          },
        },
      };
    }
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

/**
 * Whether a call may have opened a pull request: a shell command naming `gh`, `pr` and `create`
 * (OpenCode 1's shell tool is `bash`, 2's is `shell`), or a tool whose name says GitHub and ends
 * `create_pull_request` (OpenCode names an MCP tool `<server>_<tool>`). Loose on purpose — the
 * executable decides, by the same rule as for every other agent; this only keeps every other
 * command's output in this process.
 */
function mayOpenPullRequest(tool, args) {
  if (typeof tool !== "string") return false;
  if (tool === "bash" || tool === "shell") {
    const command = args?.command;
    return (
      typeof command === "string" &&
      /\bgh(\.exe)?\b/.test(command) &&
      /\bpr\b/.test(command) &&
      /\bcreate\b/.test(command)
    );
  }
  return tool.endsWith("create_pull_request") && /github/i.test(tool);
}

/** OpenCode 2's bus events worth a file. Everything else — deltas, catalogues — is not. */
const EVENTS_V2 = [
  "session.created",
  "session.step.started",
  "session.step.ended",
  "session.execution.succeeded",
  "session.execution.failed",
  "session.execution.interrupted",
  "permission.asked",
  "permission.replied",
];

/**
 * The part of an OpenCode 2 hook's event, or bus event, that Yardsort records, or null when it
 * is not one of ours. `seen` is the plugin's memory between calls — when each tool call began,
 * and which model each step ran — so a failed call carries its duration and a step's usage its
 * model; fields the raw event does not have are marked as the plugin's (`durationMs`, `model`).
 */
function reduceV2(hook, event, directory, seen = newSeen()) {
  const reduced = reduceHookV2(hook, event ?? {}, seen);
  if (!reduced) return null;
  reduced.api = 2;
  if (typeof directory === "string") reduced.directory = directory;
  return reduced;
}

function newSeen() {
  return { started: new Map(), models: new Map() };
}

function reduceHookV2(hook, e, seen) {
  switch (hook) {
    case "event": {
      if (!EVENTS_V2.includes(e.type)) return null;
      const data = e.data ?? {};
      switch (e.type) {
        case "session.created":
          return {
            hook,
            payload: {
              type: e.type,
              data: pick(data, ["sessionID", "version", "parentID", "agent", "model"]),
            },
          };
        case "session.step.started":
          // Not a file of its own: the model the step's usage, at its end, is reported with.
          if (typeof data.assistantMessageID === "string" && data.model) {
            seen.models.set(data.assistantMessageID, pick(data.model, ["id", "providerID"]));
            if (seen.models.size > 256) seen.models.delete(seen.models.keys().next().value);
          }
          return null;
        case "session.step.ended": {
          const model = seen.models.get(data.assistantMessageID);
          seen.models.delete(data.assistantMessageID);
          return {
            hook,
            payload: {
              type: e.type,
              data: {
                ...pick(data, ["sessionID", "assistantMessageID", "finish", "cost", "tokens"]),
                ...(model ? { model } : {}),
              },
            },
          };
        }
        case "session.execution.failed":
          return {
            hook,
            payload: {
              type: e.type,
              data: {
                ...pick(data, ["sessionID"]),
                ...(data.error && typeof data.error === "object"
                  ? { error: pick(data.error, ["type", "_tag", "name"]) }
                  : {}),
              },
            },
          };
        case "permission.asked":
          // Its `resources` are what it asks about — a command, a path: content, left here.
          return {
            hook,
            payload: {
              type: e.type,
              data: {
                ...pick(data, ["id", "sessionID", "action"]),
                source: pick(data.source, ["type", "messageID", "id"]),
              },
            },
          };
        case "permission.replied":
          return {
            hook,
            payload: { type: e.type, data: pick(data, ["sessionID", "requestID", "reply"]) },
          };
        default:
          return { hook, payload: { type: e.type, data: pick(data, ["sessionID"]) } };
      }
    }
    case "session.prompt": {
      const text = e.prompt?.text;
      return {
        hook,
        payload: {
          ...pick(e, ["sessionID", "messageID"]),
          chars: typeof text === "string" ? text.length : 0,
        },
      };
    }
    case "tool.execute.before":
      if (typeof e.id === "string") {
        seen.started.set(callKey(e), Date.now());
        if (seen.started.size > 256) seen.started.delete(seen.started.keys().next().value);
      }
      return {
        hook,
        payload: {
          ...pick(e, ["tool", "sessionID", "messageID", "id"]),
          input: pick(e.input, ["path", "filePath", "workdir"]),
        },
      };
    case "tool.execute.after": {
      const began = seen.started.get(callKey(e));
      seen.started.delete(callKey(e));
      const opened = e.status === "completed" && mayOpenPullRequest(e.tool, e.input);
      const result = e.result ?? {};
      const output = result.output && typeof result.output === "object" ? result.output : {};
      return {
        hook,
        payload: {
          ...pick(e, ["tool", "sessionID", "messageID", "id", "status"]),
          input: pick(e.input, ["path", "filePath", "workdir", ...(opened ? ["command"] : [])]),
          ...(e.status === "completed"
            ? {
                result: {
                  output: pick(output, [
                    "exit",
                    "truncated",
                    "existed",
                    "operation",
                    ...(opened ? ["output"] : []),
                  ]),
                  metadata: pick(result.metadata, ["exit", "truncated"]),
                  ...(opened && !mayOpenByShell(e.tool) ? pick(result, ["content"]) : {}),
                },
              }
            : {}),
          ...(e.status === "error" && typeof began === "number"
            ? { durationMs: Math.max(0, Date.now() - began) }
            : {}),
        },
      };
    }
    default:
      return null;
  }
}

/** A call's key: its tool and id, as an MCP call inside `execute` has the outer call's id. */
function callKey(e) {
  return `${e.tool}:${e.id}`;
}

function mayOpenByShell(tool) {
  return tool === "bash" || tool === "shell";
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

function send(entry) {
  try {
    if (entry) deliver(entry);
  } catch {
    // Reporting is never allowed to get in the agent's way.
  }
}

function attempt(reduceCall) {
  try {
    return reduceCall();
  } catch {
    return null;
  }
}

/** OpenCode 1: a function of its context that answers with hooks by name. */
async function server(ctx) {
  // The directory the launch was given: what a file's path is measured against.
  const directory = typeof ctx?.directory === "string" ? ctx.directory : undefined;
  const report = (hook, payload) => send(attempt(() => reduce(hook, payload, directory)));
  const on = (hook) => async (input, output) => report(hook, { input, output });
  return {
    event: async ({ event }) => report("event", event),
    "chat.message": on("chat.message"),
    "tool.execute.before": on("tool.execute.before"),
    "tool.execute.after": on("tool.execute.after"),
    "permission.ask": on("permission.ask"),
  };
}

/** Whether this process is OpenCode 2's shared background service, not a launch's own server. */
function inSharedService(argv = process.argv) {
  return Array.isArray(argv) && argv.includes("--service");
}

/** OpenCode 2: registers its hooks and its event subscription, and answers with the cleanup. */
function setup(ctx) {
  return start(ctx, process.argv);
}

/** `setup`, with the process's arguments given, for the tests. */
function start(ctx, argv) {
  if (inSharedService(argv)) return undefined;
  const directory =
    typeof ctx?.location?.directory === "string" ? ctx.location.directory : undefined;
  const seen = newSeen();
  const report = (hook, event) => send(attempt(() => reduceV2(hook, event, directory, seen)));
  const controller = new AbortController();
  void (async () => {
    try {
      const events = ctx.event.subscribe({ types: EVENTS_V2, signal: controller.signal });
      for await (const event of events) report("event", event);
    } catch {
      // A stream that ends or cannot start ends the reporting of events, and nothing else.
    }
  })();
  const hooks = [
    ["tool", "execute.before", "tool.execute.before"],
    ["tool", "execute.after", "tool.execute.after"],
    ["session", "prompt", "session.prompt"],
  ];
  const registered = hooks.map(([domain, name, hook]) => {
    try {
      return Promise.resolve(ctx[domain].hook(name, (event) => report(hook, event))).catch(
        () => undefined,
      );
    } catch {
      return Promise.resolve(undefined);
    }
  });
  return Promise.all(registered).then(() => () => controller.abort());
}

export default {
  id: "yardsort.activity",
  server,
  setup,
  // For the tests, not for OpenCode: neither API calls these.
  testing: { reduce, reduceV2, mayOpenPullRequest, inSharedService, start },
};
