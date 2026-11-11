import { useEffect, useMemo, useState } from "react";
import { ContextMenu, type MenuItem } from "@/features/sidebar/ContextMenu";
import { StatusDot } from "@/features/terminal/StatusDot";
import { errorMessage, ipc, type TestReview, type TestRun, type TestStepRun } from "@/lib/ipc";
import { native } from "@/lib/native";
import { useSmokeTestsStore, isActive } from "@/stores/smokeTests";
import { useTerminalStore } from "@/stores/terminals";
import { RunSmokeTestDialog } from "./RunSmokeTestDialog";
import { useStepOutput } from "./useStepOutput";
import {
  outputFacts,
  PHASE_WORDS,
  PHASES,
  runActivity,
  statusColour,
  statusWords,
  stepStatusWords,
  tookWords,
} from "./words";

const EMPTY: TestStepRun[] = [];

interface Props {
  run: TestRun;
  workspaceId: string;
}

/**
 * A run, as a tab beside the workspace's terminals: the steps down the left in their three
 * phases, the selected step on the right with its output tailed live, its expectations and
 * which failed, and — for a `manual` step — the question with **Pass**, **Fail** and a note.
 * A view of rows and files: closing it loses nothing.
 */
export function TestRunTab({ run, workspaceId }: Props) {
  const steps = useSmokeTestsStore((s) => s.steps[run.id]) ?? EMPTY;
  const cancel = useSmokeTestsStore((s) => s.cancel);
  const error = useSmokeTestsStore((s) => s.error);
  const dismissError = useSmokeTestsStore((s) => s.dismissError);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [review, setReview] = useState<TestReview | null>(null);
  const [runAgain, setRunAgain] = useState(false);
  const [, tick] = useState(0);

  useEffect(() => {
    void useSmokeTestsStore.getState().loadSteps(run.id);
  }, [run.id]);
  useEffect(() => {
    let current = true;
    ipc.testRunReview(run.id).then(
      (found) => current && setReview(found),
      () => {},
    );
    return () => {
      current = false;
    };
  }, [run.id]);
  // "so far" durations move while the run goes.
  useEffect(() => {
    if (!isActive(run.status)) return;
    const timer = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [run.status]);

  // The step to look at, unless one was picked: the one going, else the first that failed,
  // else the last one that ended.
  const selected = useMemo(() => {
    if (selectedId) return steps.find((step) => step.stepId === selectedId) ?? null;
    return (
      steps.find((step) => step.status === "running" || step.status === "waiting") ??
      steps.find((step) => step.status === "failed") ??
      [...steps].reverse().find((step) => step.endedAt !== null) ??
      steps[0] ??
      null
    );
  }, [steps, selectedId]);
  const words = (stepId: string) => review?.steps.find((step) => step.id === stepId)?.words;
  const took = tookWords(run);

  return (
    <section aria-label={`Smoke test run: ${run.testName}`} className="flex h-full flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-3 py-2">
        <span className={`rounded-full px-1.5 text-micro ${statusColour(run.status)}`}>
          {statusWords(run)}
        </span>
        <span className="font-medium">{run.testName}</span>
        <span className="text-small text-ink-faint">
          {run.testId}
          {took && ` · ${took}`}
          {run.port !== null && ` · port ${run.port}`}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {isActive(run.status) && (
            <button
              type="button"
              disabled={run.stopping}
              onClick={() => void cancel(run.id)}
              className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
            >
              {run.stopping ? "Stopping…" : "Cancel"}
            </button>
          )}
          {!isActive(run.status) && (
            <>
              <button
                type="button"
                onClick={() => setRunAgain(true)}
                className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink"
              >
                Run again
              </button>
              <SendToAgent run={run} workspaceId={workspaceId} />
            </>
          )}
        </div>
      </header>
      {run.error && (
        <p
          role="alert"
          className="shrink-0 border-b border-line px-3 py-1.5 text-red-400 select-text"
        >
          {run.error}
        </p>
      )}
      {error && (
        <div
          role="alert"
          className="flex shrink-0 items-start gap-3 border-b border-line px-3 py-1.5"
        >
          <p className="flex-1 text-red-400 select-text">{error}</p>
          <button type="button" onClick={dismissError} className="text-ink-faint hover:text-ink">
            Dismiss
          </button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Steps" className="w-56 shrink-0 overflow-y-auto border-r border-line py-2">
          {PHASES.map((phase) => {
            const inPhase = steps.filter((step) => step.phase === phase);
            if (inPhase.length === 0) return null;
            return (
              <div key={phase} className="mb-2">
                <h4 className="px-3 text-small font-semibold tracking-wider text-ink-faint uppercase">
                  {PHASE_WORDS[phase]}
                </h4>
                <ol>
                  {inPhase.map((step) => (
                    <li key={step.stepId}>
                      <button
                        type="button"
                        aria-current={selected?.stepId === step.stepId ? "true" : undefined}
                        aria-label={`${step.stepId}, ${stepStatusWords(step)}`}
                        onClick={() => setSelectedId(step.stepId)}
                        className={`flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-raised ${
                          selected?.stepId === step.stepId ? "bg-raised text-ink" : "text-ink-muted"
                        }`}
                      >
                        <span
                          className={`w-14 shrink-0 truncate rounded-full px-1.5 text-center text-micro ${statusColour(step.status)}`}
                        >
                          {step.status}
                        </span>
                        <span className="min-w-0 flex-1 truncate font-mono text-small">
                          {step.stepId}
                        </span>
                        <span className="shrink-0 text-micro text-ink-faint">
                          {tookWords(step)?.replace(" so far", "…") ?? ""}
                        </span>
                      </button>
                    </li>
                  ))}
                </ol>
              </div>
            );
          })}
          {steps.length === 0 && <p className="px-3 text-ink-faint">No steps yet.</p>}
        </nav>
        <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          {selected ? (
            <StepView
              key={selected.stepId}
              run={run}
              step={selected}
              words={words(selected.stepId)}
            />
          ) : (
            <p className="p-4 text-ink-faint">
              {run.status === "awaiting_approval"
                ? "Waiting for you to approve the file, at the top of the workspace."
                : run.status === "queued"
                  ? "Queued; Yardsort starts it in a moment."
                  : ""}
            </p>
          )}
        </div>
      </div>
      {runAgain && (
        <RunSmokeTestDialog
          workspaceId={workspaceId}
          testId={run.testId}
          inputs={run.inputs}
          onClose={() => setRunAgain(false)}
        />
      )}
    </section>
  );
}

/** One step: what it does, what it left, and its output. */
function StepView({ run, step, words }: { run: TestRun; step: TestStepRun; words?: string }) {
  const live = step.status === "running" || step.status === "waiting";
  const facts = outputFacts(step);
  const took = tookWords(step);
  return (
    <div className="flex flex-col gap-3 p-4">
      <div>
        <div className="flex items-center gap-2">
          <span className={`rounded-full px-1.5 text-micro ${statusColour(step.status)}`}>
            {stepStatusWords(step)}
          </span>
          <span className="font-mono">{step.stepId}</span>
          <span className="text-small text-ink-faint">
            {step.action}
            {took && ` · ${took}`}
          </span>
        </div>
        {words && <p className="mt-1 text-ink-muted select-text">{words}</p>}
      </div>
      {step.action === "manual" && <Manual run={run} step={step} />}
      {step.note && (
        <pre
          aria-label="Note"
          className={`rounded border border-line bg-canvas p-2 font-mono text-small leading-4 whitespace-pre-wrap select-text ${
            step.status === "failed" ? "text-red-300" : "text-ink-muted"
          }`}
        >
          {step.note}
        </pre>
      )}
      {facts.length > 0 && (
        <dl aria-label="Outputs" className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-small">
          {facts.map(([name, value]) => (
            <div key={name} className="contents">
              <dt className="font-mono text-ink-faint">{name}</dt>
              <dd className="min-w-0 truncate select-text" title={value}>
                {value}
              </dd>
            </div>
          ))}
        </dl>
      )}
      {step.outputs.body !== undefined && <Block label="Response body" text={step.outputs.body} />}
      {step.outputs.stdout !== undefined && (
        <Stream run={run} step={step} stream="stdout" live={live} />
      )}
      {step.outputs.stderr !== undefined && (
        <Stream run={run} step={step} stream="stderr" live={live} />
      )}
      {step.screenshots.length > 0 && (
        <div>
          <h4 className="text-small font-semibold tracking-wider text-ink-faint uppercase">
            Screenshots
          </h4>
          <ul className="mt-1 flex flex-col gap-0.5">
            {step.screenshots.map((path) => (
              <li key={path} className="flex items-center gap-2 text-small">
                <span className="min-w-0 truncate font-mono select-text">{path}</span>
                <button
                  type="button"
                  onClick={() => void native.revealInFileManager(path).catch(console.error)}
                  className="shrink-0 text-ink-faint hover:text-ink"
                >
                  Reveal
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/** A `manual` step: the question, and the answer while the run waits for it. */
function Manual({ run, step }: { run: TestRun; step: TestStepRun }) {
  const answer = useSmokeTestsStore((s) => s.answer);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const waiting = step.status === "waiting" && run.status === "running";
  const reply = async (passed: boolean) => {
    setBusy(true);
    await answer(run.id, step.stepId, passed, note.trim() || null);
    setBusy(false);
  };
  return (
    <div className="rounded border border-accent/40 bg-raised p-3">
      <p className="font-medium select-text">{step.outputs.prompt ?? "Does it look right?"}</p>
      {waiting ? (
        <>
          <textarea
            aria-label="Note"
            value={note}
            onChange={(event) => setNote(event.target.value)}
            placeholder="A note, if there is something to say…"
            rows={2}
            className="mt-2 w-full rounded border border-line bg-canvas px-2 py-1 outline-none select-text focus:border-accent"
          />
          <div className="mt-2 flex gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void reply(true)}
              className="rounded bg-accent px-4 py-1 font-medium text-canvas disabled:opacity-40"
            >
              Pass
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void reply(false)}
              className="rounded border border-red-400/60 px-4 py-1 text-red-300 hover:bg-red-500/10 disabled:opacity-40"
            >
              Fail
            </button>
          </div>
        </>
      ) : (
        step.outputs.note && (
          <p className="mt-1 text-ink-muted select-text">Note: {step.outputs.note}</p>
        )
      )}
    </div>
  );
}

function Block({ label, text }: { label: string; text: string }) {
  return (
    <div>
      <h4 className="text-small font-semibold tracking-wider text-ink-faint uppercase">{label}</h4>
      <pre className="mt-1 max-h-96 overflow-auto rounded border border-line bg-canvas p-2 font-mono text-small leading-4 whitespace-pre-wrap select-text">
        {text}
      </pre>
    </div>
  );
}

/** A step's stdout or stderr, tailed while the step runs. */
function Stream({
  run,
  step,
  stream,
  live,
}: {
  run: TestRun;
  step: TestStepRun;
  stream: "stdout" | "stderr";
  live: boolean;
}) {
  const output = useStepOutput(run.id, step.stepId, stream, live);
  if (output.loaded && output.text === "" && !live) return null;
  return (
    <div>
      <h4 className="text-small font-semibold tracking-wider text-ink-faint uppercase">
        {stream}
        {live && <span className="ml-2 normal-case tracking-normal">live</span>}
      </h4>
      {output.skipped && (
        <p className="text-small text-ink-faint">Earlier output is in the file, not shown here.</p>
      )}
      <pre
        aria-label={stream}
        className="mt-1 max-h-96 overflow-auto rounded border border-line bg-canvas p-2 font-mono text-small leading-4 whitespace-pre-wrap select-text"
      >
        {output.text}
      </pre>
    </div>
  );
}

/**
 * **Send result to agent**: type what `ys test wait` would have printed into one of the
 * workspace's agents, once it is quiet. With one agent it goes there; with several, a menu asks.
 */
function SendToAgent({ run, workspaceId }: { run: TestRun; workspaceId: string }) {
  const tabs = useTerminalStore((s) => s.tabs);
  const agents = tabs.filter(
    (tab) => tab.workspaceId === workspaceId && tab.recordId !== null && !tab.exit,
  );
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [state, setState] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = ipc.onTestResultSent((sent) => {
      if (sent.runId !== run.id) return;
      setState(sent.delivered ? "Sent." : (sent.why ?? "Not sent."));
    });
    return () => void unlisten.then((stop) => stop());
  }, [run.id]);

  const send = async (sessionId: string) => {
    setState("Sending…");
    try {
      const sent = await ipc.testRunSend(run.id, sessionId);
      setState(sent.delivered ? "Sent." : "Will be sent once the agent is quiet.");
    } catch (error) {
      setState(errorMessage(error));
    }
  };
  const items: MenuItem[] = agents.map((tab) => ({
    label: tab.title,
    onSelect: () => void send(tab.id),
  }));
  return (
    <>
      <button
        type="button"
        disabled={agents.length === 0}
        title={
          agents.length === 0
            ? "No agent is running in this workspace."
            : "Type the result into the workspace's agent, once it is quiet"
        }
        onClick={(event) => {
          const only = agents[0];
          if (agents.length === 1 && only) return void send(only.id);
          const box = event.currentTarget.getBoundingClientRect();
          setMenuAt({ x: box.left, y: box.bottom + 2 });
        }}
        className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
      >
        Send result to agent{agents.length > 1 ? "…" : ""}
      </button>
      {state && (
        <span role="status" className="text-small text-ink-faint">
          {state}
        </span>
      )}
      {menuAt && <ContextMenu at={menuAt} items={items} onClose={() => setMenuAt(null)} />}
    </>
  );
}

/** The dot a run's tab wears. */
export function RunTabDot({ run }: { run: TestRun }) {
  return <StatusDot activity={runActivity(run)} />;
}
