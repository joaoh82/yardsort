import { useProjectsStore } from "@/stores/projects";
import { useSmokeTestsStore, waitingTotal } from "@/stores/smokeTests";

/**
 * Under Workflows in the sidebar: one row that opens every project's smoke tests in the center
 * panel.
 *
 * The number is how many runs are waiting for the user to approve them, across every workspace —
 * the one thing the tests need a person for. It reaches zero when nothing is waiting.
 */
export function TestsRow() {
  const open = useProjectsStore((s) => s.testsOpen);
  // The whole list, counted outside the selector, which must hand back a stable value.
  const runs = useSmokeTestsStore((s) => s.runs);
  const count = waitingTotal(runs);

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={
        count === 0
          ? "Tests"
          : `Tests, ${count} ${count === 1 ? "run" : "runs"} waiting for approval`
      }
      title="Every project's smoke tests and their runs"
      onClick={() => useProjectsStore.getState().openTests(!open)}
      className={`flex h-10 w-full shrink-0 items-center gap-2 border-b border-line px-3.5 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <TestsIcon />
      <span className="min-w-0 flex-1 truncate">Tests</span>
      {count > 0 && (
        <span
          aria-hidden
          title={`${count} waiting for approval`}
          className="shrink-0 rounded-full border border-accent/50 px-1.5 text-micro leading-4.5 text-accent tabular-nums"
        >
          {count}
        </span>
      )}
    </button>
  );
}

/** A flask: the smoke-test mark. */
export function TestsIcon() {
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
      <path d="M6 1.8h4" />
      <path d="M6.6 1.8v4.2L3 12.2a1.3 1.3 0 0 0 1.1 2h7.8a1.3 1.3 0 0 0 1.1-2L9.4 6V1.8" />
      <path d="M5 9.6h6" />
    </svg>
  );
}
