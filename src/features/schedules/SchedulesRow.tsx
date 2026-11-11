import { useProjectsStore } from "@/stores/projects";
import { needingAttention, useSchedulesStore } from "@/stores/schedules";

/**
 * Under Tests in the sidebar: one row that opens every project's schedules in the center panel.
 *
 * The number is how many schedules need the user before they can do their job: a run waiting
 * for approval, a time that was refused, a workspace that is gone. It reaches zero when every
 * schedule can fire.
 */
export function SchedulesRow() {
  const open = useProjectsStore((s) => s.schedulesOpen);
  // The whole list, counted outside the selector, which must hand back a stable value.
  const items = useSchedulesStore((s) => s.items);
  const count = needingAttention(items);

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={
        count === 0
          ? "Schedules"
          : `Schedules, ${count} ${count === 1 ? "needs" : "need"} your attention`
      }
      title="Workflows and smoke tests that run at a set time or on a routine"
      onClick={() => useProjectsStore.getState().openSchedules(!open)}
      className={`flex h-10 w-full shrink-0 items-center gap-2 border-b border-line px-3.5 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <SchedulesIcon />
      <span className="min-w-0 flex-1 truncate">Schedules</span>
      {count > 0 && (
        <span
          aria-hidden
          title={`${count} ${count === 1 ? "needs" : "need"} your attention`}
          className="shrink-0 rounded-full border border-accent/50 px-1.5 text-micro leading-4.5 text-accent tabular-nums"
        >
          {count}
        </span>
      )}
    </button>
  );
}

/** A clock: the schedule mark. */
export function SchedulesIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-4 shrink-0"
    >
      <circle cx="8" cy="8" r="6" />
      <path d="M8 4.6V8l2.3 1.5" />
    </svg>
  );
}
