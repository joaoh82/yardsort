import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";
import { describeEnd, timeAgo } from "./sessionText";

/**
 * The conversations a workspace has had that are not on screen right now. This is what makes
 * "quit mid-task, relaunch, carry on" two clicks: pick the workspace, press Resume.
 */
export function SessionHistory({ workspaceId }: { workspaceId: string }) {
  const records = useSessionsStore((s) => s.byWorkspace[workspaceId]);
  // Select the stable array and derive from it: a selector that builds a new array on every
  // call never compares equal, and the component would re-render forever.
  const tabs = useTerminalStore((s) => s.tabs);
  const openRecordIds = tabs
    .filter((tab) => tab.workspaceId === workspaceId)
    .map((tab) => tab.recordId);
  const resume = useTerminalStore((s) => s.resume);
  const fork = useTerminalStore((s) => s.fork);
  const forget = useSessionsStore((s) => s.forget);

  const past = (records ?? []).filter((r) => !r.running && !openRecordIds.includes(r.id));
  if (past.length === 0) return null;

  return (
    <section aria-label="Previous sessions" className="mx-auto mt-6 w-full max-w-xl text-left">
      <h2 className="mb-2 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
        Previous sessions
      </h2>
      <ul className="divide-y divide-line rounded-md border border-line">
        {past.map((record, index) => (
          <li key={record.id} className="flex items-center gap-3 px-3 py-2">
            <HarnessIcon id={record.harnessId} label={record.harnessLabel} />
            <div className="min-w-0 flex-1">
              <div className="truncate">
                <span className="font-medium">{record.harnessLabel}</span>
                {record.forkedFrom && <span className="text-ink-faint"> · fork</span>}
                {record.title && <span className="text-ink-muted"> — {record.title}</span>}
              </div>
              <div className="truncate text-[11px] text-ink-faint">
                {timeAgo(record.endedAt ?? record.startedAt)} · {describeEnd(record)}
                {record.model ? ` · ${record.model}` : ""}
                {record.unavailableReason ? ` · ${record.unavailableReason}` : ""}
              </div>
            </div>
            <button
              type="button"
              disabled={!record.resumable}
              title={record.unavailableReason ?? "Continue this conversation"}
              onClick={() => void resume(record.id)}
              className={`shrink-0 rounded px-3 py-1 disabled:opacity-40 ${
                index === 0
                  ? "bg-accent font-medium text-canvas"
                  : "border border-line text-ink-muted hover:border-accent hover:text-ink"
              }`}
            >
              Resume
            </button>
            <button
              type="button"
              disabled={!record.forkable}
              title={record.unavailableReason ?? "Start a copy that goes its own way"}
              onClick={() => void fork(record.id)}
              className="shrink-0 rounded border border-line px-3 py-1 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
            >
              Fork
            </button>
            <button
              type="button"
              aria-label={`Forget ${record.harnessLabel} session${record.title ? `: ${record.title}` : ""}`}
              title="Remove from this list (the harness keeps its own copy)"
              onClick={() => void forget(record)}
              className="shrink-0 rounded px-1.5 text-ink-faint hover:bg-line hover:text-ink"
            >
              ×
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
