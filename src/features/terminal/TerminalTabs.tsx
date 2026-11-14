import { useRef, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { ContextMenu, type MenuItem } from "@/features/sidebar/ContextMenu";
import { runActivity, statusWords } from "@/features/smoke/words";
import { errorMessage, ipc, type SessionRecord, type TestRun } from "@/lib/ipc";
import { useShortcutLabel } from "@/stores/preferences";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { launchable, useHarnessStore } from "@/stores/harnesses";
import { AgentButton } from "./AgentButton";
import { BusyClock } from "./BusyClock";
import { handOff } from "./handOff";
import { ConversationsDialog } from "./ConversationsDialog";
import { StatusDot } from "./StatusDot";

export function TerminalTabs({ workspaceId }: { workspaceId: string }) {
  const newKey = useShortcutLabel("newTerminal");
  const allTabs = useTerminalStore((s) => s.tabs);
  const activeId = useTerminalStore((s) => s.active[workspaceId]);
  const open = useTerminalStore((s) => s.open);
  const tabs = allTabs.filter((tab) => tab.workspaceId === workspaceId);
  const harnesses = launchable(useHarnessStore((s) => s.harnesses));
  // Smoke test runs open as tabs here too, after the terminals; one in front hides them.
  const openRuns = useSmokeTestsStore((s) => s.open[workspaceId]);
  const shownRun = useSmokeTestsStore((s) => s.shown[workspaceId]);
  const runs = useSmokeTestsStore((s) => s.runs);

  return (
    <div className="flex h-10 shrink-0 items-stretch border-b border-line bg-surface">
      <div role="tablist" aria-label="Terminals" className="flex min-w-0 items-stretch">
        {tabs.map((tab) => (
          <Tab key={tab.id} tab={tab} active={tab.id === activeId && !shownRun} />
        ))}
        {(openRuns ?? []).map((runId) => {
          const run = runs.find((candidate) => candidate.id === runId);
          return (
            run && (
              <RunTab key={runId} run={run} workspaceId={workspaceId} active={runId === shownRun} />
            )
          );
        })}
      </div>
      <button
        type="button"
        title={`New shell (${newKey})`}
        aria-label="New shell"
        onClick={() => void open(workspaceId)}
        className="px-3 text-ink-muted hover:bg-raised hover:text-ink"
      >
        +
      </button>
      <div className="ml-auto flex items-center gap-1 pr-2">
        <HandOff workspaceId={workspaceId} />
        {harnesses.map((harness) => (
          <AgentButton
            key={harness.id}
            workspaceId={workspaceId}
            harness={harness}
            iconSize={12}
            className="flex items-center gap-1.5 rounded px-2 py-0.5 text-small text-ink-faint hover:bg-raised hover:text-ink"
          />
        ))}
      </div>
    </div>
  );
}

/** A smoke test run's tab: the test's name with a dot for how the run is going. */
function RunTab({
  run,
  workspaceId,
  active,
}: {
  run: TestRun;
  workspaceId: string;
  active: boolean;
}) {
  const openRun = useSmokeTestsStore((s) => s.openRun);
  const closeRun = useSmokeTestsStore((s) => s.closeRun);
  return (
    <div
      className={`group flex items-center border-r border-line ${
        active ? "bg-canvas text-ink" : "text-ink-muted hover:bg-raised"
      }`}
    >
      <button
        type="button"
        role="tab"
        aria-selected={active}
        aria-label={`Smoke test ${run.testName}, ${statusWords(run)}`}
        onClick={() => openRun(workspaceId, run.id)}
        title={`Smoke test run: ${statusWords(run)}`}
        className="flex h-full items-center gap-2 pr-1 pl-3"
      >
        <StatusDot activity={runActivity(run)} />
        <span className="max-w-40 truncate">{run.testName}</span>
        <span className="text-small text-ink-faint">{statusWords(run)}</span>
      </button>
      <button
        type="button"
        aria-label={`Close ${run.testName}`}
        onClick={() => closeRun(workspaceId, run.id)}
        className="mr-1 rounded px-1 text-ink-faint opacity-0 group-hover:opacity-100 hover:bg-line hover:text-ink focus-visible:opacity-100"
      >
        ×
      </button>
    </div>
  );
}

function Tab({ tab, active }: { tab: TerminalTab; active: boolean }) {
  const activateTerminal = useTerminalStore((s) => s.activate);
  const showTerminals = useSmokeTestsStore((s) => s.showTerminals);
  const activate = (id: TerminalTab["id"]) => {
    activateTerminal(id);
    showTerminals(tab.workspaceId);
  };
  const close = useTerminalStore((s) => s.close);
  const fork = useTerminalStore((s) => s.fork);
  const record = useSessionsStore((s) =>
    s.byWorkspace[tab.workspaceId]?.find((candidate) => candidate.id === tab.recordId),
  );
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [showing, setShowing] = useState(false);
  // Only a conversation has a record to keep a label in; a shell is what it runs.
  const [labelling, setLabelling] = useState(false);
  const label = record?.label || "";
  const name = label || tab.title;
  const status = tab.exit ? (tab.exit.success ? "ended" : `exited ${tab.exit.code}`) : null;
  const activity = tab.exit
    ? tab.exit.success
      ? "idle"
      : "failed"
    : tab.busy
      ? "busy"
      : "waiting";

  const items: MenuItem[] = [
    ...(tab.recordId
      ? [
          {
            label: label ? "Rename label…" : "Label…",
            disabled: !record,
            onSelect: () => setLabelling(true),
          },
          {
            label: "Fork this conversation",
            disabled: !record?.forkable,
            onSelect: () => void fork(tab.recordId!),
          },
          {
            label: "Triggers…",
            onSelect: () =>
              useProjectsStore
                .getState()
                .openTriggers(true, { workspaceId: tab.workspaceId, sessionId: tab.recordId! }),
          },
        ]
      : []),
    { label: "Close", onSelect: () => void close(tab.id) },
  ];

  return (
    <div
      className={`group flex items-center border-r border-line ${
        active ? "bg-canvas text-ink" : "text-ink-muted hover:bg-raised"
      }`}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuAt({ x: event.clientX, y: event.clientY });
      }}
    >
      <button
        type="button"
        role="tab"
        onKeyDown={(event) => {
          if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
            event.preventDefault();
            const box = event.currentTarget.getBoundingClientRect();
            setMenuAt({ x: box.left, y: box.bottom });
          }
        }}
        aria-selected={active}
        aria-label={label ? `${label} (${record?.harnessLabel ?? tab.title})` : undefined}
        onClick={() => activate(tab.id)}
        onDoubleClick={() => record && setLabelling(true)}
        title={
          [label && record?.harnessLabel, record?.title, record && "Double-click to label"]
            .filter(Boolean)
            .join(" — ") || undefined
        }
        className="flex h-full items-center gap-2 pr-1 pl-3"
      >
        <StatusDot activity={activity} attention={tab.attention} />
        {record && <HarnessIcon id={record.harnessId} label={record.harnessLabel} size={14} />}
        {!(labelling && record) && <span className="max-w-40 truncate">{name}</span>}
        {status && <span className="text-small text-ink-faint">{status}</span>}
        {!tab.exit && tab.busySince !== null && <BusyClock since={tab.busySince} />}
      </button>
      {labelling && record && (
        <LabelInput
          initial={label}
          placeholder={tab.title}
          onDone={(wanted) => {
            setLabelling(false);
            if (wanted !== null && wanted !== label) void saveLabel(record, wanted);
          }}
        />
      )}
      {tab.toldAbout && (
        <button
          type="button"
          aria-label={`Told about ${tab.toldAbout}: show`}
          title={`Told it can read ${tab.toldAbout}. Show what it reads.`}
          onClick={() => setShowing(true)}
          className="rounded px-1 text-small text-ink-faint hover:bg-line hover:text-ink"
        >
          ⇄
        </button>
      )}
      <button
        type="button"
        aria-label={`Close ${name}`}
        onClick={() => void close(tab.id)}
        className="mr-1 rounded px-1 text-ink-faint opacity-0 group-hover:opacity-100 hover:bg-line hover:text-ink focus-visible:opacity-100"
      >
        ×
      </button>
      {menuAt && <ContextMenu at={menuAt} items={items} onClose={() => setMenuAt(null)} />}
      {showing && (
        <ConversationsDialog
          workspaceId={tab.workspaceId}
          except={tab.recordId}
          onClose={() => setShowing(false)}
        />
      )}
    </div>
  );
}

