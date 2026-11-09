import { useEffect, useId, useMemo, useRef, useState } from "react";
import { EnvUses, Steps } from "@/features/smoke/ApprovalCard";
import { InputField, type FieldInput } from "@/features/workflows/InputField";
import {
  errorMessage,
  ipc,
  type IfMissed,
  type Schedule,
  type ScheduleNotify,
  type SchedulePreview,
  type ScheduleSpec,
  type ScheduleTarget,
  type TestItem,
  type TestReview,
} from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useSchedulesStore } from "@/stores/schedules";
import { useWorkflowStore } from "@/stores/workflows";
import { fieldsOf, KINDS, specOf, WEEKDAYS, type Unit, type WhenFields } from "./when";
import { clockTime } from "./words";

const control =
  "h-8 w-full rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";
const label = "text-[11px] font-semibold tracking-wider text-ink-faint uppercase";

/** An input of a workflow or of a smoke test, with what it starts on. */
type Asked = FieldInput & { default: string | null };

/** How long after the last keystroke the core is asked what a spec comes to. */
const PREVIEW_AFTER_MS = 200;

interface Props {
  /** Change this schedule; without it, the dialog makes a new one. */
  schedule?: Schedule;
  /** A new schedule starts on these, when where it was opened from already knows them. */
  target?: ScheduleTarget;
  targetId?: string;
  workspaceId?: string;
  onClose: () => void;
  onSaved?: (schedule: Schedule) => void;
}

/**
 * A schedule, new or being changed: what to run, where, its inputs, when, what to do with a
 * missed time, and when to be told. Everything is checked by the core when it is saved, exactly
 * as `ys schedule add` does; what shows here first — the next times, the test's steps — is so
 * the person can see what they are asking for.
 *
 * For a smoke test the dialog shows the file's steps, and saving approves that file, as pressing
 * Run does: the first scheduled run then goes ahead without waiting to be seen.
 */
