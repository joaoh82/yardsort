import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, type Workspace } from "@/lib/ipc";
import { trackPullRequest } from "@/stores/publish";

/**
 * Follow a pull request in another repository with a workspace: its number and checks then show
 * on the workspace's row and in its toolbar, and it is listed in the Pull requests view. For one
 * an agent opened in a nested clone without saying so with `ys pr track`.
 */
export function TrackPullRequestDialog({
  workspace,
  projectId,
  onClose,
}: {
  workspace: Workspace;
  projectId: string;
  onClose: () => void;
}) {
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    inputRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await trackPullRequest(projectId, workspace.id, url);
      onClose();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

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
        className="w-[30rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Track pull request
        </h2>
        <input
          ref={inputRef}
          aria-label="Pull request link"
          placeholder="https://github.com/owner/repo/pull/12"
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          spellCheck={false}
          autoComplete="off"
          className="mt-4 w-full rounded border border-line bg-canvas px-2 py-1.5 font-mono outline-none select-text focus:border-accent"
        />
        <p className="mt-2 text-ink-faint">
          A pull request in another repository — one of the nested clones in this project's folder,
          say — that came out of {workspace.name}. Its checks and reviews show with this workspace,
          beside its own. Agents can do the same with <code>ys pr track</code>.
        </p>
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
            disabled={busy || url.trim() === ""}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Tracking…" : "Track"}
          </button>
        </div>
      </form>
    </div>
  );
}
