import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, ipc, type Project, type ProjectAutomation } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";

export function ProjectSettingsDialog({
  project,
  onClose,
}: {
  project: Project;
  onClose: () => void;
}) {
  const [config, setConfig] = useState<ProjectAutomation | null>(null);
  const [files, setFiles] = useState("");
  const [setup, setSetup] = useState("");
  const [setupArgs, setSetupArgs] = useState("");
  const [run, setRun] = useState("");
  const [runArgs, setRunArgs] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLFormElement>(null);
  const title = useId();
  useModalFocus(ref);
  useEffect(() => {
    let cancelled = false;
    void ipc
      .projectAutomationGet(project.id)
      .then((value) => {
        if (cancelled) return;
        setConfig(value);
        setFiles(value.copyFiles.join("\n"));
        setSetup(value.setup?.program ?? "");
        setSetupArgs(value.setup?.args.join("\n") ?? "");
        setRun(value.run?.program ?? "");
        setRunArgs(value.run?.args.join("\n") ?? "");
      })
      .catch((error) => {
        if (!cancelled) setError(errorMessage(error));
      });
    return () => {
      cancelled = true;
    };
  }, [project.id]);
  const input =
    "mt-1 w-full rounded border border-line bg-canvas px-2 py-1.5 font-mono outline-none select-text focus:border-accent";
  const args = (text: string) => (text === "" ? [] : text.split("\n"));
  async function save(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await ipc.projectAutomationSave(project.id, {
        copyFiles: files
          .split("\n")
          .map((file) => file.trim())
          .filter(Boolean),
        setup: setup.trim() ? { program: setup.trim(), args: args(setupArgs) } : null,
        run: run.trim() ? { program: run.trim(), args: args(runArgs) } : null,
      });
      onClose();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50 p-4">
      <form
        ref={ref}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={title}
        onSubmit={(event) => void save(event)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !busy) onClose();
        }}
        className="max-h-full w-[36rem] overflow-y-auto rounded-lg border border-line bg-surface p-5 shadow-2xl"
      >
        <h2 id={title} className="text-base font-semibold">
          Project settings — {project.name}
        </h2>
        <p className="mt-2 text-ink-muted">
          Copy files, then run setup in each new or restored worktree before its agent starts. Local
          and imported workspaces are left as they are.
        </p>
        <fieldset disabled={!config || busy} className="mt-4 space-y-3 disabled:opacity-50">
          <label className="block">
            Files to copy
            <textarea
              aria-label="Files to copy"
              value={files}
              onChange={(e) => setFiles(e.target.value)}
              placeholder={".env\nconfig/local.json"}
              rows={3}
              className={input}
            />
          </label>
          <p className="text-ink-faint">
            One file per line, relative to the project root. Use / separators. No directories,
            symlinks, or overwriting existing files.
          </p>
          <label className="block">
            Setup executable
            <input
              value={setup}
              onChange={(e) => setSetup(e.target.value)}
              placeholder="bun"
              className={input}
            />
          </label>
          <label className="block">
            Setup arguments
            <textarea
              value={setupArgs}
              onChange={(e) => setSetupArgs(e.target.value)}
              placeholder="install"
              rows={2}
              className={input}
            />
          </label>
          <label className="block">
            Run executable
            <input
              value={run}
              onChange={(e) => setRun(e.target.value)}
              placeholder="bun"
              className={input}
            />
          </label>
          <label className="block">
            Run arguments
            <textarea
              value={runArgs}
              onChange={(e) => setRunArgs(e.target.value)}
              placeholder={"run\ndev"}
              rows={2}
              className={input}
            />
          </label>
          <p className="text-ink-faint">
            One argument per line, without shell quotes. For a script, enter its interpreter (such
            as bash or pwsh) and its path as an argument. Leave an executable blank to disable it.
            Setup has a 10-minute limit and writes a log in the worktree.
          </p>
        </fieldset>
        {error && (
          <p role="alert" className="mt-3 text-red-400 select-text">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={onClose}
            className="px-3 py-1.5 text-ink-muted"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={!config || busy}
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </div>
  );
}
