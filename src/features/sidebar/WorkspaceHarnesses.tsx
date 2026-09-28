import { HoverCard } from "@/lib/HoverCard";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { HarnessBadge } from "@/features/terminal/HarnessBadge";
import { harnessState } from "@/features/terminal/activity";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { useProjectsStore } from "@/stores/projects";

export function WorkspaceHarnesses({
  tabs,
  workspaceId,
}: {
  tabs: TerminalTab[];
  workspaceId: string;
}) {
  const records = useSessionsStore((s) => s.byWorkspace[workspaceId]);
  const harnesses = tabs.filter((tab) => tab.recordId !== null);
  const firstTab = harnesses[0];
  if (!firstTab) return null;
  const content = (
    <div className="space-y-3">
      <p className="font-medium">Open harnesses · {harnesses.length}</p>
      <ul className="space-y-1">
        {harnesses.map((tab) => {
          const record = records?.find((record) => record.id === tab.recordId);
          return (
            <li key={tab.id}>
              <button
                type="button"
                onClick={() => {
                  useProjectsStore.getState().select(workspaceId);
                  useTerminalStore.getState().activate(tab.id);
                }}
                className="flex w-full items-center gap-2 rounded px-2 py-2 text-left hover:bg-line"
              >
                <HarnessIcon
                  id={record?.harnessId ?? tab.title}
                  label={record?.harnessLabel ?? tab.title}
                />
                <span className="min-w-0 flex-1 truncate">{record?.harnessLabel ?? tab.title}</span>
                <span className={tab.exit && !tab.exit.success ? "text-red-400" : "text-ink-faint"}>
                  {tab.exit
                    ? tab.exit.success
                      ? "Finished"
                      : "Failed"
                    : tab.busy
                      ? "Working"
                      : "Waiting"}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
  const first = records?.find((record) => record.id === firstTab.recordId);
  return (
    <HoverCard label="Workspace harnesses" content={content} side="right" className="shrink-0">
      <button
        type="button"
        aria-label={`${harnesses.length} open harnesses`}
        className={`flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] hover:brightness-125 ${tabs.some((tab) => tab.attention) ? "bg-accent text-canvas" : harnesses.some((tab) => !tab.exit && !tab.busy) ? "bg-accent/15 text-accent" : "bg-canvas text-ink-muted"}`}
      >
        <HarnessIcon
          id={first?.harnessId ?? firstTab.title}
          label={first?.harnessLabel ?? firstTab.title}
          size={12}
        />
        {harnesses.length}
        <span className={harnesses.some((tab) => !tab.exit) ? "sr-only" : undefined}>
          <HarnessBadge state={harnessState(tabs)} attention={tabs.some((tab) => tab.attention)} />
        </span>
      </button>
    </HoverCard>
  );
}
