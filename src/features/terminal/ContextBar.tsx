import { useEffect, useState } from "react";
import { ipc, type ContextUsage } from "@/lib/ipc";
import type { TerminalTab } from "@/stores/terminals";
import { band, SUBMIT_DELAY_MS, useDismissedContext } from "./contextHint";

/**
 * Shown over a running agent whose context is nearly full, offering to compact it. What the
 * agent reported comes from the core; the bar only decides whether to show it.
 */
export function ContextBar({ tab }: { tab: TerminalTab }) {
  const [reading, setReading] = useState<{ recordId: string; usage: ContextUsage | null }>();
  const recordId = tab.recordId;
  const running = tab.exit === null;
  // Until this conversation's own reading arrives (a tab or workspace switch), what is held is
  // another's, or nothing: it says nothing about this one.
  const loaded = running && reading?.recordId === recordId;
  const usage = loaded ? reading.usage : null;
  const dismissed = useDismissedContext((s) => (recordId ? (s.byRecord[recordId] ?? 0) : 0));
  const setDismissed = useDismissedContext((s) => s.set);

  useEffect(() => {
    if (!recordId || !running) return;
    let stale = false;
    const refresh = () =>
      ipc.sessionContext(recordId).then(
        (next) => !stale && setReading({ recordId, usage: next }),
        () => {},
      );
    void refresh();
    const unlisten = ipc.onActivityChanged((ids) => {
      if (ids.includes(tab.workspaceId)) void refresh();
    });
    return () => {
      stale = true;
      void unlisten.then((stop) => stop());
    };
  }, [recordId, running, tab.workspaceId]);

  const current = usage ? band(usage.percent) : 0;
  // Below the threshold — a compaction, or a fresh start — a later climb is news again.
  useEffect(() => {
    if (recordId && loaded && current === 0 && dismissed !== 0) setDismissed(recordId, 0);
  }, [recordId, loaded, current, dismissed, setDismissed]);

  if (!recordId || !usage?.suggest || !usage.compactCommand || current <= dismissed) return null;
  const command = usage.compactCommand;

  const compact = async () => {
    setDismissed(recordId, current);
    await ipc.ptyWrite(tab.id, command);
    await new Promise((resolve) => setTimeout(resolve, SUBMIT_DELAY_MS));
    await ipc.ptyWrite(tab.id, "\r");
  };

  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b border-line bg-raised px-3 py-1.5"
    >
      <span className="min-w-0 flex-1 truncate text-ink-muted">
        Context {usage.percent}% full — {usage.usedTokens.toLocaleString()} of{" "}
        {usage.windowTokens.toLocaleString()} tokens. Compacting summarises the conversation so far
        and frees room.
      </span>
      <button
        type="button"
        onClick={() => void compact().catch(console.error)}
        // Mid-turn, the agent would only queue it.
        disabled={tab.busy}
        title={
          tab.busy
            ? "Waits until the agent is idle"
            : `Types ${command} into the agent's prompt and presses Enter`
        }
        className="rounded bg-accent px-3 py-0.5 font-medium text-canvas disabled:opacity-40"
      >
        Compact
      </button>
      <button
        type="button"
        onClick={() => setDismissed(recordId, current)}
        className="rounded border border-line px-3 py-0.5 text-ink-muted hover:border-accent hover:text-ink"
      >
        Not now
      </button>
    </div>
  );
}
