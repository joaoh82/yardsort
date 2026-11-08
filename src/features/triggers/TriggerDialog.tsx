import { useEffect, useId, useMemo, useRef, useState } from "react";
import { InputField, type FieldInput } from "@/features/workflows/InputField";
import {
  errorMessage,
  ipc,
  type ReviewStateFilter,
  type ScheduleNotify,
  type ScheduleTarget,
  type TestItem,
  type TriggerEvent,
  type TriggerFilters,
  type TriggerItem,
  type UntoldAgent,
} from "@/lib/ipc";
import { useModalFocus } from "@/lib/useModalFocus";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTriggersStore } from "@/stores/triggers";
import { useWorkflowStore } from "@/stores/workflows";
import { defaultQuietMs, EVENTS, sentence, subjectOf } from "./words";

const control =
  "h-8 w-full rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";
const label = "text-[11px] font-semibold tracking-wider text-ink-faint uppercase";
const check = "flex items-center gap-2 text-ink";

/** An input of a workflow or of a smoke test, with what it starts on. */
type Asked = FieldInput & { default: string | null };

const GROUPS = [...new Set(EVENTS.map((event) => event.group))];
const STATES: { id: ReviewStateFilter; label: string }[] = [
  { id: "approved", label: "Approved" },
  { id: "changes_requested", label: "Changes requested" },
  { id: "commented", label: "Commented" },
];

/** A quiet time as the dialog edits it: a number and its unit. */
type Unit = "seconds" | "minutes";
const unitMs: Record<Unit, number> = { seconds: 1000, minutes: 60_000 };
function quietFields(ms: number): { amount: string; unit: Unit } {
  return ms > 0 && ms % 60_000 === 0
    ? { amount: String(ms / 60_000), unit: "minutes" }
    : { amount: String(Math.round(ms / 1000)), unit: "seconds" };
}

interface Props {
  /** Change this trigger; without it, the dialog makes a new one. */
  trigger?: TriggerItem;
  /** A new trigger starts here, when where it was opened from knows. */
  workspaceId?: string;
  sessionId?: string;
  onClose: () => void;
  onSaved?: (trigger: TriggerItem) => void;
}

/**
 * A trigger, new or being changed, in the order its questions can be answered: where, when
 * (the events, then their filters), what to run and its inputs, the options, a name. What it
 * will do is said in words as it is filled in. Everything is checked by the core when it is
 * saved, exactly as `ys trigger add` does.
 *
 * A trigger that newly listens for `ys workspace done` in a workspace with agents already
 * running says they were not told about it, and offers **Tell the agent**.
 */
