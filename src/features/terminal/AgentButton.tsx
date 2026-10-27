import { useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import { ipc, type HarnessInfo } from "@/lib/ipc";
import { useTerminalStore } from "@/stores/terminals";
import { handOff } from "./handOff";
import { bareHarness } from "./quickLaunch";

/**
 * Start an agent here with no first message. Beside other conversations, in a project that
 * shares them, the agent is told it can read them: as background context when its harness takes
 * some, which the core does on its own. One that takes none could only be told in a first
 * message, so the button asks — the handoff in the composer, or a fresh start — and only then.
 * Alt-click starts it without being told.
 */
export function AgentButton({
  workspaceId,
  harness,
  className,
  iconSize,
}: {
  workspaceId: string;
  harness: HarnessInfo;
  className: string;
  iconSize?: number;
}) {
  const open = useTerminalStore((s) => s.open);
  const [askAt, setAskAt] = useState<{ x: number; y: number } | null>(null);
  const start = (skipContext = false) =>
    void open(
      workspaceId,
      skipContext ? { ...bareHarness(harness.id), skipContext } : bareHarness(harness.id),
    );

  const click = async (event: React.MouseEvent<HTMLButtonElement>) => {
    if (event.altKey) return start(true);
    const box = event.currentTarget.getBoundingClientRect();
    if ((harness.contextArgs ?? []).length > 0) return start();
    const context = await ipc.conversationContext(workspaceId, null).catch(() => null);
    if (!context) return start();
    setAskAt({ x: box.left, y: box.bottom });
  };

  return (
    <>
      <button
        type="button"
        title={`Start ${harness.label} here (Alt-click: without telling it about the other conversations)`}
        onClick={(event) => void click(event)}
        className={className}
      >
        <HarnessIcon id={harness.id} label={harness.label} size={iconSize} />
        {harness.id}
      </button>
      {askAt && (
        <ContextMenu
          at={askAt}
          items={[
            {
              label: "Start with the handoff…",
              onSelect: () => void handOff(workspaceId, harness.id),
            },
            { label: "Start fresh", onSelect: () => start(true) },
          ]}
          onClose={() => setAskAt(null)}
        />
      )}
    </>
  );
}
