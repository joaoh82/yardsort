import type { HarnessState } from "./activity";

/**
 * How far a workspace's agents have got, spelled out on its row in the sidebar so that nothing
 * has to be opened to find out: a count while they wait for you, a tick once they have ended, a
 * cross if one of them fell over. Working says nothing — the dot is already pulsing, and a row
 * that lights up only when it wants something is a row worth glancing at.
 */
export function HarnessBadge({ state, attention }: { state: HarnessState; attention?: boolean }) {
  if (state.kind === "none" || state.kind === "working") return null;

  const { text, label, colour } =
    state.kind === "waiting"
      ? {
          text: String(state.count),
          label: attention
            ? `${state.count} waiting for you, not seen yet`
            : `${state.count} waiting for you`,
          colour: attention ? "bg-accent text-canvas" : "bg-accent/20 text-accent",
        }
      : state.kind === "failed"
        ? { text: "✗", label: "exited with an error", colour: "text-red-400" }
        : { text: "✓", label: "finished", colour: "text-ink-faint" };

  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`shrink-0 rounded px-1 text-[10px] leading-4 font-medium tabular-nums ${colour}`}
    >
      {text}
    </span>
  );
}
