import { useEffect, useId, useRef, useState } from "react";
import { useModalFocus } from "@/lib/useModalFocus";
import { errorMessage, ipc, type ResolvedLink } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { followLink } from "./links";

/**
 * Start from an issue's or a pull request's address. The link is looked up in the core; its
 * repository's project opens on that item, and a repository that is not a project yet goes to
 * the clone step with the link kept for when the clone is done.
 */
export function StartFromLinkDialog({ onClose }: { onClose: () => void }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, busy]);

  const ready = text.trim() !== "" && !busy;
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    let link: ResolvedLink;
    try {
      link = await ipc.forgeResolveLink(text);
    } catch (failure) {
      setBusy(false);
      setError(errorMessage(failure));
      return;
    }
    if (link.projectId) {
      onClose();
      await followLink(link);
      return;
    }
    // Not a project yet: the clone step, with the repository filled in and the link kept.
    useProjectsStore.getState().openAddProject({ repository: link.cloneUrl, link });
  };

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[18vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={submit}
        className="w-[32rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Start from a link
        </h2>
        <label className="mt-4 grid gap-1">
          <span className="text-ink-muted">Issue or pull request</span>
          <input
            ref={inputRef}
            value={text}
            disabled={busy}
            onChange={(event) => setText(event.target.value)}
            placeholder="https://github.com/owner/repository/issues/12, or owner/repository#12"
            spellCheck={false}
            autoComplete="off"
            className="w-full rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
          />
        </label>
        <p className="mt-1 text-xs text-ink-faint">
          Opens it in Tasks or Pull requests, with Delegate or Start workspace a press away. A
          repository that is not a project yet is cloned first.
        </p>
        {error && (
          <p role="alert" className="mt-3 text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={!ready}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Looking…" : "Open"}
          </button>
        </div>
      </form>
    </div>
  );
}
