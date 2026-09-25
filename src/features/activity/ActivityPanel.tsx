import { useEffect } from "react";
import { ipc, type ActivityEvent } from "@/lib/ipc";
import { useActivityStore } from "@/stores/activity";
import { useChangesStore } from "@/stores/changes";
import { describeEvent, eventTime, pathOf } from "./describe";

/**
 * The experimental activity timeline: what Yardsort itself recorded about a workspace's
 * processes — starts, exits, resumes and forks — and, when capture is on, what Claude Code
 * reported through its hooks, newest first. Every row says where it came from, so a reader can
 * tell "Yardsort saw the process end" from "the agent says it wrote a file".
 */
export function ActivityPanel({
  workspaceId,
  onClose,
}: {
  workspaceId: string;
  onClose: () => void;
}) {
  const timeline = useActivityStore((s) => s.byWorkspace[workspaceId]);
  const loading = useActivityStore((s) => s.loading[workspaceId]) ?? false;
  const error = useActivityStore((s) => s.error);
  const { load, loadEarlier, clear, dismissError } = useActivityStore.getState();

  useEffect(() => {
    void load(workspaceId);
  }, [workspaceId, load]);

  // Hooks land while the agent works; the core says which workspaces gained events.
  useEffect(() => {
    const unlisten = ipc.onActivityChanged((ids) => {
      if (ids.includes(workspaceId)) void useActivityStore.getState().refresh(workspaceId);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [workspaceId]);

  const events = timeline?.events ?? [];
  return (
    <section
      aria-label="Activity"
      className="flex max-h-[40%] shrink-0 flex-col border-t border-line bg-surface text-[12px]"
    >
      <header className="flex h-7 shrink-0 items-center gap-3 border-b border-line px-3">
        <h2 className="font-semibold">Activity</h2>
        <span className="text-ink-faint">
          Experimental · starts and exits Yardsort saw, and what Claude Code reports when capture is
          on in Settings. Each row names its source.
        </span>
        <span className="flex-1" />
        <button
          type="button"
          disabled={loading}
          onClick={() => void load(workspaceId)}
          className="text-ink-muted hover:text-ink disabled:opacity-40"
        >
          Refresh
        </button>
        <button
          type="button"
          disabled={events.length === 0}
          onClick={() => void clear(workspaceId)}
          title="Forget this workspace's recorded activity. Nothing running is affected."
          className="text-ink-muted hover:text-ink disabled:opacity-40"
        >
          Clear
        </button>
        <button
          type="button"
          aria-label="Close activity"
          onClick={onClose}
          className="text-ink-faint hover:text-ink"
        >
          ×
        </button>
      </header>
      {error && (
        <div role="alert" className="flex items-start gap-3 border-b border-line px-3 py-1">
          <p className="flex-1 text-red-400 select-text">{error}</p>
          <button type="button" onClick={dismissError} className="text-ink-faint hover:text-ink">
            Dismiss
          </button>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {timeline && events.length === 0 ? (
          <p className="px-3 py-2 text-ink-faint">
            Nothing recorded for this workspace yet. Starting an agent or a shell here writes the
            first event.
          </p>
        ) : (
          <ul className="divide-y divide-line">
            {events.map((event) => (
              <Row key={event.id} event={event} />
            ))}
          </ul>
        )}
        {timeline?.hasMore && (
          <button
            type="button"
            disabled={loading}
            onClick={() => void loadEarlier(workspaceId)}
            className="w-full px-3 py-1.5 text-left text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40"
          >
            {loading ? "Loading…" : "Show earlier"}
          </button>
        )}
      </div>
    </section>
  );
}

function Row({ event }: { event: ActivityEvent }) {
  const described = describeEvent(event);
  // The binding types a float as nullable; a timestamp the core wrote is never absent.
  const at = event.occurredAt ?? 0;
  const diff = useDiffOf(event);
  return (
    <li className="flex items-baseline gap-3 px-3 py-1">
      <time
        dateTime={new Date(at).toISOString()}
        className="w-[4.5rem] shrink-0 font-mono text-[11px] text-ink-faint"
      >
        {eventTime(at)}
      </time>
      <span className="min-w-0 flex-1 truncate">
        <span className={described.tone === "bad" ? "text-red-400" : undefined}>
          {described.title}
        </span>
        {described.detail && <span className="text-ink-muted"> · {described.detail}</span>}
      </span>
      {diff && (
        <button
          type="button"
          onClick={diff.open}
          title="Show this file's diff in the Changes panel. The diff is everything that changed since the last commit, whoever changed it."
          className="shrink-0 text-[11px] text-ink-faint hover:text-ink"
        >
          diff
        </button>
      )}
      <span
        className="shrink-0 font-mono text-[10px] text-ink-faint"
        title={`Recorded by ${event.producer} (${event.method}, ${event.fidelity})`}
      >
        {event.producer}/{event.method}
      </span>
    </li>
  );
}

/**
 * The trace link the other way: an event that names a file which is on the change list of
 * the workspace the Changes panel follows can open that file's diff. The link says nothing
 * about which lines the event accounts for; the diff is git's whole answer.
 */
function useDiffOf(event: ActivityEvent): { open: () => void } | null {
  const changesWorkspace = useChangesStore((s) => s.workspaceId);
  const changes = useChangesStore((s) => s.changes);
  const path = pathOf(event);
  if (!path || !changes || changesWorkspace !== event.workspaceId) return null;
  const uncommitted = changes.uncommitted.find((change) => change.path === path);
  const committed = uncommitted ? null : changes.committed.find((change) => change.path === path);
  const change = uncommitted ?? committed;
  if (!change) return null;
  const scope = uncommitted ? "uncommitted" : "committed";
  return { open: () => void useChangesStore.getState().view({ kind: "diff", change, scope }) };
}
