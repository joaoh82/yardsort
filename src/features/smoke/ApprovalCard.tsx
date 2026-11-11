import { useEffect, useId, useRef, useState } from "react";
import { timeAgo } from "@/features/terminal/sessionText";
import { errorMessage, ipc, type TestReview, type TestRun } from "@/lib/ipc";
import { formatShortcut, isModKey } from "@/lib/platform";
import { useSmokeTestsStore, waitingIn } from "@/stores/smokeTests";
import { diffLines } from "./diff";
import { PHASE_WORDS, PHASES, requestedByWords } from "./words";

/**
 * A run waiting for the user, at the top of its workspace: the file's steps in words, the
 * environment names it uses and which are not set, what changed since the version approved
 * before, and **Run** or **Don't run**. This is the gate a file from a repository goes through
 * before it runs anything, so the words are the whole of what the file does.
 *
 * One at a time, oldest first; the next waits behind it and the card says how many.
 */
export function ApprovalCard({ workspaceId }: { workspaceId: string }) {
  const runs = useSmokeTestsStore((s) => s.runs);
  const waiting = waitingIn(runs, workspaceId);
  const run = waiting[0];
  if (!run) return null;
  return <Card key={run.id} run={run} behind={waiting.length - 1} />;
}

function Card({ run, behind }: { run: TestRun; behind: number }) {
  const titleId = useId();
  const ref = useRef<HTMLElement>(null);
  const [review, setReview] = useState<TestReview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const approve = useSmokeTestsStore((s) => s.approve);
  const refuse = useSmokeTestsStore((s) => s.refuse);
  const openRun = useSmokeTestsStore((s) => s.openRun);
  const runKey = formatShortcut("Enter");

  useEffect(() => {
    let current = true;
    ipc.testRunReview(run.id).then(
      (found) => current && setReview(found),
      (failed) => current && setError(errorMessage(failed)),
    );
    return () => {
      current = false;
    };
  }, [run.id]);

  const decide = async (yes: boolean) => {
    if (busy) return;
    setBusy(true);
    const done = yes ? await approve(run.id) : await refuse(run.id);
    // A run the user let go is the one they want to watch: its tab opens on the answer.
    if (done && yes && run.workspaceId) openRun(run.workspaceId, run.id);
    setBusy(false);
  };

  // Mod+Enter runs and Escape refuses, as in the composer — while the card has the keyboard.
  // It never takes it: an Escape meant for the agent behind must not refuse a run.
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Enter" && isModKey(event.nativeEvent)) {
      event.preventDefault();
      void decide(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      void decide(false);
    }
  };

  const inputs = Object.entries(run.inputs);
  const button =
    "rounded border border-line px-3 py-1.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40";
  return (
    <section
      ref={ref}
      tabIndex={-1}
      role="region"
      aria-labelledby={titleId}
      onKeyDown={onKeyDown}
      className="flex max-h-[50%] shrink-0 flex-col border-b border-line bg-raised outline-none focus-visible:ring-1 focus-visible:ring-accent"
    >
      <div className="flex items-start gap-3 px-4 pt-3 pb-2">
        <div className="min-w-0 flex-1">
          <h3 id={titleId} className="font-semibold">
            Run “{run.testName}”?
          </h3>
          <p className="text-ink-muted">
            Asked by {requestedByWords(run)} {timeAgo(run.createdAt)}
            {inputs.length > 0 && (
              <>
                , with{" "}
                {inputs.map(([id, value], index) => (
                  <span key={id}>
                    {index > 0 && ", "}
                    <span className="font-mono text-small">{id}</span> = {value}
                  </span>
                ))}
              </>
            )}
            . A file from the repository runs only once you have seen it; this is what it does.
            {behind > 0 && ` ${behind} more ${behind === 1 ? "run waits" : "runs wait"} behind it.`}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            disabled={busy || !review}
            onClick={() => void decide(true)}
            title={`Approve this file and run it (${runKey})`}
            className="rounded bg-accent px-4 py-1 font-medium text-canvas disabled:opacity-40"
          >
            Run
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void decide(false)}
            title="Do not run it; nothing is recorded against the file (Esc)"
            className={button}
          >
            Don’t run
          </button>
        </div>
      </div>
      <div className="min-h-0 overflow-y-auto px-4 pb-3">
        {error && (
          <p role="alert" className="text-red-400 select-text">
            {error}
          </p>
        )}
        {review && <Steps review={review} />}
        {review && <EnvUses review={review} className="mt-2" />}
        {review?.previous && <Changed before={review.previous} after={run.definition} />}
        {review?.approved && (
          <p className="mt-2 text-ink-faint">
            This exact file was approved before; the project asks to see every run.
          </p>
        )}
      </div>
    </section>
  );
}

