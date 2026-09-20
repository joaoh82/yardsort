import type { FileReview, ReviewFlag } from "@/lib/ipc";
import { useAssistStore } from "@/stores/assist";

/**
 * What Assist found, on the change list: a badge per file, and one line above the list saying
 * when it last looked. Everything here is advisory — the change list reads the same with it all
 * missing.
 */

const FLAGS: Record<ReviewFlag, { label: string; title: string; tone: string }> = {
  credentialsFile: {
    label: "credentials",
    title: "This file's name says it holds credentials. Its contents were not sent to TypeSafe.",
    tone: "border-red-400/40 text-red-300",
  },
  secret: {
    label: "secret",
    title: "The change looks like it adds a literal secret — a key, token or password.",
    tone: "border-red-400/40 text-red-300",
  },
  weakensTests: {
    label: "tests",
    title: "The change looks like it removes, skips or weakens a test.",
    tone: "border-amber-400/40 text-amber-300",
  },
  disablesChecks: {
    label: "checks",
    title: "The change looks like it switches a lint, type check or CI step off.",
    tone: "border-amber-400/40 text-amber-300",
  },
};

const OFF_TASK = {
  label: "off-task",
  title: "This file looks unrelated to what this workspace was asked to do.",
  tone: "border-amber-400/40 text-amber-300",
};

/** The badges for one file, or nothing at all when Assist had nothing to say about it. */
export function AssistBadges({ review }: { review: FileReview | undefined }) {
  if (!review) return null;
  const badges = [
    ...(review.relevance === "unrelated" ? [OFF_TASK] : []),
    ...review.flags.map((flag) => FLAGS[flag]),
  ];
  if (badges.length === 0) return null;
  return (
    <span className="flex shrink-0 gap-1">
      {badges.map((badge) => (
        <span
          key={badge.label}
          title={`Assist: ${badge.title}`}
          className={`rounded border px-1 text-[10px] leading-4 ${badge.tone}`}
        >
          {badge.label}
        </span>
      ))}
    </span>
  );
}

/** One line above the change list: what Assist is doing, or what stopped it. */
export function AssistNote() {
  const review = useAssistStore((s) => s.review);
  const reviewing = useAssistStore((s) => s.reviewing);
  const error = useAssistStore((s) => s.error);
  const status = useAssistStore((s) => s.status);
  if (!status?.reviewChanges) return null;

  const flagged = review?.files.filter(
    (file) => file.flags.length > 0 || file.relevance === "unrelated",
  ).length;

  return (
    <p className="flex items-center gap-2 px-3 py-1 text-[11px] text-ink-faint">
      <span className="min-w-0 flex-1 truncate">
        {error ? (
          <span role="alert" className="text-amber-400">
            Assist: {error}
          </span>
        ) : reviewing ? (
          "Assist is looking at these changes…"
        ) : review === null ? (
          "Assist checks these changes shortly after an agent stops writing."
        ) : review.task === null ? (
          `Checked for risky edits. Nothing on record about this workspace's task, so nothing is checked against it.`
        ) : flagged === 0 ? (
          "Assist found nothing to flag."
        ) : (
          `Assist flagged ${flagged} of ${review.files.length} files.`
        )}
      </span>
      <button
        type="button"
        disabled={reviewing}
        onClick={() => void useAssistStore.getState().reviewNow()}
        className="shrink-0 rounded px-1 hover:bg-raised hover:text-ink disabled:opacity-40"
      >
        Check now
      </button>
    </p>
  );
}
