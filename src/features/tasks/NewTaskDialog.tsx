import { useEffect, useId, useRef, useState } from "react";
import type { Project } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useTasksStore } from "@/stores/tasks";
import { rowKey } from "./rows";

const field =
  "w-full rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent";

/**
 * Open a new task: an issue on the chosen project's repository.
 *
 * Create is the confirmation, and the dialog says what it does — the issue is public the moment
 * it is made, and the people watching the repository are told. Nothing is sent until then, and
 * a refusal leaves everything typed where it was.
 */
export function NewTaskDialog({
  projects,
  initial,
  onClose,
}: {
  /** The projects a task can be opened in: on GitHub, with issues on. */
  projects: Project[];
  /** The one to start on. */
  initial: string;
  onClose: () => void;
}) {
  const [projectId, setProjectId] = useState(initial);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [labels, setLabels] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const choices = useTasksStore((s) => s.choices[projectId]);
  const titleRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, busy]);
  useEffect(() => {
    void useTasksStore.getState().loadChoices(projectId);
  }, [projectId]);

  const project = projects.find((it) => it.id === projectId);
  const ready = title.trim() !== "" && !busy && !!project;
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    const created = await useTasksStore
      .getState()
      .create(projectId, { title: title.trim(), body, labels, assignees: [] });
    setBusy(false);
    if ("error" in created) return setError(created.error);
    useTasksStore.getState().select(rowKey(projectId, created.key));
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center overflow-y-auto bg-black/50 py-[10vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={submit}
        className="w-[36rem] max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
      >
        <h2 id={titleId} className="text-base font-semibold">
          New task
        </h2>
        <label className="mt-4 block text-[12px] text-ink-muted">
          Project
          <select
            value={projectId}
            disabled={busy}
            onChange={(event) => {
              setProjectId(event.target.value);
              // Labels are a repository's own.
              setLabels([]);
            }}
            className={`${field} mt-1`}
          >
            {projects.map((it) => (
              <option key={it.id} value={it.id}>
                {it.name}
              </option>
            ))}
          </select>
        </label>
        <label className="mt-3 block text-[12px] text-ink-muted">
          Title
          <input
            ref={titleRef}
            value={title}
            maxLength={256}
            disabled={busy}
            onChange={(event) => setTitle(event.target.value)}
            autoComplete="off"
            className={`${field} mt-1`}
          />
        </label>
        <label className="mt-3 block text-[12px] text-ink-muted">
          Description
          <textarea
            value={body}
            rows={8}
            disabled={busy}
            spellCheck
            placeholder="Markdown, as on GitHub."
            onChange={(event) => setBody(event.target.value)}
            className={`${field} mt-1 resize-y`}
          />
        </label>
        {choices && choices.labels.length > 0 && (
          <fieldset className="mt-3">
            <legend className="text-[12px] text-ink-muted">Labels</legend>
            <div className="mt-1 flex max-h-24 flex-wrap gap-x-3 gap-y-1 overflow-y-auto">
              {choices.labels.map((label) => (
                <label key={label.name} className="flex items-center gap-1.5 text-[12px]">
                  <input
                    type="checkbox"
                    checked={labels.includes(label.name)}
                    disabled={busy}
                    onChange={(event) =>
                      setLabels(
                        event.target.checked
                          ? [...labels, label.name]
                          : labels.filter((name) => name !== label.name),
                      )
                    }
                    className="accent-accent"
                  />
                  {label.name}
                </label>
              ))}
            </div>
          </fieldset>
        )}
        <p className="mt-3 text-[12px] text-ink-faint">
          This opens an issue on {project ? <b>{project.name}</b> : "the project"}&rsquo;s
          repository on GitHub. It is there for everyone who can see the repository as soon as you
          press Create, and the people watching it are told.
        </p>
        {error && (
          <p role="alert" className="mt-2 break-words text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={onClose}
            className="px-3 py-1.5 text-ink-muted hover:text-ink disabled:opacity-40"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={!ready}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Creating…" : "Create"}
          </button>
        </div>
      </form>
    </div>
  );
}