/** Keep a conversation's label in its record, and show the record as the core now has it. */
async function saveLabel(record: SessionRecord, label: string) {
  const sessions = useSessionsStore.getState();
  try {
    await ipc.sessionSetLabel(record.id, label);
  } catch (error) {
    useTerminalStore.setState({ error: errorMessage(error) });
  }
  await sessions.load(record.workspaceId);
}

/**
 * The label being typed, in place of the tab's name. Enter or leaving keeps it, Escape keeps
 * what was there; empty clears it, and the agent's name comes back.
 */
function LabelInput({
  initial,
  placeholder,
  onDone,
}: {
  initial: string;
  placeholder: string;
  onDone: (label: string | null) => void;
}) {
  const [value, setValue] = useState(initial);
  const finished = useRef(false);
  const finish = (label: string | null) => {
    if (finished.current) return;
    finished.current = true;
    onDone(label === null ? null : label.trim());
  };
  return (
    <input
      aria-label="Label"
      autoFocus
      value={value}
      placeholder={placeholder}
      maxLength={40}
      spellCheck={false}
      onFocus={(event) => event.currentTarget.select()}
      onChange={(event) => setValue(event.target.value)}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Enter") finish(value);
        else if (event.key === "Escape") finish(null);
      }}
      onBlur={() => finish(value)}
      className="mr-1 h-6 w-36 rounded border border-accent bg-canvas px-1 text-ink outline-none select-text"
    />
  );
}

/**
 * Start another agent here with what Yardsort recorded about the workspace as its first
 * message. The packet opens in the composer, where it is read and edited before anything is
 * sent; the agent is chosen there too. Nothing of the last agent's conversation is in it.
 */
function HandOff({ workspaceId }: { workspaceId: string }) {
  const [busy, setBusy] = useState(false);
  const start = async () => {
    setBusy(true);
    await handOff(workspaceId);
    setBusy(false);
  };
  return (
    <button
      type="button"
      disabled={busy}
      title="Start another agent here, with what Yardsort recorded about this workspace as its first message — to read and edit before it is sent"
      onClick={() => void start()}
      className="mr-1 shrink-0 rounded border border-line px-2 py-0.5 text-small whitespace-nowrap text-ink-faint hover:border-accent hover:text-ink disabled:opacity-40"
    >
      Hand off…
    </button>
  );
}
