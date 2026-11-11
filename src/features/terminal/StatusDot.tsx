import type { Activity } from "./activity";

/**
 * What is going on somewhere, at a glance:
 * solid = running — printing right now when a [`BusyClock`] counts beside it, otherwise quiet
 * (probably waiting for you) · ringed = finished a long stretch of work you have not looked at
 * yet · red = exited with an error.
 *
 * Nothing here animates. An endless CSS animation keeps the webview drawing a frame every refresh
 * of the screen, and on Linux each of those frames is the whole window: one pulsing dot cost the
 * app a third of a CPU core.
 */
export function StatusDot({ activity, attention }: { activity: Activity; attention?: boolean }) {
  const colour = {
    busy: "bg-accent",
    waiting: "bg-accent",
    idle: "bg-line",
    failed: "bg-red-400",
  }[activity];
  const label = attention ? "finished, not seen yet" : activity;
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`size-2 shrink-0 rounded-full ${colour} ${
        attention ? "ring-2 ring-accent/60 ring-offset-1 ring-offset-surface" : ""
      }`}
    />
  );
}
