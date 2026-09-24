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
}

function parse(event: ActivityEvent): Payload {
  try {
    const value: unknown = JSON.parse(event.payload);
    return typeof value === "object" && value !== null ? (value as Payload) : {};
  } catch {
    return {};
  }
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
      const bits = [p.model, p.launchedBy === "cli" ? "from ys" : null].filter(Boolean);
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
    default:
      // A kind this build does not know — from a newer version, or a later stage's adapter.
      return { title: event.kind, detail: "", tone: "plain" };
  }
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
