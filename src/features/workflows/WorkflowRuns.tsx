import { useState } from "react";
import { createPortal } from "react-dom";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { clockTime } from "@/features/schedules/words";
import { timeAgo } from "@/features/terminal/sessionText";
import { Markdown } from "@/lib/Markdown";
import { errorMessage, ipc, type WorkflowRun, type WorkflowStepRun } from "@/lib/ipc";
import { scheduleOf, useSchedulesStore } from "@/stores/schedules";
import { triggerOf, useTriggersStore } from "@/stores/triggers";
import { statusColour, useWorkflowStore } from "@/stores/workflows";
import { AnswerDialog } from "./AnswerDialog";
import { ApprovalDialog, looksLikeMarkdown, type Decision } from "./ApprovalDialog";
import { joinAnswers } from "./answers";
import { ACTION_WORDS } from "./words";

const EMPTY: WorkflowRun[] = [];

/** What the detail pane shows for a run: its summary, or one of its steps by id. */
export const SUMMARY = ":summary";

/** A run with more steps than this folds its runs of succeeded steps into one row. */
const FOLD_OVER = 6;
/** The fewest succeeded steps in a row that fold. */
const FOLD_AT_LEAST = 3;

const quietButton =
  "rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink";

interface Props {
  workflowId: string;
  /** The run on screen: the one picked, or the newest. Null when there are none. */
  selected: string | null;
  onSelect: (runId: string) => void;
}

/**
 * A workflow's runs, side by side: the list, newest first; the picked run's steps, with what
 * went wrong and what can be done about it on top; and, across the rest of the width, the
 * run's summary or the step picked. An active run can be stopped, and a failed one continued
 * from the step that failed.
 */
