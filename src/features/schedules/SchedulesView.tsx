import { useEffect, useState } from "react";
import { timeAgo } from "@/features/terminal/sessionText";
import type { Schedule, ScheduleRun } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useSchedulesStore } from "@/stores/schedules";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { ScheduleDialog } from "./ScheduleDialog";
import {
  clockTime,
  lineColour,
  lineWords,
  standingColour,
  standingWords,
  targetWords,
} from "./words";

const EMPTY: ScheduleRun[] = [];
const quiet =
  "rounded border border-line px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40";

/**
 * Every project's schedules, in the center panel: what each runs, where, when, when it fires
 * next and how its last time went. A schedule opens to its history, with **Run now**, **Edit**
 * and **Remove**, and a switch that keeps its place while it is off.
 *
 * It holds nothing of its own but what is selected: schedules are the core's, which fires them
 * while the app is open. See `docs/design/34-schedules.md`.
 */
export function SchedulesView() {
  const items = useSchedulesStore((s) => s.items);
  const loaded = useSchedulesStore((s) => s.loaded);
  const error = useSchedulesStore((s) => s.error);
  const projects = useProjectsStore((s) => s.projects);
  const [selected, setSelected] = useState<string | null>(null);
  // The dialog: closed, a new schedule, or the one being changed.
  const [editing, setEditing] = useState<Schedule | "new" | null>(null);

  // Ages and "next" are from now, and now moves: once a minute is as often as a row changes.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    void useSchedulesStore.getState().load();
  }, []);

  const current = items.find((item) => item.id === selected) ?? null;
  const projectName = (id: string) => projects.find((p) => p.id === id)?.name ?? "";

  return (
    <section aria-label="Schedules" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2">
        <h1 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          Schedules
        </h1>
        <button
          type="button"
          onClick={() => setEditing("new")}
          className="ml-auto rounded bg-accent px-3 py-1 font-medium text-canvas"
        >
          New schedule…
        </button>
        <button
          type="button"
          aria-label="Close schedules"
          title="Close"
          onClick={() => useProjectsStore.getState().openSchedules(false)}
          className="size-6 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>

      {error && (
        <p
          role="alert"
          className="shrink-0 border-b border-line px-4 py-1.5 text-red-400 select-text"
        >
          {error}
        </p>
      )}

      <div className="flex min-h-0 flex-1">
        <div
          className={`min-h-0 overflow-y-auto ${current ? "w-80 shrink-0 border-r border-line" : "flex-1"}`}
        >
          {items.length === 0 ? (
            <p role="status" className="p-4 text-ink-faint">
              {loaded
                ? "No schedules yet. A schedule runs a workflow or a smoke test at a set time, or again and again, while Yardsort is open."
                : "Loading schedules…"}
            </p>
          ) : (
            <ul aria-label="Schedules">
              {items.map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    aria-current={selected === item.id ? "true" : undefined}
                    onClick={() => setSelected(selected === item.id ? null : item.id)}
                    className={`flex w-full flex-col gap-0.5 border-b border-line px-4 py-2 text-left hover:bg-raised ${
                      selected === item.id ? "bg-raised" : ""
                    }`}
                  >
                    <div className="flex items-baseline gap-2">
                      <span className="min-w-0 flex-1 truncate font-medium text-ink">
                        {item.name}
                      </span>
                      <span
                        className={`shrink-0 rounded-full px-1.5 text-[10px] ${standingColour(item.standing)}`}
                      >
                        {standingWords(item.standing)}
                      </span>
                    </div>
                    <div className="flex items-center gap-2 text-[12px] text-ink-faint">
                      <span className="truncate">
                        {targetWords(item)} · {projectName(item.projectId)} / {item.workspaceName}
                      </span>
                    </div>
                    <div className="flex items-center gap-2 text-[12px] text-ink-muted">
                      <span className="truncate">{item.when}</span>
                      {item.enabled && item.nextAt !== null && (
                        <span className="shrink-0 text-ink-faint tabular-nums">
                          · next {clockTime(item.nextAt)}
                        </span>
                      )}
                    </div>
                    {item.last && (
                      <div className="text-[12px] text-ink-faint">
                        Last: {lineWords(item.last)}, {timeAgo(item.last.firedAt, now)}
                      </div>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
        {current && (
          <div className="min-h-0 flex-1 overflow-y-auto">
            <ScheduleDetail
              key={current.id}
              schedule={current}
              projectName={projectName(current.projectId)}
              onEdit={() => setEditing(current)}
              onClose={() => setSelected(null)}
            />
          </div>
        )}
      </div>

      {editing && (
        <ScheduleDialog
          schedule={editing === "new" ? undefined : editing}
          onClose={() => setEditing(null)}
          onSaved={(saved) => setSelected(saved.id)}
        />
      )}
    </section>
  );
}

function ScheduleDetail({
  schedule,
  projectName,
  onEdit,
  onClose,
}: {
  schedule: Schedule;
  projectName: string;
  onEdit: () => void;
  onClose: () => void;
}) {
  const history = useSchedulesStore((s) => s.history[schedule.id]) ?? EMPTY;
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState(false);

  useEffect(() => {
    void useSchedulesStore.getState().loadHistory(schedule.id);
  }, [schedule.id]);

  const act = async (action: () => Promise<boolean>) => {
    setBusy(true);
    try {
      return await action();
    } finally {
      setBusy(false);
    }
  };
  const { setEnabled, runNow, remove } = useSchedulesStore.getState();

  /** Go to the run a line started: a test's tab in its workspace, or the workflow's own view. */
  const openRun = (run: ScheduleRun) => {
    if (!run.runId) return;
    if (schedule.target === "test") {
      if (!schedule.workspaceId) return;
      useProjectsStore.getState().select(schedule.workspaceId);
      useSmokeTestsStore.getState().openRun(schedule.workspaceId, run.runId);
    } else {
      useProjectsStore.getState().openWorkflow(schedule.targetId);
    }
  };
  const canOpen = schedule.target === "workflow" || schedule.workspaceId !== null;

  return (
    <div className="flex min-h-full flex-col gap-4 p-4">
      <header className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-base font-semibold">{schedule.name}</h2>
          <p className="text-ink-muted">
            Runs the {targetWords(schedule)} in {projectName} / {schedule.workspaceName}
          </p>
        </div>
        <button
          type="button"
          aria-label="Close schedule"
          title="Close"
          onClick={onClose}
          className="size-6 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        <label className="mr-2 flex items-center gap-2">
          <input
            type="checkbox"
            role="switch"
            aria-label={`${schedule.name} is on`}
            checked={schedule.enabled}
            disabled={busy}
            onChange={(event) => void act(() => setEnabled(schedule.id, event.target.checked))}
          />
          <span className="text-ink-muted">{schedule.enabled ? "On" : "Off"}</span>
        </label>
        <button
          type="button"
          className={quiet}
          disabled={busy || schedule.workspaceId === null}
          title="Start its run now. Its next time stays where it is."
          onClick={() => void act(() => runNow(schedule.id))}
        >
          Run now
        </button>
        <button type="button" className={quiet} disabled={busy} onClick={onEdit}>
          Edit…
        </button>
        <button type="button" className={quiet} disabled={busy} onClick={() => setRemoving(true)}>
          Remove…
        </button>
      </div>

      {removing && (
        <div
          role="alertdialog"
          aria-label="Remove this schedule?"
          className="flex flex-col gap-2 rounded border border-line bg-raised p-3"
        >
          <p>Remove “{schedule.name}” and its history? The runs it started stay where they are.</p>
          <div className="flex gap-2">
            <button
              type="button"
              className="rounded bg-red-500/80 px-3 py-1 font-medium text-white disabled:opacity-40"
              disabled={busy}
              onClick={() =>
                void act(() => remove(schedule.id)).then((removed) => removed && onClose())
              }
            >
              Remove
            </button>
            <button type="button" className={quiet} onClick={() => setRemoving(false)}>
              Keep it
            </button>
          </div>
        </div>
      )}

      <dl className="grid grid-cols-[8rem_1fr] gap-x-3 gap-y-1">
        <dt className="text-ink-faint">When</dt>
        <dd>{schedule.when}</dd>
        <dt className="text-ink-faint">Next</dt>
        <dd className="tabular-nums">
          {schedule.enabled && schedule.nextAt !== null ? clockTime(schedule.nextAt) : "—"}
        </dd>
        <dt className="text-ink-faint">If missed</dt>
        <dd>{schedule.missed === "skip" ? "Skips it" : "Runs once when Yardsort next opens"}</dd>
        <dt className="text-ink-faint">Tells you</dt>
        <dd>
          {schedule.notify === "always"
            ? "How every run ends"
            : schedule.notify === "never"
              ? "Never"
              : "When a run fails"}
        </dd>
        {Object.entries(schedule.inputs).map(([id, value]) => (
          <div key={id} className="contents">
            <dt className="truncate text-ink-faint">{id}</dt>
            <dd className="select-text">{value}</dd>
          </div>
        ))}
      </dl>

      {schedule.standing === "no_workspace" && (
        <p role="status" className="text-red-400">
          Its workspace, {schedule.workspaceName}, was deleted, so it has nowhere to run. Edit it to
          pick another.
        </p>
      )}
      {schedule.standing === "missed" && (
        <p role="status" className="text-ink-faint">
          Its time passed while Yardsort was not running, and it skips those, so it never ran. Edit
          it to give it a new time.
        </p>
      )}
      {schedule.standing === "awaiting_approval" && (
        <p role="status" className="text-amber-300">
          Its run is waiting for you to approve the test file, which is new or has changed. Open the
          run below to look at it.
        </p>
      )}

      <section aria-label="History">
        <h3 className="mb-1 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
          History
        </h3>
        {history.length === 0 ? (
          <p className="text-ink-faint">It has not come due yet.</p>
        ) : (
          <ul className="flex flex-col">
            {history.map((run) => (
              <li key={run.id} className="flex items-baseline gap-2 py-1">
                <span className={`shrink-0 rounded-full px-1.5 text-[10px] ${lineColour(run)}`}>
                  {lineWords(run)}
                </span>
                <span className="shrink-0 text-[12px] text-ink-muted tabular-nums">
                  {clockTime(run.firedAt)}
                </span>
                <span className="min-w-0 flex-1 truncate text-[12px] text-ink-faint select-text">
                  {run.manual ? "Run now" : (run.note ?? "")}
                </span>
                {run.runId && (
                  <button
                    type="button"
                    disabled={!canOpen}
                    onClick={() => openRun(run)}
                    className="shrink-0 text-[12px] text-accent hover:underline disabled:opacity-40"
                  >
                    Open run
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
