import { useNow } from "@/lib/ticker";

/** "12s", then "4m", then "1h 5m": short enough for a tab, and still once a minute after one. */
export function elapsedWords(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
}

/**
 * How long something has been printing, counting up from `since` (epoch ms). It takes the place
 * of a pulsing dot: text that changes once a second at most, where an endless animation keeps
 * the whole window repainting at the screen's refresh rate.
 */
export function BusyClock({ since }: { since: number }) {
  const now = useNow();
  const words = elapsedWords(now - since);
  return (
    <span
      aria-label={`busy for ${words}`}
      className="shrink-0 text-[11px] text-ink-faint tabular-nums"
    >
      {words}
    </span>
  );
}