/** The steps, phase by phase, each as the core puts it in words. */
export function Steps({ review }: { review: TestReview }) {
  return (
    <div className="flex flex-col gap-1.5">
      {PHASES.map((phase) => {
        const steps = review.steps.filter((step) => step.phase === phase);
        if (steps.length === 0) return null;
        return (
          <div key={phase}>
            <h4 className="text-small font-semibold tracking-wider text-ink-faint uppercase">
              {PHASE_WORDS[phase]}
            </h4>
            <ol aria-label={`${PHASE_WORDS[phase]} steps`} className="mt-0.5 flex flex-col gap-0.5">
              {steps.map((step) => (
                <li key={step.id} className="flex gap-2 select-text">
                  <span className="shrink-0 font-mono text-small leading-5 text-ink-faint">
                    {step.id}
                  </span>
                  <span className="min-w-0 break-words">{step.words}</span>
                </li>
              ))}
            </ol>
          </div>
        );
      })}
    </div>
  );
}

/**
 * What of the project's test environment the file can read. A step that runs a program is
 * handed the whole environment as `YS_ENV_*`, not only the names the file templates — so when
 * there is such a step, the card has to name all of it, or it understates the exposure.
 */
export function EnvUses({ review, className = "" }: { review: TestReview; className?: string }) {
  const unset = review.env.filter((use) => !use.set);
  if (review.runsAProgram && review.environment.length > 0) {
    return (
      <p className={`text-ink-muted ${className}`}>
        A step runs a program, so it can read the project’s whole test environment:{" "}
        {review.environment.map((name, index) => (
          <span key={name}>
            {index > 0 && ", "}
            <span className="font-mono text-small">{name}</span>
          </span>
        ))}{" "}
        (as <span className="font-mono text-small">YS_ENV_*</span>)
        {unset.length > 0 &&
          `. ${unset.map((use) => use.name).join(", ")} ${
            unset.length === 1 ? "is named but" : "are named but"
          } not set`}
        .
      </p>
    );
  }
  if (review.env.length === 0) return null;
  return (
    <p className={`text-ink-muted ${className}`}>
      Uses{" "}
      {review.env.map((use, index) => (
        <span key={use.name}>
          {index > 0 && ", "}
          <span className="font-mono text-small">{use.name}</span>
          {use.set ? "" : " (not set)"}
        </span>
      ))}{" "}
      from the project’s test environment
      {unset.length > 0 &&
        `; ${unset.length === 1 ? "one is" : `${unset.length} are`} not set and would be empty`}
      .
    </p>
  );
}

/** What changed since the version approved before, line by line. */
export function Changed({ before, after }: { before: string; after: string }) {
  const lines = diffLines(before, after);
  return (
    <details open className="mt-2">
      <summary className="cursor-pointer text-ink-muted">
        What changed since the version you approved
      </summary>
      <pre
        aria-label="What changed"
        className="mt-1 max-h-60 overflow-auto rounded border border-line bg-canvas p-2 font-mono text-small leading-4 select-text"
      >
        {lines.map((line, index) => (
          <span
            key={index}
            className={`block ${
              line.kind === "added"
                ? "bg-green-500/15 text-green-300"
                : line.kind === "removed"
                  ? "bg-red-500/15 text-red-300 line-through"
                  : "text-ink-faint"
            }`}
          >
            {line.kind === "added" ? "+ " : line.kind === "removed" ? "- " : "  "}
            {line.text}
          </span>
        ))}
      </pre>
    </details>
  );
}
