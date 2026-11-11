import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  errorMessage,
  ipc,
  type Input,
  type RunPreview,
  type TestItem,
  type TrackedPullRequest,
  type WorkflowItem,
  type Workspace,
} from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { useWorkflowStore } from "@/stores/workflows";
import { InputField } from "./InputField";

const control =
  "h-9 w-full rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";
const label = "text-small font-semibold tracking-wider text-ink-faint uppercase";

interface Props {
  /** Run this workflow; without it, the dialog asks which. */
  workflowId?: string;
  /** In this workspace; without it, the dialog asks which. */
  workspace?: Workspace;
  /**
   * On this pull request, which the workspace tracks in another repository: only the workflows
   * about one (`context: tracked_pull_request`) are offered, and the run is about it.
   */
  tracked?: TrackedPullRequest;
  onClose: () => void;
  /** The run was queued: its id, and the workflow's. */
  onStarted?: (runId: string, workflowId: string) => void;
}

/**
 * Start a run: which workflow, in which workspace, and the answers to its inputs. Everything
 * is checked again by the core when it is queued, exactly as `ys workflow run` does; what shows
 * here first is so the person can see what the run will find before asking for it.
 */
export function RunWorkflowDialog({
  workflowId,
  workspace,
  tracked: trackedGiven,
  onClose,
  onStarted,
}: Props) {
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);
  const items = useWorkflowStore((s) => s.items);
  const projects = useProjectsStore((s) => s.projects);
  const allHarnesses = useHarnessStore((s) => s.harnesses);
  const pullRequestsByProject = usePublishStore((s) => s.byProject);
  const harnesses = allHarnesses.filter((h) => h.enabled);

  const [chosenWorkflow, setChosenWorkflow] = useState(workflowId ?? "");
  const [chosenWorkspace, setChosenWorkspace] = useState(workspace?.id ?? "");
  // The pull request elsewhere the run is about: the one it was opened from, or one the person
  // picked when the workspace had no open pull request of its own.
  const [tracked, setTracked] = useState(trackedGiven);
  // When the person picked one, the workflow they had chosen first, to go back to.
  const [switchedFrom, setSwitchedFrom] = useState<string | null>(null);
  // What the person chose, per workflow; an input they have not touched shows its default.
  const [chosen, setChosen] = useState<Record<string, Record<string, string>>>({});
  // What the core said a run would find, and for which workflow and workspace it said it.
  const [previewed, setPreviewed] = useState<{ key: string; preview: RunPreview } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The chosen workspace's smoke tests, for a `test` input to pick from, and which workspace the
  // core listed them for: switching workspace leaves the old list stale until the new one lands.
  const [tests, setTests] = useState<{ workspace: string; items: TestItem[] } | null>(null);

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

  // A workflow about a pull request elsewhere runs on one, from it; every other, in a workspace.
  const context = tracked ? "tracked_pull_request" : "workspace";
  const runnable = items.filter(
    (item) =>
      item.problems.length === 0 && item.workflow && item.workflow.trigger.context === context,
  );
  const workflow = items.find((item) => item.id === chosenWorkflow)?.workflow ?? null;
  const inputs: Input[] = useMemo(() => workflow?.inputs ?? [], [workflow]);
  // Every input starts from its default. A required agent starts on the first one installed,
  // and a required choice on its first option: what the control shows must be the answer.
  const firstAgent = harnesses.find((h) => h.resolvedPath)?.id ?? "";
  // A `test` input picks from the chosen workspace's smoke tests; load them when one is needed.
  const needsTest = inputs.some((input) => input.kind === "test");
  // Only the tests the core listed for the workspace now chosen count. Until that load lands
  // (`testsLoaded` is false) there are none to offer and Run waits; a list left over from another
  // workspace never leaks in.
  const testsHere = tests?.workspace === chosenWorkspace ? tests.items : null;
  const testsLoaded = testsHere !== null;
  const runnableTests = (testsHere ?? []).filter((test) => test.runnable && !test.browserMissing);
  const firstTest = runnableTests[0]?.id ?? "";
  const startsOn = (input: Input): string => {
    if (input.default) return input.default;
    if (!input.required) return "";
    if (input.kind === "harness") return firstAgent;
    if (input.kind === "choice") return input.options[0] ?? "";
    return "";
  };
  // A `test` answer must be a runnable test in the chosen workspace: a pick left over from another
  // workspace, or a default that is not among them, falls back to the first runnable one (or "").
  // So what Run submits is always exactly what the select shows.
  const answerFor = (input: Input): string => {
    const stored = chosen[chosenWorkflow]?.[input.id];
    if (input.kind === "test") {
      const pick = stored ?? input.default ?? "";
      return pick && runnableTests.some((test) => test.id === pick) ? pick : firstTest;
    }
    return stored ?? startsOn(input);
  };
  const answers: Record<string, string> = Object.fromEntries(
    inputs.map((input) => [input.id, answerFor(input)]),
  );
  const answer = (id: string, value: string) =>
    setChosen((all) => ({ ...all, [chosenWorkflow]: { ...all[chosenWorkflow], [id]: value } }));

  const trackedId = tracked?.id ?? null;
  const previewKey = `${chosenWorkflow}|${chosenWorkspace}|${trackedId ?? ""}`;
  const preview = previewed?.key === previewKey ? previewed.preview : null;
  useEffect(() => {
    if (!chosenWorkflow || !chosenWorkspace || !workflow) return;
    let current = true;
    ipc.workflowPreview(chosenWorkflow, chosenWorkspace, trackedId).then(
      (found) => current && setPreviewed({ key: previewKey, preview: found }),
      (failed) => current && setError(errorMessage(failed)),
    );
    return () => {
      current = false;
    };
  }, [chosenWorkflow, chosenWorkspace, trackedId, workflow, previewKey]);

  useEffect(() => {
    if (!needsTest || !chosenWorkspace) return;
    let current = true;
    ipc.testList(chosenWorkspace).then(
      (found) => current && setTests({ workspace: chosenWorkspace, items: found }),
      () => current && setTests({ workspace: chosenWorkspace, items: [] }),
    );
    return () => {
      current = false;
    };
  }, [needsTest, chosenWorkspace]);

  const workspaces = projects.flatMap((project) =>
    project.workspaces
      .filter((w) => !w.archived && !w.missing)
      .map((w) => ({ id: w.id, name: `${project.name} / ${w.name}` })),
  );
  const missing = inputs.filter((input) => input.required && !answers[input.id]?.trim());
  const blockedByPullRequest = preview?.needsPullRequest && !preview.pullRequest;

  // No open pull request of its own, but open ones the workspace tracks elsewhere — often the
  // whole of its work, in a parent project. The workflow's `-elsewhere` twin can run on one.
  const projectId = projects.find((p) => p.workspaces.some((w) => w.id === chosenWorkspace))?.id;
  const openElsewhere =
    !tracked && blockedByPullRequest && projectId
      ? (pullRequestsByProject[projectId]?.tracked ?? []).filter(
          (t) => t.workspaceId === chosenWorkspace && t.pullRequest?.state === "open",
        )
      : [];
  const twin = runnableElsewhere(items, chosenWorkflow);
  const runOn = (pick: TrackedPullRequest) => {
    if (!twin) return;
    // What was answered carries over: the twin asks the same things.
    setChosen((all) => ({ ...all, [twin.id]: { ...answers, ...all[twin.id] } }));
    setSwitchedFrom(chosenWorkflow);
    setTracked(pick);
    setChosenWorkflow(twin.id);
  };
  // Undo a pick: back to the workflow chosen first, and its list of pull requests elsewhere.
  const unpick = () => {
    if (switchedFrom === null) return;
    setChosen((all) => ({ ...all, [switchedFrom]: { ...all[switchedFrom], ...answers } }));
    setTracked(undefined);
    setChosenWorkflow(switchedFrom);
    setSwitchedFrom(null);
  };
  const ready =
    !!workflow &&
    !!chosenWorkspace &&
    preview !== null &&
    missing.length === 0 &&
    !blockedByPullRequest &&
    (!needsTest || testsLoaded) &&
    !busy;

  async function start(event: React.FormEvent) {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      const given = Object.fromEntries(Object.entries(answers).filter(([, v]) => v.trim()));
      const run = await ipc.workflowStart(chosenWorkflow, chosenWorkspace, given, trackedId);
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
                onChange={(event) => {
                  // A pull request picked elsewhere is the old workspace's: let it go.
                  unpick();
                  setChosenWorkspace(event.target.value);
                }}
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
            <InputField
              key={input.id}
              input={input}
              value={answers[input.id] ?? ""}
              onChange={(value) => answer(input.id, value)}
              tests={testsLoaded ? runnableTests : null}
              hasWorkspace={!!chosenWorkspace}
            />
          ))}

          {preview?.needsPullRequest &&
            (preview.pullRequest ? (
              <p className="rounded border border-line bg-canvas/40 p-2.5 text-ink-muted">
                On pull request{" "}
                <span className="text-ink">
                  {preview.pullRequest.elsewhere ? preview.pullRequest.elsewhere.repo : ""}#
                  {preview.pullRequest.number} {preview.pullRequest.title}
                </span>
              </p>
            ) : (
              <div className="flex flex-col gap-2">
                <p role="alert" className="text-red-400 select-text">
                  {preview.pullRequestProblem}
                </p>
                {openElsewhere.length > 0 &&
                  (twin ? (
                    <div className="flex flex-col gap-1.5">
                      <p className="text-ink-muted">
                        This workspace tracks open pull requests elsewhere:
                      </p>
                      <ul aria-label="Open elsewhere" className="flex flex-col gap-1">
                        {openElsewhere.map((t) => (
                          <li key={t.id}>
                            <button
                              type="button"
                              className="w-full truncate rounded border border-line bg-canvas/40 px-2.5 py-1.5 text-left text-ink hover:border-accent"
                              onClick={() => runOn(t)}
                            >
                              {t.repo.owner}/{t.repo.name}#{t.number} {t.pullRequest?.title}
                            </button>
                          </li>
                        ))}
                      </ul>
                      <p className="text-ink-faint">Pick one to run “{twin.name}” on it instead.</p>
                    </div>
                  ) : (
                    <p className="text-ink-muted">
                      This workspace tracks open pull requests elsewhere (
                      {openElsewhere
                        .map((t) => `${t.repo.owner}/${t.repo.name}#${t.number}`)
                        .join(", ")}
                      ), but this workflow runs only on the workspace&apos;s own. Run one about a
                      pull request elsewhere from it, in the Pull requests view.
                    </p>
                  ))}
              </div>
            ))}
          {switchedFrom !== null && (
            <button
              type="button"
              className="self-start text-accent hover:underline"
              onClick={unpick}
            >
              Choose another pull request
            </button>
          )}
          {preview && preview.empty.length > 0 && (
            <ul aria-label="Will be empty" className="flex flex-col gap-1 text-amber-300">
              {preview.empty.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          )}
          {preview?.needsOrigin && (
            <p className="text-ink-faint">
              {tracked
                ? "When it is time, it tells the agent that opened the pull request, if it is known, or the one running in this workspace."
                : "When it is time, it tells the agent already running in this workspace, if there is one."}
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

/**
 * The workflow to run instead on a pull request elsewhere: `<id>-elsewhere`, when there is one
 * about such a pull request that can run — the built-ins come in such pairs, and a copy keeps
 * its built-in's id.
 */
function runnableElsewhere(items: WorkflowItem[], id: string): WorkflowItem | undefined {
  if (!id) return undefined;
  return items.find(
    (item) =>
      item.id === `${id}-elsewhere` &&
      item.problems.length === 0 &&
      item.workflow?.trigger.context === "tracked_pull_request",
  );
}
