import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useRef, useState } from "react";
import type { Project } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";

/**
 * Take a project off the list. Nothing on disk is ever touched: the folder, its branches and
 * its worktrees stay. What the box decides is Yardsort's own record — by default the
 * workspaces and their conversations wait, hidden, and adding the same folder again brings
 * them all back, imported worktrees included.
 */
export function RemoveProjectDialog({
  project,
  onClose,
}: {
  project: Project;
  onClose: () => void;
}) {
  const [keepHistory, setKeepHistory] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const allTabs = useTerminalStore((s) => s.tabs);
  const running = allTabs.filter(
    (tab) => !tab.exit && project.workspaces.some((w) => w.id === tab.workspaceId),
  ).length;
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    await useTerminalStore.getState().closeWorkspaces(project.workspaces.map((w) => w.id));
    const removed = await useProjectsStore.getState().remove(project.id, keepHistory);
    setBusy(false);
    if (removed) return onClose();
    setError(useProjectsStore.getState().error);
    useProjectsStore.getState().dismiss();
  };

  const workspaces = project.workspaces.filter((w) => w.kind === "worktree").length;
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
          Remove “{project.name}” from Yardsort?
        </h2>
        <p className="mt-3 text-ink-muted">
          Nothing on disk is deleted: the folder, its branches and its worktrees stay exactly as
          they are.
          {running > 0
            ? ` ${running} running terminal session${running === 1 ? "" : "s"} in this project will be closed.`
            : ""}
        </p>
        <label className="mt-4 flex cursor-pointer items-start gap-2">
          <input
            type="checkbox"
            checked={!keepHistory}
            disabled={busy}
            onChange={(event) => setKeepHistory(!event.target.checked)}
            className="mt-1 accent-accent"
          />
          <span>
            Also delete its {workspaces === 1 ? "workspace" : `${workspaces} workspaces`} and their
            saved conversations from Yardsort
            <span className="block text-[11px] text-ink-faint">
              Left alone, they are all back — imported worktrees too — when this folder is added
              again.
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
            className="rounded bg-red-500 px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Removing…" : "Remove"}
          </button>
        </div>
      </form>
    </div>
  );
}
