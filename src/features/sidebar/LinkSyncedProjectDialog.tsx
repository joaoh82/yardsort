import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage, ipc, type Project, type ProjectSyncStatus, type SyncOffer } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";

/**
 * Link a project here to one another computer of the account syncs. The server lists them, the
 * ones whose remote matches this project's first, but nothing is chosen for the person: linking
 * the wrong repository would share its notes and workspaces with this one.
 *
 * It opens over Project settings, so it lives in a portal (no form inside a form) and keeps its
 * keys and submits to itself: Escape closes only this dialog.
 */
export function LinkSyncedProjectDialog({
  project,
  onClose,
  onLinked,
}: {
  project: Project;
  onClose: () => void;
  onLinked: (status: ProjectSyncStatus) => void;
}) {
  const [offers, setOffers] = useState<SyncOffer[] | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    let stale = false;
    ipc.projectSyncOffers(project.id).then(
      (list) => !stale && setOffers(list),
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [project.id]);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    event.stopPropagation();
    if (chosen === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      const status = await ipc.projectSyncLink(project.id, chosen);
      onLinked(status);
      onClose();
    } catch (reason) {
      setError(errorMessage(reason));
      setBusy(false);
    }
  };

  const none = offers !== null && offers.length === 0;
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 pt-[18vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={(event) => void submit(event)}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          // Project settings is underneath, listening for Escape too.
          event.stopPropagation();
          if (!busy) onClose();
        }}
        className="w-[34rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Link “{project.name}” to a synced project
        </h2>
        <p className="mt-1 text-ink-faint">
          Projects your other computers sync that this one does not yet. Pick the one this project
          is.
        </p>

        {offers === null && !error ? (
          <p className="mt-4 text-ink-muted">Asking the server…</p>
        ) : none ? (
          <p className="mt-4 text-ink-muted">
            No other computer syncs a project you could link here. Turn sync on for it there first.
          </p>
        ) : (
          offers && (
            <>
              <ul aria-label="Synced projects" className="mt-4 max-h-72 space-y-1 overflow-y-auto">
                {offers.map((offer) => (
                  <li key={offer.id}>
                    <label className="flex cursor-pointer items-start gap-2 rounded px-2 py-1.5 hover:bg-raised">
                      <input
                        type="radio"
                        name="synced-project"
                        checked={chosen === offer.id}
                        disabled={busy}
                        onChange={() => setChosen(offer.id)}
                        aria-label={offer.name}
                        className="mt-1 accent-(--color-accent)"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="flex items-baseline gap-2">
                          <span className="truncate font-medium">{offer.name}</span>
                          {offer.matches && (
                            <span className="shrink-0 rounded bg-accent/15 px-1.5 text-micro text-accent">
                              Same remote
                            </span>
                          )}
                        </span>
                        <span className="block truncate font-mono text-small text-ink-faint select-text">
                          {offer.remote ?? "no remote"}
                        </span>
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
              <p className="mt-2 text-ink-faint">
                Link only to the same repository: its notes and workspaces will be shared with this
                one.
              </p>
            </>
          )
        )}

        {error && (
          <p role="alert" className="mt-2 text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={onClose}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            {offers && offers.length > 0 ? "Cancel" : "Close"}
          </button>
          {offers && offers.length > 0 && (
            <button
              type="submit"
              disabled={busy || chosen === null}
              className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            >
              {busy ? "Linking…" : "Link"}
            </button>
          )}
        </div>
      </form>
    </div>,
    document.body,
  );
}
