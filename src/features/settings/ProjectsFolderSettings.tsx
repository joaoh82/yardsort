import { useEffect, useId, useState } from "react";
import { errorMessage, ipc, type ProjectsDir } from "@/lib/ipc";

/**
 * Where a repository goes when a device let in to this computer has it cloned here: a folder of
 * this computer's choosing, never one the device names. The clone is made with this computer's
 * own Git sign-in and added as a project.
 */
export function ProjectsFolderSettings() {
  const [dir, setDir] = useState<ProjectsDir | null>(null);
  const [text, setText] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const fieldId = useId();

  useEffect(() => {
    let stale = false;
    ipc.remoteProjectsDir().then(
      (read) => {
        if (stale) return;
        setDir(read);
        setText(read.set ?? "");
      },
      () => {},
    );
    return () => {
      stale = true;
    };
  }, []);

  if (!dir) return null;
  const unchanged = text.trim() === (dir.set ?? "");
  const save = async (value: string | null) => {
    setProblem(null);
    try {
      const saved = await ipc.remoteSetProjectsDir(value);
      setDir(saved);
      setText(saved.set ?? "");
    } catch (error) {
      setProblem(errorMessage(error));
    }
  };

  return (
    <section aria-label="Projects cloned for a device" className="grid gap-2">
      <h4 className="text-ink-muted">Projects cloned for a device</h4>
      <p className="text-ink-faint">
        A computer you let in with typing can have this one clone a repository and add it as a
        project, to start work in it. It is cloned with this computer&rsquo;s own Git sign-in, into
        this folder:{" "}
        <span className="font-mono text-ink-muted select-text">
          {dir.used ?? "none, since this computer has no home folder"}
        </span>
        {dir.set === null && dir.used !== null && " (the default)"}. The folder is made when the
        first clone needs it.
      </p>
      <form
        className="flex max-w-xl flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void save(text.trim() || null);
        }}
      >
        <label htmlFor={fieldId} className="shrink-0 text-ink-muted">
          Folder
        </label>
        <input
          id={fieldId}
          value={text}
          placeholder="~/Projects"
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => setText(event.target.value)}
          className="h-8 min-w-0 flex-1 rounded border border-line bg-canvas px-2 font-mono outline-none select-text focus:border-accent"
        />
        <button
          type="submit"
          disabled={unchanged}
          className="h-8 rounded border border-line px-3 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
        >
          Save
        </button>
        {dir.set !== null && (
          <button
            type="button"
            onClick={() => void save(null)}
            className="h-8 px-2 text-ink-faint hover:text-ink"
          >
            Use the default
          </button>
        )}
      </form>
      {problem && (
        <p role="alert" className="text-red-400 select-text">
          {problem}
        </p>
      )}
    </section>
  );
}
