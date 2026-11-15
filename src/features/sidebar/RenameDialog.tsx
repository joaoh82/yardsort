import { useModalFocus } from "@/lib/useModalFocus";
import { useEffect, useId, useRef, useState } from "react";
import type { Workspace } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";

/**
 * Change a workspace's display name. Its folder and branch keep theirs. With `rename`, the
 * workspace is somewhere else (on another computer) and that is how it is renamed there: it
 * resolves to why it could not be, or `null`.
 */
export function RenameDialog({
  workspace,
  onClose,
  rename,
}: {
  workspace: Pick<Workspace, "id" | "name">;
  onClose: () => void;
  rename?: (name: string) => Promise<string | null>;
}) {
  const [name, setName] = useState(workspace.name);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    inputRef.current?.select();
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (rename) {
      const problem = await rename(name);
      return problem === null ? onClose() : setError(problem);
    }
    const renamed = await useProjectsStore.getState().renameWorkspace(workspace.id, name);
    if (renamed) return onClose();
    // The store holds the reason; show it here, where the user is looking.
    setError(useProjectsStore.getState().error);
    useProjectsStore.getState().dismiss();
  };

  const unchanged = name.trim() === workspace.name || name.trim() === "";
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
        className="w-[26rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          Rename workspace
        </h2>
        <input
          ref={inputRef}
          aria-label="Workspace name"
          value={name}
          maxLength={80}
          onChange={(event) => setName(event.target.value)}
          spellCheck={false}
          autoComplete="off"
          className="mt-4 w-full rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
        />
        <p className="mt-2 text-ink-faint">
          Only the name shown here changes. The folder and the branch keep theirs, so running agents
          and saved conversations are not disturbed.
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
            disabled={unchanged}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            Rename
          </button>
        </div>
      </form>
    </div>
  );
}
