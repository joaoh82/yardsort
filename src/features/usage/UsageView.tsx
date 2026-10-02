import { useState } from "react";
import { errorMessage } from "@/lib/ipc";
import { useAppStore } from "@/stores/app";
import { useProjectsStore } from "@/stores/projects";
import { MachineResources } from "./MachineResources";
import { TokenUsage } from "./TokenUsage";

type Tab = "tokens" | "machine";

/** Remembered for the session: coming back to Usage opens the tab last looked at. */
let lastTab: Tab = "tokens";

/**
 * Usage, in the center panel: the tokens the agents spent, and what Yardsort and the agents use
 * of this machine. Opened from the foot of the sidebar or the command palette.
 */
export function UsageView() {
  const [tab, setTab] = useState<Tab>(lastTab);
  const show = useAppStore((s) => s.showUsageInSidebar);
  const [saveError, setSaveError] = useState<string | null>(null);
  const choose = (next: Tab) => {
    lastTab = next;
    setTab(next);
  };
  const tabClass = (which: Tab) =>
    `rounded-full px-3 py-1 whitespace-nowrap ${
      tab === which ? "bg-raised text-ink" : "text-ink-muted hover:text-ink"
    }`;

  return (
    <section aria-label="Usage" className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-line px-4 py-2">
        <div role="tablist" aria-label="Usage" className="flex gap-1">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "tokens"}
            onClick={() => choose("tokens")}
            className={tabClass("tokens")}
          >
            Token usage
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "machine"}
            onClick={() => choose("machine")}
            className={tabClass("machine")}
          >
            Machine resources
          </button>
        </div>
        <label
          className="ml-auto flex items-center gap-2 whitespace-nowrap text-ink-muted"
          title="When it is hidden, Usage is still in the command palette."
        >
          <input
            type="checkbox"
            checked={show}
            onChange={(event) => {
              setSaveError(null);
              useAppStore
                .getState()
                .setShowUsageInSidebar(event.target.checked)
                .catch((reason: unknown) => setSaveError(errorMessage(reason)));
            }}
            className="accent-(--color-accent)"
          />
          Show Usage in the sidebar
        </label>
        <button
          type="button"
          aria-label="Close usage"
          title="Close"
          onClick={() => useProjectsStore.getState().openUsage(false)}
          className="ml-2 size-6 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>
      {saveError && (
        <p role="alert" className="border-b border-line px-4 py-2 text-red-400 select-text">
          {saveError}
        </p>
      )}
      <div role="tabpanel" className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {tab === "tokens" ? <TokenUsage /> : <MachineResources />}
      </div>
    </section>
  );
}