export function TriggerDialog(props: Props) {
  const { trigger, onClose, onSaved } = props;
  const titleId = useId();
  const dialogRef = useRef<HTMLFormElement>(null);
  useModalFocus(dialogRef);
  const workflows = useWorkflowStore((s) => s.items);
  const projects = useProjectsStore((s) => s.projects);
  const presets = useTriggersStore((s) => s.presets);
  const allHarnesses = useHarnessStore((s) => s.harnesses);
  const sessionsByWorkspace = useSessionsStore((s) => s.byWorkspace);

  const [preset, setPreset] = useState<string | null>(trigger?.preset ?? null);
  const [workspaceId, setWorkspaceId] = useState(trigger?.workspaceId ?? props.workspaceId ?? "");
  const [sessionId, setSessionId] = useState(trigger?.sessionId ?? props.sessionId ?? "");
  const [events, setEvents] = useState<TriggerEvent[]>(trigger?.events ?? []);
  const [filters, setFilters] = useState<TriggerFilters>(trigger?.filters ?? {});
  const [target, setTarget] = useState<ScheduleTarget>(trigger?.target ?? "workflow");
  const [targetId, setTargetId] = useState(trigger?.targetId ?? "");
  const [answers, setAnswers] = useState<Record<string, string>>(trigger?.inputs ?? {});
  // The quiet time follows the events until it is changed by hand.
  const [quiet, setQuiet] = useState(() => (trigger ? quietFields(trigger.quietMs) : null));
  const [cap, setCap] = useState(String(trigger?.cap ?? 3));
  const [notify, setNotify] = useState<ScheduleNotify>(trigger?.notify ?? "failure");
  const [name, setName] = useState(trigger?.name ?? "");
  // The chosen workspace's smoke tests, and which workspace the core listed them for.
  const [tests, setTests] = useState<{ workspace: string; items: TestItem[] } | null>(null);
  // Saved, with agents already running that were not told about `ys workspace done`.
  const [untold, setUntold] = useState<{ saved: TriggerItem; agents: UntoldAgent[] } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void useHarnessStore.getState().load();
    void useTriggersStore.getState().loadPresets();
    if (!useWorkflowStore.getState().loaded) void useWorkflowStore.getState().load();
  }, []);

  useEffect(() => {
    if (workspaceId) void useSessionsStore.getState().load(workspaceId);
  }, [workspaceId]);

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
  const workspaceName = workspaces.find((w) => w.id === workspaceId)?.name ?? "";
  // A session trigger belongs to an agent; one that has ended can still be the one it was.
  const agents = (sessionsByWorkspace[workspaceId] ?? []).filter(
    (record) => record.running || record.id === sessionId,
  );

  const workflow =
    target === "workflow"
      ? (workflows.find((item) => item.id === targetId)?.workflow ?? null)
      : null;
  const runnableWorkflows = workflows.filter((item) => item.problems.length === 0 && item.workflow);

  // The workspace's tests are wanted for a test trigger, for a workflow's `test` input, and for
  // the `test.finished` filter.
  const wantsTests =
    target === "test" ||
    events.includes("test.finished") ||
    (workflow?.inputs.some((input) => input.kind === "test") ?? false);
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

  const inputs: Asked[] = target === "workflow" ? (workflow?.inputs ?? []) : (test?.inputs ?? []);
  // As the schedule dialog answers them: what a control shows must be the answer that is saved.
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

  const quietNow = quiet ?? quietFields(defaultQuietMs(events));
  const quietMs = Math.round(Number(quietNow.amount) * unitMs[quietNow.unit]);
  const capNumber = Number(cap);
  const optionsOk =
    Number.isFinite(quietMs) && quietMs >= 0 && Number.isInteger(capNumber) && capNumber >= 1;

  const chosen = target === "workflow" ? !!workflow : !!test?.runnable;
  const ready =
    chosen && !!workspaceId && events.length > 0 && missing.length === 0 && optionsOk && !busy;

  // Only the filters of the events ticked: the core refuses one given for an event it is not.
  const has = (event: TriggerEvent) => events.includes(event);
  const posts = has("pr.review") || has("pr.comment");
  const sent: TriggerFilters = {
    ...(has("agent.settled") && filters.newCommits ? { newCommits: true } : {}),
    ...(has("agent.settled") && filters.clean ? { clean: true } : {}),
    ...(has("agent.exited") && filters.exit ? { exit: filters.exit } : {}),
    ...(has("test.finished") && filters.test ? { test: filters.test } : {}),
    ...(has("test.finished") && filters.verdict ? { verdict: filters.verdict } : {}),
    ...(has("workflow.finished") && filters.workflow ? { workflow: filters.workflow } : {}),
    ...(has("workflow.finished") && filters.outcome ? { outcome: filters.outcome } : {}),
    ...(has("pr.opened") && filters.drafts ? { drafts: true } : {}),
    ...(posts && filters.from === "anyone" ? { from: "anyone" as const } : {}),
    ...(has("pr.review") && filters.states?.length ? { states: filters.states } : {}),
    ...(posts && filters.noBots ? { noBots: true } : {}),
  };
  const inputsSent = Object.fromEntries(Object.entries(given).filter(([, value]) => value.trim()));
  const words = sentence({
    events,
    filters: sent,
    target,
    targetId: chosen ? targetId : "",
    inputs: inputsSent,
    quietMs: optionsOk ? quietMs : 0,
    cap: optionsOk ? capNumber : 3,
    oneAgent: !!sessionId,
  });

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.triggerSave({
        id: trigger?.id ?? null,
        // Left empty, one made from a preset is called what the preset is, as `ys` calls it.
        name: name.trim() || presets.find((p) => p.id === preset)?.name || null,
        preset,
        workspaceId,
        sessionId: sessionId || null,
        events,
        filters: sent,
        target,
        targetId,
        inputs: inputsSent,
        quietMs,
        cap: capNumber,
        notify,
      });
      await useTriggersStore.getState().refresh();
      onSaved?.(saved.trigger);
      if (saved.untold.length > 0) {
        setUntold({ saved: saved.trigger, agents: saved.untold });
        setBusy(false);
        return;
      }
      onClose();
    } catch (failed) {
      setError(errorMessage(failed));
      setBusy(false);
    }
  }

  async function tell(agents: UntoldAgent[]) {
    setBusy(true);
    setError(null);
    try {
      await ipc.triggerTellAgents(agents.map((agent) => agent.sessionId));
      onClose();
    } catch (failed) {
      setError(errorMessage(failed));
      setBusy(false);
    }
  }

  const choosePreset = (id: string) => {
    const picked = presets.find((p) => p.id === id);
    setPreset(picked?.id ?? null);
    if (!picked) return;
    setEvents(picked.events);
    setFilters({});
    setTarget("workflow");
    setTargetId(picked.workflow);
    setAnswers({});
    setQuiet(null);
    setName("");
  };
  const toggle = (id: TriggerEvent, on: boolean) => {
    // What it listens for is the user's own now.
    setPreset(null);
    setEvents((all) =>
      on
        ? EVENTS.map((e) => e.id).filter((e) => e === id || all.includes(e))
        : all.filter((e) => e !== id),
    );
  };
  const patch = (next: Partial<TriggerFilters>) => setFilters((all) => ({ ...all, ...next }));
  const setKind = (next: ScheduleTarget) => {
    setPreset(null);
    setTarget(next);
    setTargetId("");
    setAnswers({});
  };
  // One trigger is about one kind of thing: an agent session, the pull request, or the workspace.
  const subject = events[0] ? subjectOf(events[0]) : null;
  const placeholder =
    presets.find((p) => p.id === preset)?.name ?? workflow?.name ?? test?.name ?? "";

  if (untold) {
    const many = untold.agents.length > 1;
    return (
      <div className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 pt-[8vh]">
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby={titleId}
          className="flex w-[34rem] max-w-[calc(100vw-2rem)] flex-col gap-3 rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50"
        >
          <h2 id={titleId} className="text-base font-semibold">
            Tell {many ? "the agents" : "the agent"}?
          </h2>
          <p className="text-ink-muted">
            “{untold.saved.name}” starts when an agent runs <code>ys workspace done</code>. An agent
            is told about that when it starts, and{" "}
            {many
              ? `${untold.agents.length} agents in ${untold.saved.workspaceName} were already running`
              : `the agent ${untold.agents[0]?.title ? `“${untold.agents[0].title}” ` : ""}in ${untold.saved.workspaceName} was already running`}
            . Yardsort can tell {many ? "them" : "it"} now, as soon as {many ? "each is" : "it is"}{" "}
            quiet.
          </p>
          {error && (
            <p role="alert" className="text-red-400 select-text">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <button
              type="button"
              className="px-3 py-1.5 text-ink-muted hover:text-ink"
              disabled={busy}
              onClick={onClose}
            >
              Not now
            </button>
            <button
              type="button"
              className="rounded bg-accent px-4 py-1.5 font-medium text-canvas disabled:opacity-40"
              disabled={busy}
              onClick={() => void tell(untold.agents)}
            >
              Tell {many ? "the agents" : "the agent"}
            </button>
          </div>
        </div>
      </div>
    );
  }

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
        className="flex max-h-[84vh] w-[36rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
      >
        <header className="border-b border-line p-5 pb-4">
          <h2 id={titleId} className="text-base font-semibold">
            {trigger ? `Edit “${trigger.name}”` : "New trigger"}
          </h2>
          <p className="mt-1 text-ink-muted">
            Starts a workflow or a smoke test when something happens, while Yardsort is open.
          </p>
        </header>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5 pt-4">
          {!trigger && presets.length > 0 && (
            <label className="flex flex-col gap-1.5">
              <span className={label}>Start from</span>
              <select
                aria-label="Start from"
                className={control}
                value={preset ?? ""}
                onChange={(event) => choosePreset(event.target.value)}
              >
                <option value="">Nothing: put it together</option>
                {presets.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
          )}

          <fieldset className="flex flex-col gap-2">
            <legend className={label}>Where</legend>
            <select
              aria-label="Workspace"
              className={`${control} mt-1.5`}
              value={workspaceId}
              onChange={(event) => {
                setWorkspaceId(event.target.value);
                setSessionId("");
              }}
            >
              <option value="">Choose…</option>
              {workspaces.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.name}
                </option>
              ))}
            </select>
            <select
              aria-label="Which agent"
              className={control}
              value={sessionId}
              disabled={!workspaceId}
              onChange={(event) => setSessionId(event.target.value)}
            >
              <option value="">Any agent in the workspace</option>
              {agents.map((record) => (
                <option key={record.id} value={record.id}>
                  Only {record.title}
                  {record.running ? "" : " (ended)"}, and its runs talk to it
                </option>
              ))}
            </select>
          </fieldset>

          <fieldset className="flex flex-col gap-2">
            <legend className={label}>When</legend>
            {GROUPS.map((group) => (
              <div key={group} role="group" aria-label={group} className="flex flex-col gap-1">
                <span className="mt-1 text-[12px] text-ink-faint">{group}</span>
                <div className="grid grid-cols-2 gap-x-3 gap-y-1">
                  {EVENTS.filter((e) => e.group === group).map((e) => {
                    const other = subject !== null && subjectOf(e.id) !== subject;
                    return (
                      <label
                        key={e.id}
                        className={`${check} ${other ? "opacity-40" : ""}`}
                        title={
                          other
                            ? "A trigger is about one kind of thing — an agent, the pull request, or the workspace. Make a second trigger for this."
                            : e.id
                        }
                      >
                        <input
                          type="checkbox"
                          checked={has(e.id)}
                          disabled={other}
                          onChange={(event) => toggle(e.id, event.target.checked)}
                        />
                        {e.label}
                      </label>
                    );
                  })}
                </div>
              </div>
            ))}

            {(has("agent.settled") || has("agent.exited") || has("pr.opened") || posts) && (
              <div role="group" aria-label="Only when" className="flex flex-col gap-1">
                <span className="mt-1 text-[12px] text-ink-faint">Only when</span>
                {has("agent.settled") && (
                  <>
                    <label className={check}>
                      <input
                        type="checkbox"
                        checked={!!filters.newCommits}
                        onChange={(event) => patch({ newCommits: event.target.checked })}
                      />
                      The branch has new commits
                    </label>
                    <label className={check}>
                      <input
                        type="checkbox"
                        checked={!!filters.clean}
                        onChange={(event) => patch({ clean: event.target.checked })}
                      />
                      Nothing is uncommitted
                    </label>
                  </>
                )}
                {has("agent.exited") && (
                  <label className={check}>
                    <input
                      type="checkbox"
                      checked={filters.exit === "failed"}
                      onChange={(event) => patch({ exit: event.target.checked ? "failed" : null })}
                    />
                    Its program failed
                  </label>
                )}
                {has("pr.opened") && (
                  <label className={check}>
                    <input
                      type="checkbox"
                      checked={!!filters.drafts}
                      onChange={(event) => patch({ drafts: event.target.checked })}
                    />
                    A draft counts as soon as it opens, not once it is marked ready
                  </label>
                )}
                {posts && (
                  <>
                    <select
                      aria-label="Whose posts count"
                      className={control}
                      value={filters.from ?? "others"}
                      onChange={(event) =>
                        patch({ from: event.target.value === "anyone" ? "anyone" : null })
                      }
                    >
                      <option value="others">
                        Posts by anyone but you: teammates and review bots
                      </option>
                      <option value="anyone">
                        Posts by anyone, yours too, but not while its own run is going
                      </option>
                    </select>
                    <label className={check}>
                      <input
                        type="checkbox"
                        checked={!!filters.noBots}
                        onChange={(event) => patch({ noBots: event.target.checked })}
                      />
                      Leave out bots
                    </label>
                  </>
                )}
                {has("pr.review") && (
                  <div role="group" aria-label="Reviews that" className="flex flex-wrap gap-3">
                    <span className="text-ink-muted">Reviews that:</span>
                    {STATES.map((state) => (
                      <label key={state.id} className={check}>
                        <input
                          type="checkbox"
                          checked={!!filters.states?.includes(state.id)}
                          onChange={(event) =>
                            patch({
                              states: STATES.map((s) => s.id).filter((id) =>
                                id === state.id
                                  ? event.target.checked
                                  : !!filters.states?.includes(id),
                              ),
                            })
                          }
                        />
                        {state.label}
                      </label>
                    ))}
                  </div>
                )}
              </div>
            )}

            {has("test.finished") && (
              <div className="grid grid-cols-2 gap-2">
                <select
                  aria-label="Which smoke test ends"
                  className={control}
                  value={filters.test ?? ""}
                  onChange={(event) => patch({ test: event.target.value || null })}
                >
                  <option value="">Any smoke test</option>
                  {(testsHere ?? []).map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.name}
                    </option>
                  ))}
                </select>
                <select
                  aria-label="With the verdict"
                  className={control}
                  value={filters.verdict ?? "any"}
                  onChange={(event) =>
                    patch({
                      verdict:
                        event.target.value === "any"
                          ? null
                          : (event.target.value as "passed" | "failed"),
                    })
                  }
                >
                  <option value="any">Whatever the verdict</option>
                  <option value="passed">Only when it passes</option>
                  <option value="failed">Only when it fails</option>
                </select>
              </div>
            )}
            {has("workflow.finished") && (
              <div className="grid grid-cols-2 gap-2">
                <select
                  aria-label="Which workflow ends"
                  className={control}
                  value={filters.workflow ?? ""}
                  onChange={(event) => patch({ workflow: event.target.value || null })}
                >
                  <option value="">Any workflow</option>
                  {runnableWorkflows.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.name}
                    </option>
                  ))}
                </select>
                <select
                  aria-label="With the outcome"
                  className={control}
                  value={filters.outcome ?? "any"}
                  onChange={(event) =>
                    patch({
                      outcome:
                        event.target.value === "any"
                          ? null
                          : (event.target.value as "succeeded" | "failed"),
                    })
                  }
                >
                  <option value="any">However it ends</option>
                  <option value="succeeded">Only when it succeeds</option>
                  <option value="failed">Only when it fails</option>
                </select>
              </div>
            )}
          </fieldset>

          <fieldset className="flex flex-col gap-2">
            <legend className={label}>Run</legend>
            <div className="mt-1.5 grid grid-cols-[10rem_1fr] gap-2">
              <select
                aria-label="What to run"
                className={control}
                value={target}
                onChange={(event) => setKind(event.target.value as ScheduleTarget)}
              >
                <option value="workflow">A workflow</option>
                <option value="test">A smoke test</option>
              </select>
              {target === "workflow" ? (
                <select
                  aria-label="Workflow"
                  className={control}
                  value={targetId}
                  onChange={(event) => {
                    setPreset(null);
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
              ) : (
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
              )}
            </div>
          </fieldset>

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

          <div className="grid grid-cols-3 gap-3">
            <label className="flex flex-col gap-1.5">
              <span className={label}>Quiet time</span>
              <span className="flex gap-1">
                <input
                  type="number"
                  min={0}
                  aria-label="Quiet time"
                  title="How long to wait after the last event before firing, so a burst is one run"
                  className={`${control} w-16`}
                  value={quietNow.amount}
                  onChange={(event) => setQuiet({ ...quietNow, amount: event.target.value })}
                />
                <select
                  aria-label="Quiet time unit"
                  className={control}
                  value={quietNow.unit}
                  onChange={(event) => setQuiet({ ...quietNow, unit: event.target.value as Unit })}
                >
                  <option value="seconds">seconds</option>
                  <option value="minutes">minutes</option>
                </select>
              </span>
            </label>
            <label className="flex flex-col gap-1.5">
              <span className={label}>At most</span>
              <input
                type="number"
                min={1}
                aria-label="At most"
                title="How many runs it may start for one agent session, one pull request, or the workspace, before it stops and asks"
                className={control}
                value={cap}
                onChange={(event) => setCap(event.target.value)}
              />
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
          </div>

          <label className="flex flex-col gap-1.5">
            <span className={label}>Name (optional)</span>
            <input
              aria-label="Name"
              className="rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent"
              placeholder={placeholder}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>

          {words && (
            <p
              aria-label="What it will do"
              className="rounded border border-line bg-canvas/40 p-2.5 text-ink"
            >
              {words}
              {workspaceName && <span className="text-ink-muted"> In {workspaceName}.</span>}
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
            {busy ? "Saving…" : trigger ? "Save" : "Add trigger"}
          </button>
        </footer>
      </form>
    </div>
  );
}
