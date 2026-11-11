import { useEffect, useState } from "react";
import { clockTime } from "@/features/schedules/words";
import { timeAgo } from "@/features/terminal/sessionText";
import type { TriggerFire, TriggerItem } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { useTriggersStore } from "@/stores/triggers";
import { useWorkflowStore } from "@/stores/workflows";
import { TriggerDialog } from "./TriggerDialog";
import {
  durationWords,
  fireColour,
  fireWords,
  standingColour,
  standingWords,
  subjectOf,
  targetWords,
} from "./words";

const EMPTY: TriggerFire[] = [];
const quiet =
  "rounded border border-line px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40";

/**
 * Every project's triggers, in the center panel: what each listens for, where, what it runs and
 * how its last fire went. A trigger opens to its history, with **Edit**, **Reset** when it is
 * capped, **Remove**, and a switch. Opened from a workspace's menu or an agent tab's, it shows
 * that workspace's or that agent's triggers, and a new one starts there.
 *
 * It holds nothing of its own but what is selected: triggers are the core's, which fires them
 * while the app is open. See `docs/design/36-triggers.md`.
 */
export function TriggersView() {
  const items = useTriggersStore((s) => s.items);
  const loaded = useTriggersStore((s) => s.loaded);
  const error = useTriggersStore((s) => s.error);
  const projects = useProjectsStore((s) => s.projects);
  const focus = useProjectsStore((s) => s.triggersFocus);
  const [selected, setSelected] = useState<string | null>(null);
  // The dialog: closed, a new trigger, or the one being changed.
  const [editing, setEditing] = useState<TriggerItem | "new" | null>(null);

  // Ages are from now, and now moves: once a minute is as often as a row changes.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    void useTriggersStore.getState().load();
  }, []);

  const shown = focus
    ? items.filter(
        (item) =>
          item.workspaceId === focus.workspaceId &&
          (!focus.sessionId || item.sessionId === focus.sessionId),
      )
    : items;
  const current = shown.find((item) => item.id === selected) ?? null;
  const projectName = (id: string) => projects.find((p) => p.id === id)?.name ?? "";
  const focusWorkspace = focus
    ? projects.flatMap((p) => p.workspaces).find((w) => w.id === focus.workspaceId)
    : undefined;

  return (
    <section aria-label="Triggers" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2">
        <h1 className="text-small font-semibold tracking-wider text-ink-muted uppercase">
          Triggers
        </h1>
        {focus && (
          <span className="flex min-w-0 items-center gap-2 text-ink-muted">
            <span className="truncate">
              {focus.sessionId ? "One agent's, in " : "In "}
              {focusWorkspace?.name ?? "a workspace"}
            </span>
            <button
              type="button"
              onClick={() => useProjectsStore.getState().openTriggers(true)}
              className="shrink-0 text-detail text-accent hover:underline"
            >
              Show all
            </button>
          </span>
        )}
        <button
          type="button"
          onClick={() => setEditing("new")}
          className="ml-auto rounded bg-accent px-3 py-1.5 font-medium text-canvas"
        >
          New trigger…
        </button>
        <button
          type="button"
          aria-label="Close triggers"
          title="Close"
          onClick={() => useProjectsStore.getState().openTriggers(false)}
          className="size-7 rounded text-ink-muted hover:bg-raised hover:text-ink"
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
          {shown.length === 0 ? (
            <p role="status" className="p-4 text-ink-faint">
              {!loaded
                ? "Loading triggers…"
                : focus
                  ? "No triggers here yet. A trigger starts a workflow or a smoke test when something happens: the agent says it is done, a review comes in, the checks fail."
                  : "No triggers yet. A trigger starts a workflow or a smoke test when something happens in a workspace — the agent says it is done, a review comes in, the checks fail — while Yardsort is open."}
            </p>
          ) : (
            <ul aria-label="Triggers">
              {shown.map((item) => (
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
                        className={`shrink-0 rounded-full px-1.5 text-micro ${standingColour(item)}`}
                      >
                        {standingWords(item)}
                      </span>
                    </div>
                    <div className="truncate text-detail text-ink-muted">{item.when}</div>
                    <div className="truncate text-detail text-ink-faint">
                      {targetWords(item)} · {projectName(item.projectId)} / {item.workspaceName}
                      {item.sessionId && ` · ${item.sessionTitle ?? "an agent"}'s tab`}
                    </div>
                    {item.last && (
                      <div className="text-detail text-ink-faint">
                        Last: {fireWords(item.last)},{" "}
                        {timeAgo(item.last.decidedAt ?? item.last.firstAt, now)}
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
            <TriggerDetail
              key={current.id}
              trigger={current}
              projectName={projectName(current.projectId)}
              onEdit={() => setEditing(current)}
              onClose={() => setSelected(null)}
            />
          </div>
        )}
      </div>

      {editing && (
        <TriggerDialog
          trigger={editing === "new" ? undefined : editing}
          workspaceId={focus?.workspaceId}
          sessionId={focus?.sessionId}
          onClose={() => setEditing(null)}
          onSaved={(saved) => setSelected(saved.id)}
        />
      )}
    </section>
  );
}

/** What a trigger counts its runs for: "pull request", "agent session", "workspace". */
function kindOf(trigger: TriggerItem): string {
  const subject = trigger.events[0] ? subjectOf(trigger.events[0]) : "workspace";
  return subject === "session" ? "agent session" : subject;
}

/** What a capped trigger stopped for, in words: "#142", "the agent session". */
function subjectWords(trigger: TriggerItem): string {
  const kind = kindOf(trigger);
  return kind === "pull request" && trigger.watch?.number
    ? `#${trigger.watch.number}`
    : `the ${kind}`;
}

function TriggerDetail({
  trigger,
  projectName,
  onEdit,
  onClose,
}: {
  trigger: TriggerItem;
  projectName: string;
  onEdit: () => void;
  onClose: () => void;
}) {
  const history = useTriggersStore((s) => s.history[trigger.id]) ?? EMPTY;
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState(false);

  useEffect(() => {
    void useTriggersStore.getState().loadHistory(trigger.id);
  }, [trigger.id]);

  const act = async (action: () => Promise<boolean>) => {
    setBusy(true);
    try {
      return await action();
    } finally {
      setBusy(false);
    }
  };
  const { setEnabled, reset, remove } = useTriggersStore.getState();

  /** Go to the run a line started: a test's tab in its workspace, or the workflow's own view. */
  const openRun = (fire: TriggerFire) => {
    if (!fire.runId) return;
    if (trigger.target === "test") {
      if (!trigger.workspaceId) return;
      useProjectsStore.getState().select(trigger.workspaceId);
      useSmokeTestsStore.getState().openRun(trigger.workspaceId, fire.runId);
    } else {
      useWorkflowStore.getState().showRun(trigger.targetId, fire.runId);
      useProjectsStore.getState().openWorkflow(trigger.targetId);
    }
  };
  const canOpen = trigger.target === "workflow" || trigger.workspaceId !== null;
  const per = subjectWords(trigger);
  // Capped, though a run it started before may still be going: Reset is for either.
  const capped = trigger.capped;
  const watch = trigger.watch;

  return (
    <div className="flex min-h-full flex-col gap-4 p-4">
      <header className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-base font-semibold">{trigger.name}</h2>
          <p className="text-ink-muted">
            Runs the {targetWords(trigger)} in {projectName} / {trigger.workspaceName}
          </p>
        </div>
        <button
          type="button"
          aria-label="Close trigger"
          title="Close"
          onClick={onClose}
          className="size-7 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        <label className="mr-2 flex items-center gap-2">
          <input
            type="checkbox"
            role="switch"
            aria-label={`${trigger.name} is on`}
            checked={trigger.enabled}
            disabled={busy}
            onChange={(event) => void act(() => setEnabled(trigger.id, event.target.checked))}
          />
          <span className="text-ink-muted">{trigger.enabled ? "On" : "Off"}</span>
        </label>
        {capped && (
          <button
            type="button"
            className={quiet}
            disabled={busy}
            title="The runs it started stop counting against its cap, so it starts runs again."
            onClick={() => void act(() => reset(trigger.id))}
          >
            Reset
          </button>
        )}
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
          aria-label="Remove this trigger?"
          className="flex flex-col gap-2 rounded border border-line bg-raised p-3"
        >
          <p>Remove “{trigger.name}” and its history? The runs it started stay where they are.</p>
          <div className="flex gap-2">
            <button
              type="button"
              className="rounded bg-red-500/80 px-3 py-1.5 font-medium text-white disabled:opacity-40"
              disabled={busy}
              onClick={() =>
                void act(() => remove(trigger.id)).then((removed) => removed && onClose())
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

      {trigger.standing === "capped" && (
        <p role="status" className="text-red-400">
          It has started all {trigger.cap} of the runs it may for {per}, and fires nothing more for
          it until you press Reset. Look at what the runs did first: a trigger that keeps firing may
          be answering itself.
        </p>
      )}
      {trigger.standing === "refused" && (
        <p role="status" className="text-red-400 select-text">
          Its last fire could not start a run
          {trigger.last?.note ? `: ${trigger.last.note}` : "."}
        </p>
      )}
      {trigger.standing === "no_workspace" && (
        <p role="status" className="text-red-400">
          Its workspace, {trigger.workspaceName}, was deleted, so it has nothing to listen to.
        </p>
      )}
      {trigger.standing === "awaiting_approval" && (
        <p role="status" className="text-amber-300">
          Its run is waiting for you to approve the test file, which is new or has changed. Open the
          run below to look at it.
        </p>
      )}
      {trigger.standing === "waiting" && (
        <p role="status" className="text-amber-300">
          A run of the same {trigger.target === "test" ? "test" : "workflow"} is going in{" "}
          {trigger.workspaceName}; what it gathered is decided when that run ends.
        </p>
      )}
      {trigger.sessionId && !trigger.sessionRunning && trigger.standing !== "no_workspace" && (
        <p role="status" className="text-ink-faint">
          The agent it listens to has ended, so it has nothing to hear until that conversation is
          resumed.
        </p>
      )}

      <dl className="grid grid-cols-[8rem_1fr] gap-x-3 gap-y-1">
        <dt className="text-ink-faint">When</dt>
        <dd>{trigger.when}</dd>
        <dt className="text-ink-faint">Where</dt>
        <dd>
          {trigger.sessionId
            ? `${trigger.sessionTitle ?? "An agent"}'s tab, in ${trigger.workspaceName}`
            : trigger.workspaceName}
        </dd>
        {watch && (
          <>
            <dt className="text-ink-faint">Pull request</dt>
            <dd className="select-text">
              {watch.problem
                ? `Could not look: ${watch.problem}`
                : watch.number && !watch.closed
                  ? `Watching #${watch.number}`
                  : watch.number
                    ? `#${watch.number} is closed; watching for the branch's next one`
                    : "None yet. One opening is news."}
            </dd>
          </>
        )}
        {trigger.filters.elsewhere && (
          <>
            <dt className="text-ink-faint">Pull requests</dt>
            <dd className="select-text">
              {trigger.watchingElsewhere.length === 0 ? (
                "None tracked elsewhere yet. One tracked is news."
              ) : (
                <ul>
                  {trigger.watchingElsewhere.map((tracked) => (
                    <li key={tracked.url}>
                      {tracked.problem
                        ? `${tracked.name}: could not look: ${tracked.problem}`
                        : tracked.closed
                          ? `${tracked.name} is closed`
                          : `Watching ${tracked.name}`}
                    </li>
                  ))}
                </ul>
              )}
            </dd>
          </>
        )}
        <dt className="text-ink-faint">Quiet time</dt>
        <dd>
          {trigger.quietMs > 0
            ? `${durationWords(trigger.quietMs)} after the last event`
            : "None: it fires at once"}
        </dd>
        <dt className="text-ink-faint">At most</dt>
        <dd>
          {trigger.cap === 1 ? "One run" : `${trigger.cap} runs`} per {kindOf(trigger)}
        </dd>
        <dt className="text-ink-faint">Tells you</dt>
        <dd>
          {trigger.notify === "always"
            ? "How every run ends"
            : trigger.notify === "never"
              ? "Never"
              : "When a run fails"}
        </dd>
        {Object.entries(trigger.inputs).map(([id, value]) => (
          <div key={id} className="contents">
            <dt className="truncate text-ink-faint">{id}</dt>
            <dd className="select-text">{value}</dd>
          </div>
        ))}
      </dl>

      <section aria-label="History">
        <h3 className="mb-1 text-small font-semibold tracking-wider text-ink-faint uppercase">
          History
        </h3>
        {history.length === 0 ? (
          <p className="text-ink-faint">Nothing has happened that it listens for yet.</p>
        ) : (
          <ul className="flex flex-col">
            {history.map((fire) => (
              <li key={fire.id} className="flex items-baseline gap-2 py-1">
                <span className={`shrink-0 rounded-full px-1.5 text-micro ${fireColour(fire)}`}>
                  {fireWords(fire)}
                </span>
                <span className="shrink-0 text-detail text-ink-muted tabular-nums">
                  {clockTime(fire.decidedAt ?? fire.firstAt)}
                </span>
                <span className="shrink-0 font-mono text-small text-ink-faint">
                  {fire.event}
                  {fire.events > 1 && ` ×${fire.events}`}
                </span>
                <span className="min-w-0 flex-1 truncate text-detail text-ink-faint select-text">
                  {fire.note ?? fire.summary ?? ""}
                </span>
                {fire.runId && (
                  <button
                    type="button"
                    disabled={!canOpen}
                    onClick={() => openRun(fire)}
                    className="shrink-0 text-detail text-accent hover:underline disabled:opacity-40"
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
