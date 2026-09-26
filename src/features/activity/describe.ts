import type { ActivityEvent } from "@/lib/ipc";

/** What a stage-1 payload may carry. Everything is optional: the shape depends on the kind. */
interface Payload {
  kind?: string;
  harnessId?: string | null;
  model?: string | null;
  program?: string;
  continuation?: string;
  launchedBy?: string;
  exitCode?: number | null;
  success?: boolean;
  signal?: string | null;
  reason?: string;
  via?: string;
  fromSessionId?: string;
  capture?: string | null;
  // Reported by an agent's hooks: see `crates/core/src/activity/claude.rs`.
  source?: string | null;
  tool?: string | null;
  path?: string | null;
  pathOutsideWorkspace?: boolean;
  subagentType?: string | null;
  durationMs?: number | null;
  chars?: number | null;
  decision?: string | null;
  type?: string | null;
  errorType?: string | null;
  agentType?: string | null;
  trigger?: string | null;
  contextTokens?: number | null;
  // Read from Codex's session file.
  status?: string | null;
  totalTokens?: number | null;
  outputTokens?: number | null;
  detail?: string | null;
  permission?: string | null;
  waitMs?: number | null;
  outcome?: string | null;
  turnNumber?: number | null;
}

function parse(event: ActivityEvent): Payload {
  try {
    const value: unknown = JSON.parse(event.payload);
    return typeof value === "object" && value !== null ? (value as Payload) : {};
  } catch {
    return {};
  }
}

/** The workspace-relative file an event names, if it names one. */
export function pathOf(event: ActivityEvent): string | null {
  return parse(event).path ?? null;
}

