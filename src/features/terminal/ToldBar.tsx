import { useEffect, useState } from "react";
import type { TerminalTab } from "@/stores/terminals";
import { ConversationsDialog } from "./ConversationsDialog";

/** How long a new agent's tab says it was told about the other conversations; a mark stays. */
export const TOLD_SHOWN_MS = 60_000;

/**
 * Over an agent this window just started beside other conversations, for its first minute:
 * that it was told it can read them, with what it reads. After that the tab's mark says it.
 */
export function ToldBar({ tab }: { tab: TerminalTab }) {
  const [now, setNow] = useState(() => Date.now());
  const [showing, setShowing] = useState(false);
  const left = tab.toldAt === null ? 0 : tab.toldAt + TOLD_SHOWN_MS - now;

  useEffect(() => {
    if (left <= 0) return;
    const timer = setTimeout(() => setNow(Date.now()), left);
    return () => clearTimeout(timer);
  }, [left]);

  if (!tab.toldAbout || (left <= 0 && !showing)) return null;
  return (
    <>
      {left > 0 && (
        <div
          role="status"
          className="flex shrink-0 items-center gap-2 border-b border-line bg-raised px-3 py-1.5"
        >
          <span className="min-w-0 flex-1 truncate text-ink-muted">
            Told about {tab.toldAbout}: it can read what was said there with{" "}
            <code className="font-mono text-[12px]">ys workspace context</code>.
          </span>
          <button
            type="button"
            onClick={() => setShowing(true)}
            className="rounded border border-line px-3 py-0.5 text-ink-muted hover:border-accent hover:text-ink"
          >
            Show
          </button>
        </div>
      )}
      {showing && (
        <ConversationsDialog
          workspaceId={tab.workspaceId}
          except={tab.recordId}
          onClose={() => setShowing(false)}
        />
      )}
    </>
  );
}
