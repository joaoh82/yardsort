import { useEffect, useId, useMemo, useRef, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { errorMessage, ipc, type Input, type RunPreview, type Workspace } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useWorkflowStore } from "@/stores/workflows";

const control =
  "h-8 w-full rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";
const label = "text-[11px] font-semibold tracking-wider text-ink-faint uppercase";

interface Props {
  /** Run this workflow; without it, the dialog asks which. */
  workflowId?: string;
  /** In this workspace; without it, the dialog asks which. */
  workspace?: Workspace;
  onClose: () => void;
  /** The run was queued: its id, and the workflow's. */
  onStarted?: (runId: string, workflowId: string) => void;
}

/**
 * Start a run: which workflow, in which workspace, and the answers to its inputs. Everything
 * is checked again by the core when it is queued, exactly as `ys workflow run` does; what shows
 * here first is so the person can see what the run will find before asking for it.
 */
export function RunWorkflowDialog({ workflowId, workspace, onClose, onStarted }: Props) {
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);
  const items = useWorkflowStore((s) => s.items);
  const projects = useProjectsStore((s) => s.projects);
  const allHarnesses = useHarnessStore((s) => s.harnesses);
  const harnesses = allHarnesses.filter((h) => h.enabled);

  const [chosenWorkflow, setChosenWorkflow] = useState(workflowId ?? "");
  const [chosenWorkspace, setChosenWorkspace] = useState(workspace?.id ?? "");
  // What the person chose, per workflow; an input they have not touched shows its default.
  const [chosen, setChosen] = useState<Record<string, Record<string, string>>>({});
  // What the core said a run would find, and for which workflow and workspace it said it.
  const [previewed, setPreviewed] = useState<{ key: string; preview: RunPreview } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void useHarnessStore.getState().load();
    if (!useWorkflowStore.getState().loaded) void useWorkflowStore.getState().load();
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, busy]);

  const runnable = items.filter((item) => item.problems.length === 0 && item.workflow);
  const workflow = items.find((item) => item.id === chosenWorkflow)?.workflow ?? null;
  const inputs: Input[] = useMemo(() => workflow?.inputs ?? [], [workflow]);
  // Every input starts from its default. A required agent starts on the first one installed,
  // and a required choice on its first option: what the control shows must be the answer.
  const firstAgent = harnesses.find((h) => h.resolvedPath)?.id ?? "";
  const startsOn = (input: Input): string => {
    if (input.default) return input.default;
    if (!input.required) return "";
    if (input.kind === "harness") return firstAgent;
    if (input.kind === "choice") return input.options[0] ?? "";
    return "";
  };
  const answers: Record<string, string> = Object.fromEntries(
    inputs.map((input) => [input.id, chosen[chosenWorkflow]?.[input.id] ?? startsOn(input)]),
  );
  const answer = (id: string, value: string) =>
    setChosen((all) => ({ ...all, [chosenWorkflow]: { ...all[chosenWorkflow], [id]: value } }));

  const previewKey = `${chosenWorkflow}|${chosenWorkspace}`;
  const preview = previewed?.key === previewKey ? previewed.preview : null;
  useEffect(() => {
    if (!chosenWorkflow || !chosenWorkspace || !workflow) return;
    let current = true;
    ipc.workflowPreview(chosenWorkflow, chosenWorkspace).then(
      (found) => current && setPreviewed({ key: previewKey, preview: found }),
      (failed) => current && setError(errorMessage(failed)),
    );
    return () => {
      current = false;
    };
  }, [chosenWorkflow, chosenWorkspace, workflow, previewKey]);

  const workspaces = projects.flatMap((project) =>
    project.workspaces
      .filter((w) => !w.archived && !w.missing)
      .map((w) => ({ id: w.id, name: `${project.name} / ${w.name}` })),
  );
  const missing = inputs.filter((input) => input.required && !answers[input.id]?.trim());
  const blockedByPullRequest = preview?.needsPullRequest && !preview.pullRequest;
  const ready =
    !!workflow &&
    !!chosenWorkspace &&
    preview !== null &&
    missing.length === 0 &&
    !blockedByPullRequest &&
    !busy;

  async function start(event: React.FormEvent) {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      const given = Object.fromEntries(Object.entries(answers).filter(([, v]) => v.trim()));
      const run = await ipc.workflowStart(chosenWorkflow, chosenWorkspace, given);
      onStarted?.(run, chosenWorkflow);
      onClose();
    } catch (failed) {
      setError(errorMessage(failed));
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[12vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={start}
        className="flex max-h-[76vh] w-[30rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            {workflow ? `Run “${workflow.name}”` : "Run a workflow"}
          </h2>
          {workflow?.description && <p className="mt-1 text-ink-muted">{workflow.description}</p>}
        </header>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5 pt-4">
          {workflowId === undefined && (
            <label className="flex flex-col gap-1.5">
              <span className={label}>Workflow</span>
              <select
                aria-label="Workflow"
                className={control}
                value={chosenWorkflow}
                onChange={(event) => setChosenWorkflow(event.target.value)}
              >
                <option value="">Choose…</option>
                {runnable.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {workspace === undefined && (
            <label className="flex flex-col gap-1.5">
              <span className={label}>Workspace</span>
              <select
                aria-label="Workspace"
                className={control}
                value={chosenWorkspace}
                onChange={(event) => setChosenWorkspace(event.target.value)}
              >
                <option value="">Choose…</option>
                {workspaces.map((w) => (
                  <option key={w.id} value={w.id}>
                    {w.name}
                  </option>
                ))}
              </select>
            </label>
          )}

          {inputs.map((input) => (
            <label key={input.id} className="flex flex-col gap-1.5">
              <span className={label}>
                {input.label}
                {input.required ? "" : " (optional)"}
              </span>
              {input.kind === "harness" ? (
                <span className="relative flex items-center">
                  <HarnessIcon
                    id={answers[input.id] ?? ""}
                    className="pointer-events-none absolute left-2"
                  />
                  <select
                    aria-label={input.label}
                    className={`${control} pl-7`}
                    value={answers[input.id] ?? ""}
                    onChange={(event) => answer(input.id, event.target.value)}
                  >
                    {!input.required && <option value="">None</option>}
                    {harnesses.map((h) => (
                      <option key={h.id} value={h.id} disabled={!h.resolvedPath}>
                        {h.label}
                        {h.resolvedPath ? "" : " — not installed"}
                      </option>
                    ))}
                  </select>
                </span>
              ) : input.kind === "choice" ? (
                <select
                  aria-label={input.label}
                  className={control}
                  value={answers[input.id] ?? ""}
                  onChange={(event) => answer(input.id, event.target.value)}
                >
                  {!input.required && <option value="">None</option>}
                  {input.options.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  aria-label={input.label}
                  className="rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
                  value={answers[input.id] ?? ""}
                  onChange={(event) => answer(input.id, event.target.value)}
                />
              )}
            </label>
          ))}

          {preview?.needsPullRequest &&
            (preview.pullRequest ? (
              <p className="rounded border border-line bg-canvas/40 p-2.5 text-ink-muted">
                On pull request{" "}
                <span className="text-ink">
                  #{preview.pullRequest.number} {preview.pullRequest.title}
                </span>
              </p>
            ) : (
              <p role="alert" className="text-red-400 select-text">
                {preview.pullRequestProblem}
              </p>
            ))}
          {preview && preview.empty.length > 0 && (
            <ul aria-label="Will be empty" className="flex flex-col gap-1 text-amber-300">
              {preview.empty.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          )}
          {preview?.needsOrigin && (
            <p className="text-ink-faint">
              When it is time, it tells the agent already running in this workspace, if there is
              one.
            </p>
          )}
          {error && (
            <p role="alert" className="text-red-400 select-text">
              {error}
            </p>
          )}
        </div>

        <footer className="flex justify-end gap-2 border-t border-line p-3">
          <button
            type="button"
            className="px-3 py-1.5 text-ink-muted hover:text-ink"
            onClick={onClose}
            disabled={busy}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
            disabled={!ready}
          >
            {busy ? "Starting…" : "Run"}
          </button>
        </footer>
      </form>
    </div>
  );
}
