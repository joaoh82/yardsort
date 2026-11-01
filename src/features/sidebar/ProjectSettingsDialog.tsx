import { useEffect, useId, useRef, useState } from "react";
import {
  errorMessage,
  ipc,
  type Project,
  type ProjectAutomation,
  type TestEnvEntry,
  type TestSettings,
} from "@/lib/ipc";
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
  // Smoke tests: the test environment and what the project asks of its runs.
  const [env, setEnv] = useState<TestEnvEntry[] | null>(null);
  const [tests, setTests] = useState<TestSettings | null>(null);
  const [showValues, setShowValues] = useState(false);
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
    void Promise.all([ipc.testEnvGet(project.id), ipc.testSettingsGet(project.id)])
      .then(([entries, settings]) => {
        if (cancelled) return;
        setEnv(entries);
        setTests(settings);
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
  const args = (text: string) => text.split("\n").filter((line) => line.trim() !== "");
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
      if (env && tests) {
        await ipc.testEnvSave(
          project.id,
          env
            .map((entry) => ({ name: entry.name.trim(), value: entry.value }))
            .filter((entry) => entry.name !== ""),
        );
        await ipc.testSettingsSave(project.id, tests);
      }
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
            One argument per line, without shell quotes. Blank lines are ignored. For a script,
            enter its interpreter (such as bash or pwsh) and its path as an argument. Leave an
            executable blank to disable it. Setup has a 10-minute limit and writes a log in the
            worktree’s private Git directory.
          </p>
        </fieldset>
        <fieldset disabled={!env || !tests || busy} className="mt-5 space-y-3 disabled:opacity-50">
          <legend className="font-semibold">Smoke tests</legend>
          <p className="text-ink-muted">
            The test environment: what a smoke test reads as <code>{"{{ env.NAME }}"}</code>, kept
            here and never in the repository. A test that uses a name not set here gets an empty
            string, and the approval card says so.
          </p>
          <div className="space-y-1.5">
            {(env ?? []).map((entry, index) => (
              <div key={index} className="flex items-center gap-2">
                <input
                  aria-label={`Variable ${index + 1} name`}
                  value={entry.name}
                  onChange={(e) =>
                    setEnv((all) =>
                      (all ?? []).map((it, i) =>
                        i === index ? { ...it, name: e.target.value } : it,
                      ),
                    )
                  }
                  placeholder="TEST_PASSWORD"
                  className={`${input} mt-0 w-56`}
                />
                <input
                  aria-label={`Variable ${index + 1} value`}
                  type={showValues ? "text" : "password"}
                  value={entry.value}
                  onChange={(e) =>
                    setEnv((all) =>
                      (all ?? []).map((it, i) =>
                        i === index ? { ...it, value: e.target.value } : it,
                      ),
                    )
                  }
                  className={`${input} mt-0 flex-1`}
                />
                <button
                  type="button"
                  aria-label={`Remove variable ${index + 1}`}
                  onClick={() => setEnv((all) => (all ?? []).filter((_, i) => i !== index))}
                  className="shrink-0 rounded px-1.5 text-ink-faint hover:bg-raised hover:text-ink"
                >
                  ×
                </button>
              </div>
            ))}
            <div className="flex gap-3">
              <button
                type="button"
                onClick={() => setEnv((all) => [...(all ?? []), { name: "", value: "" }])}
                className="text-ink-muted hover:text-ink"
              >
                + Add a variable
              </button>
              {(env?.length ?? 0) > 0 && (
                <button
                  type="button"
                  aria-pressed={showValues}
                  onClick={() => setShowValues((shown) => !shown)}
                  className="text-ink-muted hover:text-ink"
                >
                  {showValues ? "Hide values" : "Show values"}
                </button>
              )}
            </div>
          </div>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={tests?.askEveryTime ?? false}
              onChange={(e) =>
                setTests((it) => (it ? { ...it, askEveryTime: e.target.checked } : it))
              }
            />
            Ask every time
          </label>
          <p className="text-ink-faint">
            Show every run’s steps and ask before it runs, even for a file you approved before. Off,
            a file you have approved runs straight away until it changes.
          </p>
          <label className="flex items-center gap-2">
            Keep runs for
            <input
              type="number"
              min={1}
              max={3650}
              aria-label="Keep runs for days"
              value={tests?.keepDays || ""}
              onChange={(e) =>
                setTests((it) => (it ? { ...it, keepDays: Number(e.target.value) || 0 } : it))
              }
              className={`${input} mt-0 w-20`}
            />
            days
          </label>
          <p className="text-ink-faint">
            Older runs are removed with their output files. Runs waiting, queued or going are never
            removed.
          </p>
          <label className="flex flex-col gap-1">
            Browser
            <input
              type="text"
              aria-label="Browser"
              placeholder="Found automatically — Chrome, Chromium or Edge"
              value={tests?.browser ?? ""}
              onChange={(e) => setTests((it) => (it ? { ...it, browser: e.target.value } : it))}
              className={`${input} mt-0`}
            />
          </label>
          <p className="text-ink-faint">
            The browser a <code>browser</code> step drives. Left empty, one is found on this
            machine. Set the full path to a Chromium-family browser to use that one instead.
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
