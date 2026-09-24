import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, ipc, type Project, type UntrackedWorktree } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";

/**
 * Bring worktrees made elsewhere in as workspaces. Yardsort only picks up worktrees under its
 * own folder by itself; anything git knows about beyond that waits here until asked for, listed
 * with its branch and path so nothing is imported blind. Nothing on disk is touched either way.
 */
export function ImportWorktreesDialog({
  project,
  onClose,
}: {
  project: Project;
  onClose: () => void;
}) {
  const [found, setFound] = useState<UntrackedWorktree[] | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    let stale = false;
    ipc.projectUntrackedWorktrees(project.id).then(
      (list) => {
        if (stale) return;
        setFound(list);
        setChosen(new Set(list.map((worktree) => worktree.path)));
      },
      (reason) => !stale && setError(errorMessage(reason)),
    );
    return () => {
      stale = true;
    };
  }, [project.id]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const toggle = (path: string) =>
    setChosen((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (chosen.size === 0 || busy) return;
    setBusy(true);
    const imported = await useProjectsStore.getState().importWorktrees(project.id, [...chosen]);
    setBusy(false);
    if (imported) return onClose();
    setError(useProjectsStore.getState().error);
    useProjectsStore.getState().dismiss();
  };

  const none = found !== null && found.length === 0;
  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[18vh]"
      onPointerDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={submit}
        className="w-[34rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Import worktrees into {project.name}
        </h2>
        <p className="mt-1 text-ink-faint">
          Worktrees git knows about that are not workspaces yet. Importing only tells Yardsort about
          them: nothing is moved, and no branch is changed.
        </p>

        {found === null && !error ? (
          <p className="mt-4 text-ink-muted">Asking git…</p>
        ) : none ? (
          <p className="mt-4 text-ink-muted">
            Nothing to import. Every worktree of this project is a workspace already.
          </p>
        ) : (
          <ul aria-label="Worktrees" className="mt-4 max-h-72 space-y-1 overflow-y-auto">
            {found?.map((worktree) => {
              const name = worktree.path.split(/[\\/]/).filter(Boolean).pop() ?? worktree.path;
              return (
                <li key={worktree.path}>
                  <label className="flex cursor-pointer items-start gap-2 rounded px-2 py-1.5 hover:bg-raised">
                    <input
                      type="checkbox"
                      checked={chosen.has(worktree.path)}
                      disabled={busy}
                      onChange={() => toggle(worktree.path)}
                      aria-label={name}
                      className="mt-1 accent-accent"
                    />
                    <span className="min-w-0 flex-1">
                      <span className="flex items-baseline gap-2">
                        <span className="truncate font-medium">{name}</span>
                        <span className="truncate font-mono text-[11px] text-ink-faint">
                          {worktree.branch ?? "detached"}
                        </span>
                      </span>
                      <span className="block truncate text-[11px] text-ink-faint select-text">
                        {worktree.path}
                      </span>
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}

        {error && (
          <p role="alert" className="mt-2 text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex items-center justify-end gap-2">
          {found && found.length > 1 && (
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                setChosen(
                  chosen.size === found.length
                    ? new Set()
                    : new Set(found.map((worktree) => worktree.path)),
                )
              }
              className="mr-auto text-ink-muted hover:text-ink"
            >
              {chosen.size === found.length ? "Select none" : "Select all"}
            </button>
          )}
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            {none ? "Close" : "Cancel"}
          </button>
          {!none && (
            <button
              type="submit"
              disabled={busy || chosen.size === 0}
              className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            >
              {busy
                ? "Importing…"
                : chosen.size === 1
                  ? "Import 1 worktree"
                  : `Import ${chosen.size} worktrees`}
            </button>
          )}
        </div>
      </form>
    </div>
  );
}
