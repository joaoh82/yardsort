import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { describeEnd } from "./sessionText";

/** Shown over a harness terminal whose process has ended: carry on from here, or let it go. */
export function EndedBar({ tab }: { tab: TerminalTab }) {
  const record = useSessionsStore((s) =>
    s.byWorkspace[tab.workspaceId]?.find((candidate) => candidate.id === tab.recordId),
  );
  const resume = useTerminalStore((s) => s.resume);
  const fork = useTerminalStore((s) => s.fork);
  const close = useTerminalStore((s) => s.close);
  if (!tab.exit || !record) return null;

  const button =
    "rounded border border-line px-3 py-0.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40";
  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b border-line bg-raised px-3 py-1.5"
    >
      <HarnessIcon id={record.harnessId} label={record.harnessLabel} />
      <span className="min-w-0 flex-1 truncate text-ink-muted">
        {record.harnessLabel} {describeEnd(record)}.
        {record.unavailableReason ? ` ${record.unavailableReason}` : ""}
      </span>
      <button
        type="button"
        disabled={!record.resumable}
        onClick={() => void resume(record.id, tab.id)}
        className="rounded bg-accent px-3 py-0.5 font-medium text-canvas disabled:opacity-40"
      >
        Resume
      </button>
      <button
        type="button"
        disabled={!record.forkable}
        onClick={() => void fork(record.id)}
        className={button}
      >
        Fork
      </button>
      <button type="button" onClick={() => void close(tab.id)} className={button}>
        Close
      </button>
    </div>
  );
}
