import { useEffect, useId, useRef, useState } from "react";
import { timeAgo } from "@/features/terminal/sessionText";
import { errorMessage, ipc, type TestItem, type TestReview } from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { Changed, EnvUses, Steps } from "./ApprovalCard";
import { statusColour, statusWords } from "./words";

const control =
  "h-9 w-full rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";
const label = "text-small font-semibold tracking-wider text-ink-faint uppercase";

interface Props {
  workspaceId: string;
  /** Run this test; without it, the dialog asks which. */
  testId?: string;
  /** Start the inputs from these: a run being run again. */
  inputs?: Record<string, string>;
  onClose: () => void;
}

/**
 * **Run smoke test…**: the worktree's tests with their readiness, the chosen one's inputs,
 * its steps as the approval card shows them, and **Run**. Seeing the steps and pressing Run is
 * the approval, so a file not approved yet needs no second look; a file changed since it was
 * approved shows what changed. The workspace's recent runs are listed too, to open any again.
 */
export function RunSmokeTestDialog({ workspaceId, testId, inputs: given, onClose }: Props) {
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);
  const [tests, setTests] = useState<TestItem[] | null>(null);
  const [chosenId, setChosenId] = useState(testId ?? "");
  const [answers, setAnswers] = useState<Record<string, string>>(given ?? {});
  const [reviewed, setReviewed] = useState<{ id: string; review: TestReview } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const runs = useSmokeTestsStore((s) => s.runs);
  const openRun = useSmokeTestsStore((s) => s.openRun);
  const recent = runs.filter((run) => run.workspaceId === workspaceId).slice(0, 8);

  useEffect(() => {
    let current = true;
    ipc.testList(workspaceId).then(
      (found) => {
        if (!current) return;
        setTests(found);
        // One runnable test and none asked for: it is the one.
        const runnable = found.filter((test) => test.runnable && !test.browserMissing);
        if (!testId && runnable.length === 1 && runnable[0]) setChosenId(runnable[0].id);
      },
      (failed) => current && setError(errorMessage(failed)),
    );
    return () => {
      current = false;
    };
  }, [workspaceId, testId]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, busy]);

  const chosen = tests?.find((test) => test.id === chosenId) ?? null;
  const review = reviewed?.id === chosenId ? reviewed.review : null;
  useEffect(() => {
    if (!chosen || !chosen.runnable) return;
    let current = true;
    ipc.testReview(workspaceId, chosen.id).then(
      (found) => current && setReviewed({ id: chosen.id, review: found }),
      (failed) => current && setError(errorMessage(failed)),
    );
    return () => {
      current = false;
    };
  }, [workspaceId, chosen]);

  const value = (id: string, fallback: string | null) => answers[id] ?? fallback ?? "";
  const missing =
    chosen?.inputs.filter((input) => input.required && !value(input.id, input.default).trim()) ??
    [];
  const ready =
    !!chosen &&
    chosen.runnable &&
    !chosen.browserMissing &&
    !!review &&
    missing.length === 0 &&
    !busy;

  async function start(event: React.FormEvent) {
    event.preventDefault();
    if (!ready || !chosen || !review) return;
    setBusy(true);
    setError(null);
    try {
      const inputs = Object.fromEntries(
        chosen.inputs
          .map((input) => [input.id, value(input.id, input.default).trim()])
          .filter(([, text]) => text),
      );
      // The hash of the file the dialog showed: if it changed since, the run waits for the
      // approval card instead, so what runs is always a version someone saw.
      const started = await ipc.testRunStart(chosen.id, workspaceId, inputs, review.contentHash);
      await useSmokeTestsStore.getState().refresh();
      // When the file is unchanged the run is going, so open its tab. When it changed, it waits
      // for approval; the card at the top of the workspace shows the new version — leave it there.
      if (!started.awaitingApproval) openRun(workspaceId, started.runId);
      onClose();
    } catch (failed) {
      setError(errorMessage(failed));
      setBusy(false);
    }
  }

  const readiness = (test: TestItem) =>
    !test.runnable
      ? `${test.problems.length} problem${test.problems.length === 1 ? "" : "s"}`
      : test.browserMissing
        ? "needs a browser"
        : "ready";

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[8vh]"
      onPointerDown={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <form
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onSubmit={start}
        className="flex max-h-[84vh] w-[36rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            {chosen ? `Run “${chosen.name}”` : "Run a smoke test"}
          </h2>
          {chosen?.description && <p className="mt-1 text-ink-muted">{chosen.description}</p>}
        </header>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5 pt-4">
          {tests && tests.length === 0 && (
            <p className="text-ink-muted">
              This workspace has no smoke tests: nothing in its <code>.yardsort/tests</code> folder.
              Ask the agent to write one — <code>ys test guide</code> tells it how.
            </p>
          )}
          {tests && tests.length > 0 && testId === undefined && (
            <label className="flex flex-col gap-1.5">
              <span className={label}>Test</span>
              <select
                aria-label="Test"
                className={control}
                value={chosenId}
                onChange={(event) => {
                  setChosenId(event.target.value);
                  setAnswers({});
                }}
              >
                <option value="">Choose…</option>
                {tests.map((test) => (
                  <option key={test.id} value={test.id}>
                    {test.name} — {readiness(test)}
                  </option>
                ))}
              </select>
            </label>
          )}
          {chosen && !chosen.runnable && (
            <ul aria-label="Problems" className="flex flex-col gap-1 text-red-400 select-text">
              {chosen.problems.map((problem) => (
                <li key={`${problem.line}:${problem.column}:${problem.message}`}>
                  {problem.line ? `${problem.line}:${problem.column ?? 1}: ` : ""}
                  {problem.message}
                </li>
              ))}
            </ul>
          )}
          {chosen && chosen.runnable && chosen.browserMissing && (
            <p role="alert" className="text-amber-300">
              This test has a <code>browser</code> step, and no browser was found on this machine.
              Install Chrome, Chromium or Edge, or name one under the project’s smoke test settings.
            </p>
          )}

          {chosen?.inputs.map((input) => (
            <label key={input.id} className="flex flex-col gap-1.5">
              <span className={label}>
                {input.label}
                {input.required ? "" : " (optional)"}
              </span>
              {input.kind === "choice" ? (
                <select
                  aria-label={input.label}
                  className={control}
                  value={value(input.id, input.default)}
                  onChange={(event) =>
                    setAnswers((all) => ({ ...all, [input.id]: event.target.value }))
                  }
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
                  value={value(input.id, input.default)}
                  onChange={(event) =>
                    setAnswers((all) => ({ ...all, [input.id]: event.target.value }))
                  }
                />
              )}
            </label>
          ))}

          {review && (
            <section aria-label="What it does" className="flex flex-col gap-2">
              <Steps review={review} />
              <EnvUses review={review} />
              {review.previous && chosen && (
                <Changed before={review.previous} after={chosen.text} />
              )}
              <p className="text-ink-faint">
                {review.approved
                  ? "You approved this exact file before."
                  : "Pressing Run approves this file: the next run of it goes straight ahead."}
              </p>
            </section>
          )}

          {recent.length > 0 && (
            <section aria-label="Recent runs">
              <h3 className={label}>Recent runs here</h3>
              <ul className="mt-1 flex flex-col gap-0.5">
                {recent.map((run) => (
                  <li key={run.id}>
                    <button
                      type="button"
                      onClick={() => {
                        openRun(workspaceId, run.id);
                        onClose();
                      }}
                      className="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-raised"
                    >
                      <span
                        className={`shrink-0 rounded-full px-1.5 text-micro ${statusColour(run.status)}`}
                      >
                        {statusWords(run)}
                      </span>
                      <span className="min-w-0 flex-1 truncate">{run.testName}</span>
                      <span className="shrink-0 text-small text-ink-faint">
                        {timeAgo(run.createdAt)}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
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
