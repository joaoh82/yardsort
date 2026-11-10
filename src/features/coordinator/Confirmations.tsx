import { useEffect, useRef, useState } from "react";
import { errorMessage, hasCore, ipc, type CoordinatorConfirmation } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";

/**
 * The yes the Floor Manager's `ys` is waiting for. The panel can be closed — the agent keeps
 * running — so this watches whenever the window is open, and the dialog is the only thing that
 * answers. Answering writes the user's choice; it does not run the command.
 */
export function FloorManagerConfirmations() {
  const [pending, setPending] = useState<CoordinatorConfirmation[]>([]);
  const [gone, setGone] = useState(false);

  useEffect(() => {
    if (!hasCore()) return;
    let cancelled = false;
    const look = () => {
      ipc.coordinatorConfirmations().then(
        (next) => {
          if (!cancelled) setPending(next);
        },
        () => {
          if (!cancelled) setPending([]);
        },
      );
    };
    look();
    const timer = setInterval(look, 1000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  if (gone) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-6">
        <div
          role="alertdialog"
          aria-labelledby="floor-manager-gone-title"
          className="w-[32rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
        >
          <h2 id="floor-manager-gone-title" className="text-base font-semibold">
            No longer waiting
          </h2>
          <p className="mt-2 text-ink-muted">It was not run.</p>
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              autoFocus
              onClick={() => setGone(false)}
              className="h-8 rounded border border-line px-3 text-ink-muted hover:border-accent hover:text-ink"
            >
              Close
            </button>
          </div>
        </div>
      </div>
    );
  }

  const current = pending[0];
  if (!current) return null;
  return (
    <ConfirmDialog
      key={current.id}
      request={current}
      more={pending.length - 1}
      onDone={(id) => setPending((waiting) => waiting.filter((item) => item.id !== id))}
      onGone={(id) => {
        setPending((waiting) => waiting.filter((item) => item.id !== id));
        setGone(true);
      }}
    />
  );
}

function ConfirmDialog({
  request,
  more,
  onDone,
  onGone,
}: {
  request: CoordinatorConfirmation;
  /** How many more are waiting behind this one. */
  more: number;
  onDone: (id: string) => void;
  /** The waiting `ys` is gone: answering would record a yes nothing runs. */
  onGone: (id: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const busyRef = useRef(false);
  useModalFocus(dialogRef);

  const answer = async (allow: boolean) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      const recorded = await ipc.coordinatorAnswer(request.id, allow);
      if (!recorded) {
        onGone(request.id);
        return;
      }
      onDone(request.id);
    } catch (reason) {
      busyRef.current = false;
      setError(errorMessage(reason));
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation();
      void answer(false);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
    // `answer` closes over this request. A new request remounts the dialog.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request.id]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-6">
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="floor-manager-confirm-title"
        className="w-[32rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50 outline-none"
      >
        <h2 id="floor-manager-confirm-title" className="text-base font-semibold">
          The Floor Manager wants to run this
        </h2>
        <p className="mt-2 text-ink-muted">
          Allow runs this one command. Don't allow leaves it unrun.
        </p>
        <pre className="mt-3 overflow-x-auto rounded border border-line bg-canvas p-3 font-mono text-[12px] text-ink select-text">
          {request.command}
        </pre>
        {more > 0 && (
          <p className="mt-2 text-[11px] text-ink-faint">
            {more === 1
              ? "Another is waiting after this one."
              : `${more} more are waiting after this one.`}
          </p>
        )}
        {error && (
          <p role="alert" className="mt-2 text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            autoFocus
            disabled={busy}
            onClick={() => void answer(false)}
            className="h-8 rounded border border-line px-3 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
          >
            Don't allow
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void answer(true)}
            className="h-8 rounded bg-accent px-4 font-medium text-canvas disabled:opacity-40"
          >
            Allow
          </button>
        </div>
      </div>
    </div>
  );
}