/** A row's words: what happened, in the user's terms, and how sure to be of it. */
export function describeEvent(event: ActivityEvent): {
  title: string;
  detail: string;
  tone: "ok" | "bad" | "plain";
} {
  const p = parse(event);
  const who = p.harnessId ?? p.program ?? (p.kind === "shell" ? "shell" : "process");
  switch (event.kind) {
    case "process.started": {
      const how =
        p.continuation === "resumed"
          ? "resumed"
          : p.continuation === "forked"
            ? "forked"
            : "started";
      const bits = [
        p.model,
        p.launchedBy === "cli" ? "from ys" : null,
        p.capture ? "reporting through hooks" : null,
      ].filter(Boolean);
      return { title: `${who} ${how}`, detail: bits.join(" · "), tone: "plain" };
    }
    case "process.exited": {
      if (p.exitCode === null || p.exitCode === undefined) {
        return {
          title: "ended without an exit status",
          detail: `interrupted · ${howKnown(p.via)}`,
          tone: "bad",
        };
      }
      const signal = p.signal ? ` (${p.signal})` : "";
      return {
        title: p.success ? "exited" : `exited with ${p.exitCode}${signal}`,
        detail: howKnown(p.via),
        tone: p.success ? "ok" : "bad",
      };
    }
    case "process.spawn_failed":
      return { title: `${who} could not start`, detail: p.reason ?? "", tone: "bad" };
    case "session.resumed":
      return { title: "conversation resumed", detail: "", tone: "plain" };
    case "session.forked":
      return {
        title: "conversation forked",
        detail: p.fromSessionId ? `from ${p.fromSessionId.slice(0, 8)}` : "",
        tone: "plain",
      };
    // What the agent reported. The words say "reported": the agent is the witness here.
    case "session.started":
      return {
        title:
          p.source === "resume"
            ? "agent resumed its session"
            : p.source === "fork"
              ? "agent forked its session"
              : "agent session started",
        detail:
          p.model ??
          (p.contextTokens ? `${p.contextTokens.toLocaleString()} tokens of context` : ""),
        tone: "plain",
      };
    case "session.ended":
      return { title: "agent session ended", detail: p.reason ?? "", tone: "plain" };
    case "prompt.submitted":
      return {
        title: "prompt submitted",
        detail: p.chars !== null && p.chars !== undefined ? `${p.chars} characters` : "",
        tone: "plain",
      };
    case "tool.started":
      return { title: `${toolName(p)} started`, detail: toolDetail(p), tone: "plain" };
    case "tool.completed":
      return {
        title: `${toolName(p)} done`,
        detail: [
          toolDetail(p),
          p.exitCode !== null && p.exitCode !== undefined ? `exit ${p.exitCode}` : null,
          p.durationMs !== null && p.durationMs !== undefined ? `${p.durationMs} ms` : null,
        ]
          .filter(Boolean)
          .join(" · "),
        tone: "plain",
      };
    case "tool.failed":
      return {
        title: `${toolName(p)} failed`,
        detail: [
          toolDetail(p),
          p.exitCode !== null && p.exitCode !== undefined ? `exit ${p.exitCode}` : null,
        ]
          .filter(Boolean)
          .join(" · "),
        tone: "bad",
      };
    case "file.reported_write": {
      const what = p.kind === "add" ? "added" : p.kind === "delete" ? "deleted" : "changed";
      return { title: `file ${what}`, detail: toolDetail(p), tone: "plain" };
    }
    case "usage.reported":
      return {
        title: "tokens used",
        detail:
          p.totalTokens !== null && p.totalTokens !== undefined
            ? `${p.totalTokens.toLocaleString()} total${
                p.outputTokens !== null && p.outputTokens !== undefined
                  ? ` · ${p.outputTokens.toLocaleString()} out`
                  : ""
              }`
            : "",
        tone: "plain",
      };
    case "approval.requested":
      return { title: `permission asked for ${toolName(p)}`, detail: toolDetail(p), tone: "plain" };
    case "approval.resolved":
      return {
        title: `${toolName(p)} ${p.decision ?? "resolved"}`,
        detail: [
          toolDetail(p),
          p.waitMs !== null && p.waitMs !== undefined && p.waitMs > 0
            ? `you took ${(p.waitMs / 1000).toFixed(1)} s`
            : null,
        ]
          .filter(Boolean)
          .join(" · "),
        tone: p.decision === "denied" || p.decision === "deny" ? "bad" : "plain",
      };
    case "turn.started":
      return {
        title: "agent started a turn",
        detail: [
          p.turnNumber !== null && p.turnNumber !== undefined ? `turn ${p.turnNumber}` : null,
          p.model,
        ]
          .filter(Boolean)
          .join(" · "),
        tone: "plain",
      };
    case "agent.notified":
      return { title: "agent raised a notification", detail: p.type ?? "", tone: "plain" };
    case "turn.completed":
      return {
        title: "agent finished its turn",
        detail: [
          p.durationMs !== null && p.durationMs !== undefined
            ? `${(p.durationMs / 1000).toFixed(1)} s`
            : null,
          // The pi family and Cursor report a turn's tokens on the turn itself.
          p.totalTokens !== null && p.totalTokens !== undefined
            ? `${p.totalTokens.toLocaleString()} tokens`
            : null,
          p.detail === "notify" ? "from notify alone; the session file could not be read" : null,
        ]
          .filter(Boolean)
          .join(" · "),
        tone: "ok",
      };
    case "turn.failed":
      return {
        title: p.outcome === "interrupted" ? "agent's turn was interrupted" : "agent's turn failed",
        detail: p.errorType ?? (p.outcome !== "interrupted" ? (p.outcome ?? "") : ""),
        tone: "bad",
      };
    case "agent.subagent_started":
      return { title: "subagent started", detail: p.agentType ?? "", tone: "plain" };
    case "agent.subagent_stopped":
      return { title: "subagent stopped", detail: p.agentType ?? "", tone: "plain" };
    case "session.compacted":
      return { title: "agent compacted its context", detail: p.trigger ?? "", tone: "plain" };
    case "agent.model_switched":
      return { title: "agent switched model", detail: p.model ?? "", tone: "plain" };
    default:
      // A kind this build does not know — from a newer version, or a later stage's adapter.
      return { title: event.kind, detail: "", tone: "plain" };
  }
}

function toolName(p: Payload): string {
  const name = p.tool ?? p.permission;
  return name ? `${name}${p.subagentType ? ` (${p.subagentType})` : ""}` : "tool";
}

function toolDetail(p: Payload): string {
  if (p.path) return p.path;
  return p.pathOutsideWorkspace ? "a file outside the workspace" : "";
}

/** How the exit came to be known, in words a reader can weigh. */
function howKnown(via: string | undefined): string {
  switch (via) {
    case "live":
      return "seen as it happened";
    case "settle":
      return "found already ended at launch";
    case "spool":
      return "kept by the background process while Yardsort was closed";
    case "reconcile":
      return "process gone when Yardsort started";
    default:
      return via ?? "";
  }
}

/** A clock time for a row, in the user's locale. */
export function eventTime(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
