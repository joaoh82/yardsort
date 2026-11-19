import { useEffect, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { SchedulesIcon } from "@/features/schedules/SchedulesRow";
import { timeAgo } from "@/features/terminal/sessionText";
import { TestsIcon } from "@/features/tests/TestsRow";
import { TriggersIcon } from "@/features/triggers/TriggersRow";
import { WorkflowsIcon } from "@/features/workflows/WorkflowsRow";
import type { NotificationItem } from "@/lib/ipc";
import { counts, rowKey, useNotificationsStore, type Grouping } from "@/stores/notifications";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore } from "@/stores/terminals";
import { useWorkflowStore } from "@/stores/workflows";
import {
  about,
  chip,
  detailName,
  leadName,
  opens,
  sections,
  wentBadly,
  what,
  type Filter,
  type Section,
} from "./words";

const GROUPINGS: { id: Grouping; label: string }[] = [
  { id: "kind", label: "What it is" },
  { id: "project", label: "Project" },
];

/**
 * The Notifications list, in the center panel: what waits for an answer, what finished while
 * nobody was looking, and what was answered or seen in the last week.
 *
 * The core keeps the list and says when a row leaves. A row here is a way to what it is about:
 * pressing it opens the agent's terminal, the workflow's run, or the view a schedule or a
 * trigger is changed in, and looking at a terminal or a run is what marks its row seen.
 *
 * Rows of the other computers this one reaches are in the same list, each naming its computer.
 * One opens its workspace there, and marking it seen is said there.
 */
export function NotificationsView() {
  const items = useNotificationsStore((s) => s.items);
  const loaded = useNotificationsStore((s) => s.loaded);
  const error = useNotificationsStore((s) => s.error);
  const grouping = useNotificationsStore((s) => s.grouping);
  const [filter, setFilter] = useState<Filter>("all");

  // Ages are from now, and now moves: once a minute is as often as a row changes.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    void useNotificationsStore.getState().load();
  }, []);

  const { needs, finished } = counts(items);
  const tabs: { id: Filter; label: string; count: number | null }[] = [
    { id: "all", label: "All", count: needs + finished },
    { id: "needs", label: "Needs you", count: needs },
    { id: "finished", label: "Finished", count: finished },
    { id: "earlier", label: "Earlier", count: null },
  ];
  const shown = sections(items, grouping, filter);
  const nothingWaits = needs + finished === 0;

  return (
    <section aria-label="Notifications" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-4 py-2">
        <h1 className="text-small font-semibold tracking-wider text-ink-muted uppercase">
          Notifications
        </h1>
        <div role="tablist" aria-label="Show" className="flex gap-1">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={filter === tab.id}
              aria-label={tab.count ? `${tab.label}, ${tab.count}` : tab.label}
              onClick={() => setFilter(tab.id)}
              className={`rounded-full px-3 py-1.5 whitespace-nowrap ${
                filter === tab.id ? "bg-raised text-ink" : "text-ink-muted hover:text-ink"
              }`}
            >
              {tab.label}
              {tab.count !== null && tab.count > 0 && (
                <span
                  aria-hidden
                  className="ml-1.5 font-mono text-small text-ink-faint tabular-nums"
                >
                  {tab.count}
                </span>
              )}
            </button>
          ))}
        </div>
        <button
          type="button"
          disabled={finished === 0}
          title="Move everything that finished to Earlier"
          onClick={() => void useNotificationsStore.getState().seenAll()}
          className="ml-auto rounded border border-line px-2 py-1 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40"
        >
          Mark all seen
        </button>
        <button
          type="button"
          aria-label="Close notifications"
          title="Close"
          onClick={() => useProjectsStore.getState().openNotifications(false)}
          className="size-7 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>
      <div
        role="radiogroup"
        aria-label="Group by"
        className="flex shrink-0 items-center gap-1 border-b border-line px-4 py-1.5 text-small"
      >
        <span aria-hidden className="mr-1 text-ink-faint">
          Group by
        </span>
        {GROUPINGS.map((choice) => (
          <button
            key={choice.id}
            type="button"
            role="radio"
            aria-checked={grouping === choice.id}
            onClick={() => useNotificationsStore.getState().setGrouping(choice.id)}
            className={`rounded px-2 py-1 ${
              grouping === choice.id ? "bg-raised text-ink" : "text-ink-faint hover:text-ink"
            }`}
          >
            {choice.label}
          </button>
        ))}
      </div>

      {error && (
        <p role="alert" className="shrink-0 border-b border-line px-4 py-2 text-red-400">
          {error}
          <button
            type="button"
            onClick={() => useNotificationsStore.getState().dismissError()}
            className="ml-3 text-ink-muted hover:text-ink"
          >
            Dismiss
          </button>
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loaded && nothingWaits && filter !== "earlier" && (
          <div className="border-b border-line px-4 py-5">
            <p className="text-ink-muted">
              Nothing needs you, and nothing finished while you were away.
            </p>
            <p className="mt-1 text-small text-ink-faint">
              Agents that ask, finish or stop with an error show up here. So do workflows that wait
              for you or end, smoke tests waiting for approval, and schedules and triggers that
              could not start.
            </p>
          </div>
        )}
        {loaded && filter === "earlier" && shown.length === 0 && (
          <p className="px-4 py-5 text-ink-muted">Nothing was answered or seen in the last week.</p>
        )}
        {shown.map((section) => (
          <SectionRows key={section.id} section={section} now={now} />
        ))}
        {shown.length > 0 && (
          <p className="px-4 py-3 text-small text-ink-faint">
            A row opens what it is about. Looking at an agent&apos;s terminal, or at a
            workflow&apos;s run, is what marks its row seen.
          </p>
        )}
      </div>
    </section>
  );
}

