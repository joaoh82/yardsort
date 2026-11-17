import { useShortcutLabel } from "@/stores/preferences";
import { counts, useNotificationsStore } from "@/stores/notifications";
import { useProjectsStore } from "@/stores/projects";

/**
 * At the top of the sidebar: one row that opens the Notifications list in the center panel.
 *
 * Two numbers, because they ask different things of the person: how many things wait for an
 * answer (filled), and how many finished while nobody was looking (outlined).
 */
export function NotificationsRow() {
  const open = useProjectsStore((s) => s.notificationsOpen);
  // The whole list, counted outside the selector, which must hand back a stable value.
  const items = useNotificationsStore((s) => s.items);
  const { needs, finished } = counts(items);
  const shortcut = useShortcutLabel("notifications");

  const said = [
    needs > 0 ? `${needs} ${needs === 1 ? "needs" : "need"} you` : null,
    finished > 0 ? `${finished} finished and not seen` : null,
  ].filter((part) => part !== null);

  return (
    <button
      type="button"
      aria-current={open ? "page" : undefined}
      aria-label={["Notifications", ...said].join(", ")}
      title={`What needs you, and what finished while you were not looking${
        shortcut ? ` (${shortcut})` : ""
      }`}
      onClick={() => useProjectsStore.getState().openNotifications(!open)}
      className={`flex h-10 w-full shrink-0 items-center gap-2 border-b border-line px-3.5 text-left hover:bg-raised ${
        open ? "bg-raised text-ink" : "text-ink-muted"
      }`}
    >
      <NotificationsIcon />
      <span className="min-w-0 flex-1 truncate">Notifications</span>
      {needs > 0 && (
        <span
          aria-hidden
          title={`${needs} ${needs === 1 ? "needs" : "need"} you`}
          className="shrink-0 rounded-full bg-accent px-1.5 text-micro leading-4.5 text-canvas tabular-nums"
        >
          {needs}
        </span>
      )}
      {finished > 0 && (
        <span
          aria-hidden
          title={`${finished} finished and not seen`}
          className="shrink-0 rounded-full border border-accent/50 px-1.5 text-micro leading-4.5 text-accent tabular-nums"
        >
          {finished}
        </span>
      )}
    </button>
  );
}

/** A bell. */
export function NotificationsIcon() {
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
      <path d="M4 6.6a4 4 0 0 1 8 0c0 3.1 1.3 4.2 1.3 4.2H2.7S4 9.7 4 6.6ZM6.6 13a1.5 1.5 0 0 0 2.8 0" />
    </svg>
  );
}
