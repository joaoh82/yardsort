import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useRef, useState } from "react";
import type { Workspace } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";

/**
 * Take a workspace out of Yardsort without touching its folder or branch — the way to let go of
 * a worktree that was imported or that belongs to another tool. Its conversations are kept by
 * default, so importing the worktree again finds them; the box below throws them away instead.
 */
export function ForgetDialog({
  workspace,
  onClose,
}: {
  workspace: Workspace;
  onClose: () => void;
}) {
  const [keepHistory, setKeepHistory] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conversations, setConversations] = useState<number | null>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    let stale = false;
    useSessionsStore
      .getState()
      .load(workspace.id)
      .then((records) => !stale && setConversations(records.length));
    return () => {
      stale = true;
    };
  }, [workspace.id]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    await useTerminalStore.getState().closeWorkspaces([workspace.id]);
    const forgotten = await useProjectsStore.getState().forgetWorkspace(workspace.id, keepHistory);
    setBusy(false);
    if (forgotten) return onClose();
    setError(useProjectsStore.getState().error);
    useProjectsStore.getState().dismiss();
  };

  const branch = workspace.head && !workspace.head.detached ? workspace.head.label : null;
  const history =
    conversations === null
      ? "its saved conversations"
      : conversations === 1
        ? "its 1 saved conversation"
        : `its ${conversations} saved conversations`;
  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[22vh]"
      onPointerDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={submit}
        className="w-[28rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Forget workspace “{workspace.name}”?
        </h2>
        <p className="mt-3 text-ink-muted">
          It disappears from Yardsort, and that is all. The folder stays where it is
          {branch ? (
            <>
              , and so does the branch <span className="font-mono text-[12px]">{branch}</span>
            </>
          ) : null}
          . Terminals running in it are closed.
        </p>
        <p className="mt-1 truncate text-[11px] text-ink-faint select-text" title={workspace.path}>
          {workspace.path}
        </p>
        <label className="mt-4 flex cursor-pointer items-start gap-2">
          <input
            type="checkbox"
            checked={!keepHistory}
            disabled={busy || conversations === 0}
            onChange={(event) => setKeepHistory(!event.target.checked)}
            className="mt-1 accent-accent"
          />
          <span>
            Also delete {history}
            <span className="block text-[11px] text-ink-faint">
              Left alone, they are back if this worktree is ever imported again.
            </span>
          </span>
        </label>
        {error && (
          <p role="alert" className="mt-2 text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={busy}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Forgetting…" : "Forget"}
          </button>
        </div>
      </form>
    </div>
  );
}