export function WorkflowRuns({ workflowId, selected, onSelect }: Props) {
  const runs = useWorkflowStore((s) => s.runs[workflowId]) ?? EMPTY;
  const steps = useWorkflowStore((s) => (selected ? s.steps[selected] : undefined));
  // The runs a schedule started, to say so on each.
  const started = useSchedulesStore((s) => s.started);
  // And the runs a trigger started.
  const triggered = useTriggersStore((s) => s.started);
  const [error, setError] = useState<string | null>(null);
  const run = runs.find((r) => r.id === selected) ?? null;

  async function cancel(target: WorkflowRun) {
    setError(null);
    try {
      await ipc.workflowCancel(target.id);
      await useWorkflowStore.getState().refresh();
    } catch (failed) {
      setError(errorMessage(failed));
    }
  }

  async function carryOn(target: WorkflowRun) {
    setError(null);
    try {
      await ipc.workflowContinue(target.id);
      await useWorkflowStore.getState().refresh();
    } catch (failed) {
      setError(errorMessage(failed));
    }
  }

  return (
    <section aria-label="Runs" className="flex min-h-0 min-w-0 flex-1">
      <div className="flex w-64 shrink-0 flex-col border-r border-line">
        <h3 className="shrink-0 px-3 pt-3 pb-2 text-small font-semibold tracking-wider text-ink-faint uppercase">
          Runs
        </h3>
        {runs.length === 0 ? (
          <p className="px-3 text-ink-faint">No runs yet. Run… starts one.</p>
        ) : (
          <ul aria-label="Run list" className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
            {runs.map((each) => {
              const picked = each.id === selected;
              return (
                <li key={each.id} className="mb-1">
                  <button
                    type="button"
                    aria-pressed={picked}
                    aria-label={`Run in ${each.workspaceName}, ${each.status}`}
                    className={`flex w-full flex-col gap-1 rounded border px-2.5 py-1.5 text-left hover:bg-raised ${picked ? "border-accent bg-raised" : "border-transparent"}`}
                    onClick={() => onSelect(each.id)}
                  >
                    <span className="flex items-center gap-2">
                      <span
                        className={`shrink-0 rounded-full px-1.5 text-micro ${statusColour(each.status)}`}
                      >
                        {each.status}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-small text-ink-faint">
                        {timeAgo(each.createdAt)}
                      </span>
                      {scheduleOf(started, each.id) && (
                        <span className="shrink-0 text-small text-ink-faint">scheduled</span>
                      )}
                      {triggerOf(triggered, each.id) && (
                        <span className="shrink-0 text-small text-ink-faint">triggered</span>
                      )}
                    </span>
                    <span className={`truncate ${picked ? "text-ink" : "text-ink-muted"}`}>
                      {each.workspaceName}
                    </span>
                    {each.blockedOn && (
                      <span className="text-small text-amber-300">
                        Blocked: waiting for {each.blockedOn}.
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      {run ? (
        <RunPanes
          key={run.id}
          run={run}
          steps={steps}
          scheduleName={scheduleOf(started, run.id)?.scheduleName ?? null}
          triggerName={triggerOf(triggered, run.id)?.triggerName ?? null}
          error={error}
          onError={setError}
          onCancel={() => void cancel(run)}
          onContinue={() => void carryOn(run)}
        />
      ) : (
        error && (
          <p role="alert" className="p-4 text-red-400 select-text">
            {error}
          </p>
        )
      )}
    </section>
  );
}

/** The step a run opens on: the one waiting on the person, else the one that failed, else the
 *  summary. */
export function openingPick(run: WorkflowRun, steps: WorkflowStepRun[] | undefined): string {
  const waiting = steps?.find((s) => s.status === "waiting");
  if (waiting) return waiting.stepId;
  if (run.status === "failed") {
    const failed = steps?.find((s) => s.status === "failed");
    if (failed) return failed.stepId;
  }
  return SUMMARY;
}

/** One row of the step rail: a step, or several succeeded ones folded together. */
type RailRow =
  { kind: "step"; step: WorkflowStepRun } | { kind: "folded"; steps: WorkflowStepRun[] };

/**
 * A long run's steps with its runs of succeeded steps folded, so the step that failed or is
 * waiting is in view without scrolling past the ones that went well. The step picked is never
 * folded away. A short run, or one unfolded, lists every step.
 */
export function railRows(steps: WorkflowStepRun[], picked: string, unfolded: boolean): RailRow[] {
  if (unfolded || steps.length <= FOLD_OVER) return steps.map((step) => ({ kind: "step", step }));
  const rows: RailRow[] = [];
  let streak: WorkflowStepRun[] = [];
  const flush = () => {
    if (streak.length >= FOLD_AT_LEAST) rows.push({ kind: "folded", steps: streak });
    else rows.push(...streak.map((step) => ({ kind: "step" as const, step })));
    streak = [];
  };
  for (const step of steps) {
    if (step.status === "succeeded" && step.stepId !== picked) {
      streak.push(step);
    } else {
      flush();
      rows.push({ kind: "step", step });
    }
  }
  flush();
  return rows;
}

function RunPanes({
  run,
  steps,
  scheduleName,
  triggerName,
  error,
  onError,
  onCancel,
  onContinue,
}: {
  run: WorkflowRun;
  steps: WorkflowStepRun[] | undefined;
  scheduleName: string | null;
  triggerName: string | null;
  error: string | null;
  onError: (message: string | null) => void;
  onCancel: () => void;
  onContinue: () => void;
}) {
  // Null until the person picks: the run then opens on what needs them, as its steps arrive.
  const [chosen, setChosen] = useState<string | null>(null);
  const [unfolded, setUnfolded] = useState(false);
  const [copied, setCopied] = useState(false);
  const picked = chosen ?? openingPick(run, steps);
  const active = run.status === "queued" || run.status === "running";
  const step = steps?.find((s) => s.stepId === picked) ?? null;
  const done = (steps ?? []).filter((s) => s.status === "succeeded").length;

  function copyId() {
    void writeText(run.id).then(
      () => setCopied(true),
      (failed) => onError(errorMessage(failed)),
    );
  }

  return (
    <>
      <div
        aria-label="Run"
        role="group"
        className="flex w-80 shrink-0 flex-col gap-2 overflow-y-auto border-r border-line bg-surface px-3 py-3"
      >
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <h3 className="truncate font-semibold">{run.workspaceName}</h3>
            <p className="text-small text-ink-faint">{runTimes(run)}</p>
          </div>
          <button
            type="button"
            title={`Copy the run's id, ${run.id}, for ys workflow runs --run`}
            className={`shrink-0 ${quietButton}`}
            onClick={copyId}
          >
            {copied ? "Copied" : "Copy id"}
          </button>
        </div>
        {scheduleName && <p className="text-ink-muted">Scheduled · {scheduleName}</p>}
        {triggerName && <p className="text-ink-muted">Triggered · {triggerName}</p>}
        {run.pullRequest && (
          <p className="text-ink-muted">
            Pull request #{run.pullRequest.number} {run.pullRequest.title}
          </p>
        )}
        {run.blockedOn && <p className="text-amber-300">Blocked: waiting for {run.blockedOn}.</p>}
        {error && (
          <p role="alert" className="text-red-400 select-text">
            {error}
          </p>
        )}
        {run.error && (
          <div className="flex flex-col items-start gap-2 rounded border border-red-500/40 bg-red-500/10 p-2.5">
            <p className="whitespace-pre-wrap text-red-400 select-text">{run.error}</p>
            {/* Beside the error, not after the steps: a run's notes and artifacts can be long
                enough to push anything below them out of sight. */}
            {run.status === "failed" && run.workspaceId && (
              <button
                type="button"
                title="Run the failed step again, then the steps after it. Steps that succeeded are kept."
                className="rounded bg-accent px-2.5 py-1 text-small font-medium text-canvas"
                onClick={onContinue}
              >
                Continue from the failed step
              </button>
            )}
          </div>
        )}
        {active && (
          <button type="button" className={`self-start ${quietButton}`} onClick={onCancel}>
            Cancel run
          </button>
        )}
        <button
          type="button"
          aria-pressed={picked === SUMMARY}
          className={`mt-1 flex items-center gap-2 rounded border px-2.5 py-1.5 text-left hover:bg-raised ${picked === SUMMARY ? "border-accent bg-raised" : "border-line"}`}
          onClick={() => setChosen(SUMMARY)}
        >
          <span className="flex-1 font-medium">Summary</span>
          <span className={`rounded-full px-1.5 text-micro ${statusColour(run.status)}`}>
            {run.status}
          </span>
        </button>
        <h4 className="mt-1 text-small font-semibold tracking-wider text-ink-faint uppercase">
          Steps{steps ? ` · ${done} of ${steps.length} done` : ""}
        </h4>
        <ol aria-label="Steps of this run" className="flex flex-col gap-0.5">
          {railRows(steps ?? [], picked, unfolded).map((row) =>
            row.kind === "folded" ? (
              <li key={`folded-${row.steps[0]!.stepId}`}>
                <button
                  type="button"
                  className="flex w-full items-center gap-2 rounded border border-line px-2 py-1 text-left hover:bg-raised"
                  onClick={() => setUnfolded(true)}
                >
                  <span aria-hidden className="size-2 shrink-0 rounded-full bg-green-400" />
                  <span className="flex-1 text-ink-muted">{row.steps.length} steps succeeded</span>
                  <span className="text-small text-accent">Show</span>
                </button>
              </li>
            ) : (
              <li key={row.step.stepId}>
                <StepRow
                  step={row.step}
                  picked={row.step.stepId === picked}
                  onPick={() => setChosen(row.step.stepId)}
                />
              </li>
            ),
          )}
        </ol>
      </div>
      <div
        aria-label={step ? `Step ${step.stepId}` : "Run summary"}
        role="region"
        className="min-h-0 min-w-0 flex-1 overflow-y-auto px-7 py-5"
      >
        {step ? (
          <StepDetail run={run} step={step} active={active} onError={onError} />
        ) : (
          <RunSummary run={run} steps={steps} />
        )}
      </div>
    </>
  );
}

function StepRow({
  step,
  picked,
  onPick,
}: {
  step: WorkflowStepRun;
  picked: boolean;
  onPick: () => void;
}) {
  const words = ACTION_WORDS[step.action as keyof typeof ACTION_WORDS] ?? step.action;
  return (
    <button
      type="button"
      aria-pressed={picked}
      className={`flex w-full items-center gap-2 rounded border px-2 py-1 text-left hover:bg-raised ${picked ? "border-accent bg-raised" : "border-transparent"}`}
      onClick={onPick}
    >
      <span aria-hidden className={`size-2 shrink-0 rounded-full ${dotColour(step.status)}`} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={`truncate ${step.status === "skipped" ? "text-ink-faint" : ""}`}>
          {words}
        </span>
        <span className="truncate font-mono text-small text-ink-faint">{step.stepId}</span>
      </span>
      {step.status !== "succeeded" && (
        <span className={`shrink-0 rounded-full px-1.5 text-micro ${statusColour(step.status)}`}>
          {step.status}
        </span>
      )}
    </button>
  );
}

function dotColour(status: string): string {
  switch (status) {
    case "succeeded":
      return "bg-green-400";
    case "failed":
      return "bg-red-400";
    case "running":
    case "waiting":
      return "bg-amber-300";
    default:
      return "bg-ink-faint";
  }
}

/** When a run was asked for and how long it took, in one line. */
function runTimes(run: WorkflowRun): string {
  const parts: string[] = [];
  if (run.createdAt !== null) parts.push(`Asked ${clockTime(run.createdAt)}`);
  if (run.endedAt !== null) parts.push(`ended ${clockTime(run.endedAt)}`);
  const from = run.startedAt ?? run.createdAt;
  if (from !== null && run.endedAt !== null) parts.push(lasted(run.endedAt - from));
  return parts.join(" · ");
}

/** "45 s", "12 min", "1 h 29 min". */
export function lasted(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** Where a run was asked for, in words. */
function askedFrom(requestedBy: string): string {
  if (requestedBy === "app") return "this window";
  if (requestedBy === "cli") return "the command line";
  return `a ${requestedBy}`;
}

/** The run as a whole: what it was asked with, and how it went. */
function RunSummary({ run, steps }: { run: WorkflowRun; steps: WorkflowStepRun[] | undefined }) {
  const inputs = Object.entries(run.inputs);
  const counts = new Map<string, number>();
  for (const step of steps ?? []) counts.set(step.status, (counts.get(step.status) ?? 0) + 1);
  return (
    <div className="flex max-w-4xl flex-col gap-5">
      <div className="flex items-center gap-2">
        <span className={`rounded-full px-1.5 text-micro ${statusColour(run.status)}`}>
          {run.status}
        </span>
        <h3 className="text-base font-semibold">Run summary</h3>
      </div>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5">
        <dt className="text-ink-faint">Workflow</dt>
        <dd>{run.workflowName}</dd>
        <dt className="text-ink-faint">Workspace</dt>
        <dd>{run.workspaceName}</dd>
        <dt className="text-ink-faint">Asked from</dt>
        <dd>{askedFrom(run.requestedBy)}</dd>
        {run.createdAt !== null && (
          <>
            <dt className="text-ink-faint">Asked</dt>
            <dd>{clockTime(run.createdAt)}</dd>
          </>
        )}
        {run.endedAt !== null && (
          <>
            <dt className="text-ink-faint">Ended</dt>
            <dd>
              {clockTime(run.endedAt)}
              {(run.startedAt ?? run.createdAt) !== null &&
                ` · took ${lasted(run.endedAt - (run.startedAt ?? run.createdAt)!)}`}
            </dd>
          </>
        )}
        {steps && (
          <>
            <dt className="text-ink-faint">Steps</dt>
            <dd>
              {[...counts.entries()].map(([status, n]) => `${n} ${status}`).join(", ") ||
                "none yet"}
            </dd>
          </>
        )}
        <dt className="text-ink-faint">Run id</dt>
        <dd className="font-mono text-small select-text">{run.id}</dd>
      </dl>
      {inputs.length > 0 && (
        <section aria-label="Inputs" className="flex flex-col gap-1.5">
          <h4 className="text-small font-semibold tracking-wider text-ink-faint uppercase">
            Inputs
          </h4>
          <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5">
            {inputs.map(([id, value]) => (
              <div key={id} className="contents">
                <dt className="font-mono text-small text-ink-faint">{id}</dt>
                <dd className="whitespace-pre-wrap select-text">{value}</dd>
              </div>
            ))}
          </dl>
        </section>
      )}
    </div>
  );
}

/** One step at the width of the window: what it noted, what it waits for, what it left. */
function StepDetail({
  run,
  step,
  active,
  onError,
}: {
  run: WorkflowRun;
  step: WorkflowStepRun;
  active: boolean;
  onError: (message: string | null) => void;
}) {
  const words = ACTION_WORDS[step.action as keyof typeof ACTION_WORDS] ?? step.action;
  const times: string[] = [];
  if (step.startedAt !== null) times.push(`Started ${clockTime(step.startedAt)}`);
  if (step.endedAt !== null) times.push(`ended ${clockTime(step.endedAt)}`);
  if (step.startedAt !== null && step.endedAt !== null)
    times.push(lasted(step.endedAt - step.startedAt));
  const refresh = () => useWorkflowStore.getState().refresh();
  return (
    <div className="flex max-w-4xl flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`rounded-full px-1.5 text-micro ${statusColour(step.status)}`}>
          {step.status}
        </span>
        <h3 className="text-base font-semibold">{words}</h3>
        <span className="font-mono text-small text-ink-faint">{step.stepId}</span>
      </div>
      {times.length > 0 && <p className="text-small text-ink-faint">{times.join(" · ")}</p>}
      {step.note && (
        <p className="leading-relaxed whitespace-pre-wrap text-ink-muted select-text">
          {step.note}
        </p>
      )}
      {step.action === "approve" && step.status === "waiting" && active && (
        <ApprovalBox
          runId={run.id}
          workspaceName={run.workspaceName}
          step={step}
          onError={onError}
          onDone={refresh}
        />
      )}
      {step.clarificationPrompt && step.status === "waiting" && active && (
        <AnswerBox
          key={step.clarificationPrompt}
          runId={run.id}
          workspaceName={run.workspaceName}
          step={step}
          onError={onError}
          onDone={refresh}
        />
      )}
      {step.outcome?.decision && (
        <p className="text-ink-muted select-text">
          {decisionWord(step.outcome.decision)}
          {step.outcome.feedback ? `: ${step.outcome.feedback}` : ""}
        </p>
      )}
      {step.artifacts.map((artifact) => (
        <section
          key={artifact.name}
          aria-label={`Artifact ${artifact.name}`}
          className="flex flex-col gap-1"
        >
          <h4 className="flex items-baseline gap-2">
            <span className="font-medium">Artifact {artifact.name}</span>
            <span className="font-mono text-micro text-ink-faint">
              {artifact.source}
              {artifact.detail ? ` ${artifact.detail}` : ""}
              {artifact.truncated ? " · cut short" : ""}
            </span>
          </h4>
          <div className="rounded border border-line bg-surface px-4 py-3 select-text">
            {looksLikeMarkdown(artifact) ? (
              <Markdown text={artifact.body} />
            ) : (
              <pre className="whitespace-pre-wrap text-ink">{artifact.body}</pre>
            )}
          </div>
        </section>
      ))}
    </div>
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