function SectionRows({ section, now }: { section: Section; now: number }) {
  return (
    <section aria-label={section.title}>
      <h2 className="flex h-8 items-center gap-2 border-b border-line bg-surface px-4 text-small">
        <span className="font-semibold tracking-wider text-ink-muted uppercase">
          {section.title}
        </span>
        <span className="font-mono text-ink-faint tabular-nums">{section.items.length}</span>
        <span className="ml-auto text-ink-faint">{section.note}</span>
      </h2>
      <ul>
        {section.items.map((item) => (
          <Row key={rowKey(item)} item={item} namesProject={section.namesProject} now={now} />
        ))}
      </ul>
    </section>
  );
}

/** Open what the row is about. An agent's is its workspace, with its terminal in front when it
 *  still has one; a workflow's is the run; a smoke test's is its workspace, where its approval
 *  card is; a schedule's or a trigger's is the view it is changed in. Looking at a terminal or
 *  a run is what marks its row seen. */
function open(item: NotificationItem) {
  const projects = useProjectsStore.getState();
  const kind = about(item);
  if (item.hostId !== null) {
    // On another computer: its workspace there, with the agent's terminal in front. What a
    // workflow, a schedule or a trigger there waits for is answered on that computer.
    if (!item.workspaceId) return;
    if (kind === "agent" && item.terminalId)
      useOtherComputersStore.getState().activate(item.hostId, item.workspaceId, item.terminalId);
    projects.selectRemote(item.hostId, item.workspaceId);
    return;
  }
  if (kind === "workflow" && item.workflowId && item.subjectId) {
    useWorkflowStore.getState().showRun(item.workflowId, item.subjectId);
    projects.openWorkflow(item.workflowId);
    return;
  }
  if (kind === "schedule") {
    projects.openSchedules(true);
    return;
  }
  if (kind === "trigger") {
    projects.openTriggers(true, item.workspaceId ? { workspaceId: item.workspaceId } : undefined);
    return;
  }
  if (!item.workspaceId) return;
  projects.select(item.workspaceId);
  const terminals = useTerminalStore.getState();
  if (item.terminalId && terminals.tabs.some((tab) => tab.id === item.terminalId))
    terminals.activate(item.terminalId);
}

/** The mark of what a row is about: the agent's own, or the one its view has in the sidebar. */
function AboutIcon({ item }: { item: NotificationItem }) {
  switch (about(item)) {
    case "agent":
      return <HarnessIcon id={item.harnessId ?? ""} label={item.harnessId ?? "agent"} />;
    case "workflow":
      return <WorkflowsIcon />;
    case "test":
      return <TestsIcon />;
    case "schedule":
      return <SchedulesIcon />;
    case "trigger":
      return <TriggersIcon />;
  }
}

