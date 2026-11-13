import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { timeAgo } from "@/features/terminal/sessionText";
import { errorMessage, ipc, type WorkflowRun, type WorkflowStepRun } from "@/lib/ipc";
import { scheduleOf, useSchedulesStore } from "@/stores/schedules";
import { triggerOf, useTriggersStore } from "@/stores/triggers";
import { statusColour, useWorkflowStore } from "@/stores/workflows";
import { AnswerDialog } from "./AnswerDialog";
import { ApprovalDialog, type Decision } from "./ApprovalDialog";
import { joinAnswers } from "./answers";
import { ACTION_WORDS } from "./words";

const EMPTY: WorkflowRun[] = [];

interface Props {
  workflowId: string;
  /** The run whose steps the chart shows. */
  selected: string | null;
  onSelect: (runId: string | null) => void;
}

/**
 * A workflow's runs, newest first; one open shows its steps. An active one can be stopped, and a
 * failed one continued from the step that failed.
 */
export function WorkflowRuns({ workflowId, selected, onSelect }: Props) {
  const runs = useWorkflowStore((s) => s.runs[workflowId]) ?? EMPTY;
  const steps = useWorkflowStore((s) => (selected ? s.steps[selected] : undefined));
  // The runs a schedule started, to say so on each.
  const started = useSchedulesStore((s) => s.started);
  // And the runs a trigger started.
  const triggered = useTriggersStore((s) => s.started);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void useWorkflowStore.getState().loadRuns(workflowId);
  }, [workflowId]);

  useEffect(() => {
    if (selected) void useWorkflowStore.getState().loadSteps(selected);
  }, [selected]);

  async function cancel(run: WorkflowRun) {
    setError(null);
    try {
      await ipc.workflowCancel(run.id);
      await useWorkflowStore.getState().refresh();
    } catch (failed) {
      setError(errorMessage(failed));
    }
  }

  async function carryOn(run: WorkflowRun) {
    setError(null);
    try {
      await ipc.workflowContinue(run.id);
      await useWorkflowStore.getState().refresh();
    } catch (failed) {
      setError(errorMessage(failed));
    }
  }

  return (
    <section aria-label="Runs" className="flex min-h-0 flex-1 flex-col">
      <h3 className="shrink-0 px-3 pt-3 pb-2 text-small font-semibold tracking-wider text-ink-faint uppercase">
        Runs
      </h3>
      {error && (
        <p role="alert" className="px-3 pb-2 text-red-400 select-text">
          {error}
        </p>
      )}
      {runs.length === 0 ? (
        <p className="px-3 text-ink-faint">No runs yet.</p>
      ) : (
        <ul className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {runs.map((run) => {
            const open = run.id === selected;
            const active = run.status === "queued" || run.status === "running";
            const scheduled = scheduleOf(started, run.id);
            const fired = triggerOf(triggered, run.id);
            return (
              <li key={run.id} className="mb-1 rounded border border-line bg-canvas/40">
                <button
                  type="button"
                  aria-expanded={open}
                  aria-label={`Run in ${run.workspaceName}, ${run.status}`}
                  className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left hover:bg-raised"
                  onClick={() => onSelect(open ? null : run.id)}
                >
                  <span
                    className={`shrink-0 rounded-full px-1.5 text-micro ${statusColour(run.status)}`}
                  >
                    {run.status}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{run.workspaceName}</span>
                  {run.blockedOn && (
                    <span className="shrink-0 text-small text-amber-300">
                      Blocked: waiting for {run.blockedOn}.
                    </span>
                  )}
                  {!open && run.status === "failed" && run.error && (
                    <span
                      title={run.error}
                      className="max-w-[40%] shrink truncate text-small text-red-400"
                    >
                      {run.error}
                    </span>
                  )}
                  {scheduled && (
                    <span
                      title={`Started by the schedule “${scheduled.scheduleName}”`}
                      className="shrink-0 text-small text-ink-faint"
                    >
                      scheduled
                    </span>
                  )}
                  {fired && (
                    <span
                      title={`Started by the trigger “${fired.triggerName}”`}
                      className="shrink-0 text-small text-ink-faint"
                    >
                      triggered
                    </span>
                  )}
                  <span className="shrink-0 text-small text-ink-faint">
                    {timeAgo(run.createdAt)}
                  </span>
                </button>
                {open && (
                  <div className="border-t border-line px-2.5 py-2">
                    {scheduled && (
                      <p className="mb-1.5 text-ink-muted">Scheduled · {scheduled.scheduleName}</p>
                    )}
                    {fired && (
                      <p className="mb-1.5 text-ink-muted">Triggered · {fired.triggerName}</p>
                    )}
                    {run.pullRequest && (
                      <p className="mb-1.5 text-ink-muted">
                        Pull request #{run.pullRequest.number} {run.pullRequest.title}
                      </p>
                    )}
                    {run.blockedOn && (
                      <p className="mb-1.5 text-amber-300">Blocked: waiting for {run.blockedOn}.</p>
                    )}
                    {run.error && (
                      <p className="mb-1.5 whitespace-pre-wrap text-red-400 select-text">
                        {run.error}
                      </p>
                    )}
                    <ol className="flex flex-col gap-1">
                      {(steps ?? []).map((step) => (
                        <li key={step.stepId} className="flex flex-col">
                          <span className="flex items-center gap-2">
                            <span
                              className={`shrink-0 rounded-full px-1.5 text-micro ${statusColour(step.status)}`}
                            >
                              {step.status}
                            </span>
                            <span className="truncate">
                              {ACTION_WORDS[step.action as keyof typeof ACTION_WORDS] ??
                                step.action}
                            </span>
                            <span className="truncate font-mono text-small text-ink-faint">
                              {step.stepId}
                            </span>
                          </span>
                          {step.note && (
                            <span className="pl-1 whitespace-pre-wrap text-small text-ink-muted select-text">
                              {step.note}
                            </span>
                          )}
                          {step.action === "approve" && step.status === "waiting" && active && (
                            <ApprovalBox
                              runId={run.id}
                              workspaceName={run.workspaceName}
                              step={step}
                              onError={setError}
                              onDone={() => useWorkflowStore.getState().refresh()}
                            />
                          )}
                          {step.clarificationPrompt && step.status === "waiting" && active && (
                            <AnswerBox
                              key={step.clarificationPrompt}
                              runId={run.id}
                              workspaceName={run.workspaceName}
                              step={step}
                              onError={setError}
                              onDone={() => useWorkflowStore.getState().refresh()}
                            />
                          )}
                          {step.outcome?.decision && (
                            <span className="pl-1 text-small text-ink-muted select-text">
                              {decisionWord(step.outcome.decision)}
                              {step.outcome.feedback ? `: ${step.outcome.feedback}` : ""}
                            </span>
                          )}
                          {step.artifacts.map((artifact) => (
                            <span key={artifact.name} className="mt-1 flex flex-col gap-0.5 pl-1">
                              <span className="text-small text-ink-muted">
                                Artifact {artifact.name}
                                {artifact.truncated ? " (cut short)" : ""}
                              </span>
                              <span className="font-mono text-micro text-ink-faint">
                                {artifact.source}
                                {artifact.detail ? ` ${artifact.detail}` : ""}
                              </span>
                              <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded border border-line px-1.5 py-1 text-small text-ink select-text">
                                {artifact.body}
                              </pre>
                            </span>
                          ))}
                        </li>
                      ))}
                    </ol>
                    {active && (
                      <button
                        type="button"
                        className="mt-2 rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink"
                        onClick={() => void cancel(run)}
                      >
                        Cancel run
                      </button>
                    )}
                    {run.status === "failed" && run.workspaceId && (
                      <button
                        type="button"
                        title="Run the failed step again, then the steps after it. Steps that succeeded are kept."
                        className="mt-2 rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink"
                        onClick={() => void carryOn(run)}
                      >
                        Continue from the failed step
                      </button>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function decisionWord(decision: string): string {
  if (decision === "approved") return "Approved";
  if (decision === "rejected") return "Rejected";
  if (decision === "changes_requested") return "Changes requested";
  return decision;
}

/**
 * A clarifying step: the questions, and a dialog that takes them one at a time and sends one
 * answer for the round. What is typed survives closing the dialog until the round changes.
 */
function AnswerBox({
  runId,
  workspaceName,
  step,
  onError,
  onDone,
}: {
  runId: string;
  workspaceName: string;
  step: WorkflowStepRun;
  onError: (message: string | null) => void;
  onDone: () => Promise<void>;
}) {
  const questions =
    step.clarificationQuestions.length > 0
      ? step.clarificationQuestions
      : [step.clarificationPrompt ?? ""];
  const [answers, setAnswers] = useState<string[]>(() => questions.map(() => ""));
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const answered = answers.filter((answer) => answer.trim() !== "").length;

  async function showCode() {
    onError(null);
    setBusy(true);
    try {
      await ipc.workflowShowApprovalCode(runId, step.stepId);
      await onDone();
    } catch (failed) {
      onError(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  async function send() {
    setProblem(null);
    setBusy(true);
    try {
      await ipc.workflowAnswer(runId, step.stepId, joinAnswers(questions, answers));
      setOpen(false);
      setAnswers(questions.map(() => ""));
      await onDone();
    } catch (failed) {
      setProblem(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  const count = questions.length === 1 ? "the question" : `${questions.length} questions`;
  return (
    <div className="mt-1 flex flex-col gap-1 pl-1">
      <span className="text-small text-amber-300">Blocked: waiting for your answer.</span>
      {step.approvalCode ? (
        <span className="text-small text-ink select-text">
          Code {step.approvalCode}. A terminal needs this. This window does not.
        </span>
      ) : (
        <span className="text-small text-ink-muted">
          The code is not in this window yet. Show a new code to use a terminal.
        </span>
      )}
      {step.approvalLocked ? (
        <span className="text-small text-ink-muted">
          Too many wrong codes. Show a new code to use a terminal again.
        </span>
      ) : null}
      <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded border border-line px-1.5 py-1 text-small text-ink select-text">
        {step.clarificationPrompt}
      </pre>
      <span className="flex flex-wrap items-center gap-1">
        <button
          type="button"
          disabled={busy}
          className="rounded bg-accent px-2 py-0.5 text-small font-medium text-canvas disabled:opacity-50"
          onClick={() => setOpen(true)}
        >
          {answered > 0 ? `Continue answering` : `Answer ${count}`}
        </button>
        <button
          type="button"
          disabled={busy}
          className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink disabled:opacity-50"
          onClick={() => void showCode()}
        >
          Show a new code
        </button>
        {answered > 0 && questions.length > 1 && (
          <span className="text-small text-ink-faint">
            {answered} of {questions.length} answered, not sent
          </span>
        )}
      </span>
      {open &&
        createPortal(
          <AnswerDialog
            stepId={step.stepId}
            workspaceName={workspaceName}
            questions={questions}
            answers={answers}
            onAnswers={setAnswers}
            busy={busy}
            error={problem}
            onSend={() => void send()}
            onClose={() => setOpen(false)}
          />,
          document.body,
        )}
    </div>
  );
}

/**
 * The pending approval: the documents it reads, and a dialog the size of the window to read
 * them in and decide. A gate that reads nothing can be approved or rejected here at once; a
 * note, and so a request for changes, still goes through the dialog. The note survives closing
 * the dialog, and a quick decision sends it too.
 */
function ApprovalBox({
  runId,
  workspaceName,
  step,
  onError,
  onDone,
}: {
  runId: string;
  workspaceName: string;
  step: WorkflowStepRun;
  onError: (message: string | null) => void;
  onDone: () => Promise<void>;
}) {
  const [feedback, setFeedback] = useState("");
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const consumed = step.consumed ?? [];

  async function decide(decision: Decision) {
    setProblem(null);
    setBusy(true);
    try {
      await ipc.workflowDecide(runId, step.stepId, decision, feedback.trim() || null);
      setFeedback("");
      setOpen(false);
      await onDone();
    } catch (failed) {
      setProblem(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  async function showCode() {
    onError(null);
    setBusy(true);
    try {
      await ipc.workflowShowApprovalCode(runId, step.stepId);
      await onDone();
    } catch (failed) {
      onError(errorMessage(failed));
    } finally {
      setBusy(false);
    }
  }

  const names = consumed.map((artifact) => artifact.name);
  return (
    <div className="mt-1 flex flex-col gap-1 pl-1">
      <span className="text-small text-amber-300">Blocked: waiting for your approval.</span>
      <span className="text-small text-ink-muted">Approve, reject, or request changes.</span>
      {step.approvalCode ? (
        <span className="text-small text-ink select-text">
          Code {step.approvalCode}. A terminal needs this. This window does not.
        </span>
      ) : (
        <span className="text-small text-ink-muted">
          The code is not in this window yet. Show a new code to use a terminal.
        </span>
      )}
      {step.approvalLocked ? (
        <span className="text-small text-ink-muted">
          Too many wrong codes. Show a new code to use a terminal again.
        </span>
      ) : null}
      {names.length > 0 && (
        <span className="text-small text-ink-muted">To review: {names.join(", ")}</span>
      )}
      <span className="flex flex-wrap items-center gap-1">
        <button
          type="button"
          disabled={busy}
          className="rounded bg-accent px-2 py-0.5 text-small font-medium text-canvas disabled:opacity-50"
          onClick={() => (names.length === 0 ? void decide("approved") : setOpen(true))}
        >
          {names.length === 0
            ? "Approve"
            : names.length === 1
              ? `Review ${names[0]} and decide`
              : "Review and decide"}
        </button>
        {names.length === 0 && (
          <>
            <button
              type="button"
              disabled={busy}
              className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink disabled:opacity-50"
              onClick={() => void decide("rejected")}
            >
              Reject
            </button>
            <button
              type="button"
              disabled={busy}
              className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink disabled:opacity-50"
              onClick={() => setOpen(true)}
            >
              Add a note…
            </button>
          </>
        )}
        <button
          type="button"
          disabled={busy}
          className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink disabled:opacity-50"
          onClick={() => void showCode()}
        >
          Show a new code
        </button>
        {feedback.trim() !== "" && !open && (
          <span className="text-small text-ink-faint">Note written, not sent</span>
        )}
      </span>
      {problem && !open && (
        <span role="alert" className="text-small text-red-400 select-text">
          {problem}
        </span>
      )}
      {open &&
        createPortal(
          <ApprovalDialog
            stepId={step.stepId}
            workspaceName={workspaceName}
            instructions={step.note}
            documents={consumed}
            feedback={feedback}
            onFeedback={setFeedback}
            busy={busy}
            error={problem}
            onDecide={(decision) => void decide(decision)}
            onClose={() => setOpen(false)}
          />,
          document.body,
        )}
    </div>
  );
}