export function ScheduleDialog(props: Props) {
  const { schedule, onClose, onSaved } = props;
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);
  const workflows = useWorkflowStore((s) => s.items);
  const projects = useProjectsStore((s) => s.projects);
  const allHarnesses = useHarnessStore((s) => s.harnesses);

  const [target, setTarget] = useState<ScheduleTarget>(
    schedule?.target ?? props.target ?? "workflow",
  );
  const [targetId, setTargetId] = useState(schedule?.targetId ?? props.targetId ?? "");
  const [workspaceId, setWorkspaceId] = useState(schedule?.workspaceId ?? props.workspaceId ?? "");
  const [answers, setAnswers] = useState<Record<string, string>>(schedule?.inputs ?? {});
  const [when, setWhen] = useState<WhenFields>(() => fieldsOf(schedule?.spec ?? null));
  const [missed, setMissed] = useState<IfMissed>(schedule?.missed ?? "run_once");
  const [notify, setNotify] = useState<ScheduleNotify>(schedule?.notify ?? "failure");
  const [name, setName] = useState(schedule?.name ?? "");
  // The chosen workspace's smoke tests, and which workspace the core listed them for.
  const [tests, setTests] = useState<{ workspace: string; items: TestItem[] } | null>(null);
  // The chosen test as the approval card would show it, and which test in which workspace.
  const [reviewed, setReviewed] = useState<{ key: string; review: TestReview } | null>(null);
  // What the core said the spec comes to, and for which spec it said it.
  const [previewed, setPreviewed] = useState<{ key: string; preview: SchedulePreview } | null>(
    null,
  );
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

  const workspaces = projects.flatMap((project) =>
    project.workspaces
      .filter((w) => !w.archived && !w.missing)
      .map((w) => ({ id: w.id, name: `${project.name} / ${w.name}` })),
  );

  const workflow =
    target === "workflow"
      ? (workflows.find((item) => item.id === targetId)?.workflow ?? null)
      : null;
  // One about a pull request elsewhere is started from that pull request, never on its own.
  const runnableWorkflows = workflows.filter(
    (item) =>
      item.problems.length === 0 && item.workflow && item.workflow.trigger.context === "workspace",
  );

  // The workspace's tests are wanted for a test schedule, and for a workflow's `test` input.
  const wantsTests =
    target === "test" || (workflow?.inputs.some((input) => input.kind === "test") ?? false);
  useEffect(() => {
    if (!wantsTests || !workspaceId) return;
    let current = true;
    ipc.testList(workspaceId).then(
      (found) => current && setTests({ workspace: workspaceId, items: found }),
      () => current && setTests({ workspace: workspaceId, items: [] }),
    );
    return () => {
      current = false;
    };
  }, [wantsTests, workspaceId]);
  const testsHere = tests?.workspace === workspaceId ? tests.items : null;
  const runnableTests = useMemo(
    () => (testsHere ?? []).filter((test) => test.runnable && !test.browserMissing),
    [testsHere],
  );
  const test = target === "test" ? (testsHere?.find((item) => item.id === targetId) ?? null) : null;

  const reviewKey = `${workspaceId}|${targetId}`;
  const review = reviewed?.key === reviewKey ? reviewed.review : null;
  useEffect(() => {
    if (!test?.runnable || !workspaceId) return;
    let current = true;
    ipc.testReview(workspaceId, test.id).then(
      (found) => current && setReviewed({ key: `${workspaceId}|${test.id}`, review: found }),
      (failed) => current && setError(errorMessage(failed)),
    );
    return () => {
      current = false;
    };
  }, [workspaceId, test]);

  const inputs: Asked[] = target === "workflow" ? (workflow?.inputs ?? []) : (test?.inputs ?? []);
  // Every input starts from its default. A required agent starts on the first one installed,
  // a required choice on its first option, and a test on one that can run here: what a control
  // shows must be the answer that is saved.
  const firstAgent = allHarnesses.find((h) => h.enabled && h.resolvedPath)?.id ?? "";
  const answerFor = (input: Asked): string => {
    const stored = answers[input.id];
    if (input.kind === "test") {
      const pick = stored ?? input.default ?? "";
      return pick && runnableTests.some((item) => item.id === pick)
        ? pick
        : (runnableTests[0]?.id ?? "");
    }
    if (stored !== undefined) return stored;
    if (input.default) return input.default;
    if (!input.required) return "";
    if (input.kind === "harness") return firstAgent;
    if (input.kind === "choice") return input.options[0] ?? "";
    return "";
  };
  const given = Object.fromEntries(inputs.map((input) => [input.id, answerFor(input)]));
  const missing = inputs.filter((input) => input.required && !given[input.id]?.trim());

  const spec = specOf(when);
  // The spec's text keys the preview: the object is new on every render.
  const specKey = spec ? JSON.stringify(spec) : "";
  const preview = previewed?.key === specKey ? previewed.preview : null;
  useEffect(() => {
    if (!specKey) return;
    let current = true;
    const timer = window.setTimeout(() => {
      ipc.schedulePreview(JSON.parse(specKey) as ScheduleSpec).then(
        (found) => current && setPreviewed({ key: specKey, preview: found }),
        (failed) => current && setError(errorMessage(failed)),
      );
    }, PREVIEW_AFTER_MS);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [specKey]);

  const chosen = target === "workflow" ? !!workflow : !!test?.runnable && !!review;
  const ready =
    chosen &&
    !!workspaceId &&
    !!spec &&
    preview !== null &&
    preview.problem === null &&
    missing.length === 0 &&
    !busy;

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!ready || !spec) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.scheduleSave({
        id: schedule?.id ?? null,
        name: name.trim() || null,
        target,
        targetId,
        workspaceId,
        inputs: Object.fromEntries(Object.entries(given).filter(([, value]) => value.trim())),
        spec,
        missed,
        notify,
        // The hash of the file the dialog showed: saving approves that file and no other.
        contentHash: target === "test" ? (review?.contentHash ?? null) : null,
      });
      await useSchedulesStore.getState().refresh();
      onSaved?.(saved);
      onClose();
    } catch (failed) {
      setError(errorMessage(failed));
      setBusy(false);
    }
  }

  const setKind = (next: ScheduleTarget) => {
    setTarget(next);
    setTargetId("");
    setAnswers({});
  };
  const patch = (next: Partial<WhenFields>) => setWhen((fields) => ({ ...fields, ...next }));
  const targetName = workflow?.name ?? test?.name ?? "";

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
        onSubmit={save}
        className="flex max-h-[84vh] w-[34rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            {schedule ? `Edit “${schedule.name}”` : "New schedule"}
          </h2>
          <p className="mt-1 text-ink-muted">
            Runs while Yardsort is open, at this machine’s local time.
          </p>
        </header>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5 pt-4">
          <label className="flex flex-col gap-1.5">
            <span className={label}>Run</span>
            <select
              aria-label="What to run"
              className={control}
              value={target}
              onChange={(event) => setKind(event.target.value as ScheduleTarget)}
            >
              <option value="workflow">A workflow</option>
              <option value="test">A smoke test</option>
            </select>
          </label>

          <label className="flex flex-col gap-1.5">
            <span className={label}>In workspace</span>
            <select
              aria-label="Workspace"
              className={control}
              value={workspaceId}
              onChange={(event) => setWorkspaceId(event.target.value)}
            >
              <option value="">Choose…</option>
              {workspaces.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.name}
                </option>
              ))}
            </select>
          </label>

          {target === "workflow" ? (
            <label className="flex flex-col gap-1.5">
              <span className={label}>Workflow</span>
              <select
                aria-label="Workflow"
                className={control}
                value={targetId}
                onChange={(event) => {
                  setTargetId(event.target.value);
                  setAnswers({});
                }}
              >
                <option value="">Choose…</option>
                {runnableWorkflows.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                  </option>
                ))}
              </select>
            </label>
          ) : (
            <label className="flex flex-col gap-1.5">
              <span className={label}>Smoke test</span>
              <select
                aria-label="Smoke test"
                className={control}
                value={targetId}
                disabled={!workspaceId || testsHere === null}
                onChange={(event) => {
                  setTargetId(event.target.value);
                  setAnswers({});
                }}
              >
                <option value="">
                  {!workspaceId
                    ? "Choose a workspace first"
                    : testsHere === null
                      ? "Loading…"
                      : runnableTests.length === 0
                        ? "No runnable smoke tests here"
                        : "Choose…"}
                </option>
                {runnableTests.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                  </option>
                ))}
              </select>
            </label>
          )}

          {inputs.map((input) => (
            <InputField
              key={input.id}
              input={input}
              value={given[input.id] ?? ""}
              onChange={(value) => setAnswers((all) => ({ ...all, [input.id]: value }))}
              tests={testsHere === null ? null : runnableTests}
              hasWorkspace={!!workspaceId}
            />
          ))}

          {review && (
            <section aria-label="What it does" className="flex flex-col gap-2">
              <Steps review={review} />
              <EnvUses review={review} />
              <p className="text-ink-faint">
                {review.approved
                  ? "You approved this exact file before."
                  : "Saving approves this file, so its runs go ahead without waiting for you. If the file changes, the next run waits to be seen."}
              </p>
            </section>
          )}

          <fieldset className="flex flex-col gap-2">
            <legend className={label}>When</legend>
            <div className="mt-1.5 flex flex-wrap items-center gap-2">
              <select
                aria-label="How often"
                className={`${control} w-44`}
                value={when.kind}
                onChange={(event) => patch({ kind: event.target.value as WhenFields["kind"] })}
              >
                {KINDS.map((kind) => (
                  <option key={kind.id} value={kind.id}>
                    {kind.label}
                  </option>
                ))}
              </select>
              {when.kind === "once" && (
                <input
                  type="datetime-local"
                  aria-label="Date and time"
                  className={`${control} w-56`}
                  value={when.at}
                  onChange={(event) => patch({ at: event.target.value })}
                />
              )}
              {when.kind === "every" && (
                <>
                  <input
                    type="number"
                    min={1}
                    aria-label="Every how many"
                    className={`${control} w-20`}
                    value={when.every}
                    onChange={(event) => patch({ every: event.target.value })}
                  />
                  <select
                    aria-label="Unit"
                    className={`${control} w-28`}
                    value={when.unit}
                    onChange={(event) => patch({ unit: event.target.value as Unit })}
                  >
                    <option value="minutes">minutes</option>
                    <option value="hours">hours</option>
                    <option value="days">days</option>
                  </select>
                </>
              )}
              {when.kind === "monthly" && (
                <select
                  aria-label="Day of the month"
                  className={`${control} w-32`}
                  value={when.day}
                  onChange={(event) => patch({ day: event.target.value })}
                >
                  {Array.from({ length: 28 }, (_, i) => String(i + 1)).map((day) => (
                    <option key={day} value={day}>
                      Day {day}
                    </option>
                  ))}
                  <option value="last">Last day</option>
                </select>
              )}
              {(when.kind === "daily" || when.kind === "weekly" || when.kind === "monthly") && (
                <input
                  type="time"
                  aria-label="Time"
                  className={`${control} w-28`}
                  value={when.time}
                  onChange={(event) => patch({ time: event.target.value })}
                />
              )}
              {when.kind === "cron" && (
                <input
                  aria-label="Cron expression"
                  placeholder="minute hour day month weekday"
                  spellCheck={false}
                  className={`${control} min-w-0 flex-1 font-mono`}
                  value={when.expr}
                  onChange={(event) => patch({ expr: event.target.value })}
                />
              )}
            </div>
            {when.kind === "weekly" && (
              <div role="group" aria-label="Days" className="flex flex-wrap gap-1">
                {WEEKDAYS.map((day) => {
                  const on = when.days.includes(day.id);
                  return (
                    <button
                      key={day.id}
                      type="button"
                      aria-pressed={on}
                      onClick={() =>
                        patch({
                          // Kept in week order, whatever order they were ticked in.
                          days: WEEKDAYS.map((d) => d.id).filter((id) =>
                            id === day.id ? !on : when.days.includes(id),
                          ),
                        })
                      }
                      className={`rounded-full border px-2.5 py-0.5 ${
                        on
                          ? "border-accent bg-accent/15 text-ink"
                          : "border-line text-ink-muted hover:text-ink"
                      }`}
                    >
                      {day.label}
                    </button>
                  );
                })}
              </div>
            )}
            {spec && preview?.problem && (
              <p role="alert" className="text-red-400 select-text">
                {preview.problem}
              </p>
            )}
            {spec && preview && !preview.problem && (
              <div className="rounded border border-line bg-canvas/40 p-2.5">
                <p className="text-ink">{preview.when}</p>
                <ul aria-label="Next times" className="mt-1 flex flex-col text-ink-muted">
                  {preview.next.map((at) => (
                    <li key={at} className="tabular-nums">
                      {clockTime(at)}
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </fieldset>

          <label className="flex flex-col gap-1.5">
            <span className={label}>If a time is missed</span>
            <select
              aria-label="If a time is missed"
              className={control}
              value={missed}
              onChange={(event) => setMissed(event.target.value as IfMissed)}
            >
              <option value="run_once">Run once when Yardsort next opens</option>
              <option value="skip">Skip it and wait for the next time</option>
            </select>
          </label>

          <label className="flex flex-col gap-1.5">
            <span className={label}>Tell me</span>
            <select
              aria-label="Tell me"
              className={control}
              value={notify}
              onChange={(event) => setNotify(event.target.value as ScheduleNotify)}
            >
              <option value="failure">When a run fails</option>
              <option value="always">How every run ends</option>
              <option value="never">Never</option>
            </select>
          </label>

          <label className="flex flex-col gap-1.5">
            <span className={label}>Name (optional)</span>
            <input
              aria-label="Name"
              className="rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
              placeholder={targetName}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>

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
            {busy ? "Saving…" : schedule ? "Save" : "Add schedule"}
          </button>
        </footer>
      </form>
    </div>
  );
}
