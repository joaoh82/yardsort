import { useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { ContextMenu, type MenuItem } from "@/features/sidebar/ContextMenu";
import { useShortcutLabel } from "@/stores/preferences";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { launchable, useHarnessStore } from "@/stores/harnesses";
import { AgentButton } from "./AgentButton";
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

  return (
    <div className="flex h-9 shrink-0 items-stretch border-b border-line bg-surface">
      <div role="tablist" aria-label="Terminals" className="flex min-w-0 items-stretch">
        {tabs.map((tab) => (
          <Tab key={tab.id} tab={tab} active={tab.id === activeId} />
        ))}
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
            className="flex items-center gap-1.5 rounded px-2 py-0.5 text-[11px] text-ink-faint hover:bg-raised hover:text-ink"
          />
        ))}
      </div>
    </div>
  );
}

function Tab({ tab, active }: { tab: TerminalTab; active: boolean }) {
  const activate = useTerminalStore((s) => s.activate);
  const close = useTerminalStore((s) => s.close);
  const fork = useTerminalStore((s) => s.fork);
  const record = useSessionsStore((s) =>
    s.byWorkspace[tab.workspaceId]?.find((candidate) => candidate.id === tab.recordId),
  );
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [showing, setShowing] = useState(false);
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
            label: "Fork this conversation",
            disabled: !record?.forkable,
            onSelect: () => void fork(tab.recordId!),
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
        onClick={() => activate(tab.id)}
        title={record?.title || undefined}
        className="flex h-full items-center gap-2 pr-1 pl-3"
      >
        <StatusDot activity={activity} attention={tab.attention} />
        {record && <HarnessIcon id={record.harnessId} label={record.harnessLabel} size={12} />}
        <span className="max-w-40 truncate">{tab.title}</span>
        {status && <span className="text-[11px] text-ink-faint">{status}</span>}
      </button>
      {tab.toldAbout && (
        <button
          type="button"
          aria-label={`Told about ${tab.toldAbout}: show`}
          title={`Told it can read ${tab.toldAbout}. Show what it reads.`}
          onClick={() => setShowing(true)}
          className="rounded px-1 text-[11px] text-ink-faint hover:bg-line hover:text-ink"
        >
          ⇄
        </button>
      )}
      <button
        type="button"
        aria-label={`Close ${tab.title}`}
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
      className="mr-1 shrink-0 rounded border border-line px-2 py-0.5 text-[11px] whitespace-nowrap text-ink-faint hover:border-accent hover:text-ink disabled:opacity-40"
    >
      Hand off…
    </button>
  );
}