function Row(props: { item: NotificationItem; namesProject: boolean; now: number }) {
  const { item, namesProject, now } = props;
  const words = what(item);
  const code = chip(item);
  const left = item.part === "earlier";
  const name = leadName(item);
  const detail = detailName(item);
  return (
    <li className="group flex min-h-15 items-center gap-3 border-b border-line px-4 hover:bg-raised">
      <Mark item={item} />
      <span className="flex shrink-0 text-ink-muted">
        <AboutIcon item={item} />
      </span>
      <button
        type="button"
        onClick={() => open(item)}
        aria-label={`${name} in ${item.projectName}${item.hostId !== null ? ` on ${item.hostName ?? "another computer"}` : ""}: ${words}`}
        title={`Open ${opens(item)}`}
        className="min-w-0 flex-1 py-2 text-left"
      >
        <span className="flex min-w-0 items-baseline gap-2">
          <span className={`truncate font-medium ${left ? "text-ink-muted" : ""}`}>{name}</span>
          {item.hostId !== null && (
            <span className="shrink-0 rounded bg-raised px-1.5 text-small text-ink-muted group-hover:bg-line">
              on {item.hostName ?? "another computer"}
            </span>
          )}
          {namesProject && (
            <span className="shrink-0 text-small text-ink-muted">{item.projectName}</span>
          )}
          {item.branch && (
            <span className="shrink-0 font-mono text-small text-ink-faint">{item.branch}</span>
          )}
          {detail && <span className="truncate text-small text-ink-faint">· {detail}</span>}
        </span>
        <span className="mt-0.5 flex min-w-0 items-baseline gap-2">
          <span className={`shrink-0 ${left ? "text-ink-faint" : "text-ink-muted"}`}>{words}</span>
          {code && (
            <code className="truncate rounded bg-raised px-1.5 font-mono text-detail text-ink-muted group-hover:bg-line">
              {code}
            </code>
          )}
          {item.said && <span className="truncate text-ink-faint">“{item.said}”</span>}
          {item.note && <span className="truncate text-ink-faint">{item.note}</span>}
        </span>
      </button>
      {item.part === "finished" && (
        <button
          type="button"
          title="Move it to Earlier without opening it"
          onClick={() => void useNotificationsStore.getState().seen([rowKey(item)])}
          className="shrink-0 rounded border border-line px-2 py-0.5 text-small text-ink-muted opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 hover:bg-canvas hover:text-ink focus-visible:opacity-100"
        >
          Mark seen
        </button>
      )}
      <time
        dateTime={new Date(item.at).toISOString()}
        className="shrink-0 font-mono text-small text-ink-faint"
      >
        {timeAgo(item.at, now)}
      </time>
    </li>
  );
}

/** How a row under Earlier left its part: a question was answered, a finished thing was seen,
 *  and a schedule or a trigger stopped needing anyone. */
function leftAs(item: NotificationItem): string {
  if (item.kind === "agent.asks") return "answered";
  switch (about(item)) {
    case "agent":
      return "seen";
    case "workflow":
      return item.kind === "workflow.succeeded" || item.kind === "workflow.failed"
        ? "seen"
        : "answered";
    case "test":
      return "answered";
    case "schedule":
    case "trigger":
      return "no longer waiting";
  }
}

/** The dot, in the sidebar's own language: solid amber waits for you, ringed is finished and
 *  not seen, red went badly, grey has left. */
function Mark({ item }: { item: NotificationItem }) {
  const label =
    item.part === "earlier"
      ? leftAs(item)
      : item.part === "needs"
        ? "needs you"
        : wentBadly(item)
          ? "went badly, not seen yet"
          : "finished, not seen yet";
  const look =
    item.part === "earlier"
      ? "bg-line"
      : item.part === "needs"
        ? "bg-accent"
        : wentBadly(item)
          ? "bg-red-400"
          : "bg-accent ring-2 ring-accent/60 ring-offset-1 ring-offset-canvas";
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`size-2 shrink-0 rounded-full ${look}`}
    />
  );
}
