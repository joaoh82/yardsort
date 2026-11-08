import { useProjectsStore } from "@/stores/projects";
import { needingAttention, useTriggersStore } from "@/stores/triggers";

/**
 * Under Schedules in the sidebar: one row that opens every project's triggers in the center
 * panel.
 *
 * The number is how many triggers need the user before they can do their job: one that is
 * capped, one whose last fire was refused, a run waiting for approval, a workspace that is gone.
 */
export function TriggersRow() {
  const open = useProjectsStore((s) => s.triggersOpen);
  // The whole list, counted outside the selector, which must hand back a stable value.
  const items = useTriggersStore((s) => s.items);
  const count = needingAttention(items);

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={
        count === 0
          ? "Triggers"
          : `Triggers, ${count} ${count === 1 ? "needs" : "need"} your attention`
      }
      title="Workflows and smoke tests that start when something happens"
      onClick={() => useProjectsStore.getState().openTriggers(!open)}
      className={`flex h-9 w-full shrink-0 items-center gap-2 border-b border-line px-3 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <TriggersIcon />
      <span className="min-w-0 flex-1 truncate">Triggers</span>
      {count > 0 && (
        <span
          aria-hidden
          title={`${count} ${count === 1 ? "needs" : "need"} your attention`}
          className="shrink-0 rounded-full border border-accent/50 px-1.5 text-[10px] leading-4 text-accent tabular-nums"
        >
          {count}
        </span>
      )}
    </button>
  );
}

/** A bolt: the trigger mark. */
export function TriggersIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-3.5 shrink-0"
    >
      <path d="M9 1.8 3.8 9h4l-1 5.2L12.2 7h-4z" />
    </svg>
  );
}
