import { useEffect, useState } from "react";
import { TerminalView } from "@/features/terminal/TerminalView";
import { errorMessage, ipc, type HarnessDef, type SessionId } from "@/lib/ipc";
import { useTerminalStore } from "@/stores/terminals";
import { buttonClass } from "./fields";

/**
 * Start a definition — saved or not — with no prompt, in the home directory, and show it. The
 * session is closed together with this dialog.
 */
export function TestLaunch({ def, onClose }: { def: HarnessDef; onClose: () => void }) {
  const [sessionId, setSessionId] = useState<SessionId | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let started: SessionId | null = null;
    let cancelled = false;
    ipc.harnessTest(def, useTerminalStore.getState().lastSize).then(
      (session) => {
        if (cancelled) return void ipc.ptyClose(session.id).catch(() => {});
        started = session.id;
        setSessionId(session.id);
      },
      (reason) => !cancelled && setError(errorMessage(reason)),
    );
    return () => {
      cancelled = true;
      if (started) void ipc.ptyClose(started).catch(() => {});
    };
  }, [def]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-8">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Test launch: ${def.label}`}
        className="flex h-full max-h-[40rem] w-full max-w-4xl flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="flex h-11 shrink-0 items-center justify-between border-b border-line px-3">
          <h2 className="font-medium">
            Test launch: {def.label}{" "}
            <span className="text-ink-faint">(home directory, no prompt)</span>
          </h2>
          <button type="button" onClick={onClose} className={buttonClass}>
            Close
          </button>
        </header>
        <div className="min-h-0 flex-1">
          {error ? (
            <p role="alert" className="p-4 text-red-400 select-text">
              {error}
            </p>
          ) : (
            sessionId && <TerminalView sessionId={sessionId} />
          )}
        </div>
      </div>
    </div>
  );
}
