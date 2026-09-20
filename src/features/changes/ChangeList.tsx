import { useMemo } from "react";
import type { ChangeSet, FileChange, Scope } from "@/lib/ipc";
import { useAssistStore } from "@/stores/assist";
import { useChangesStore } from "@/stores/changes";
import { AssistBadges, AssistNote } from "./AssistBadges";

const KIND: Record<FileChange["kind"], { letter: string; label: string; colour: string }> = {
  added: { letter: "A", label: "Added", colour: "text-green-400" },
  modified: { letter: "M", label: "Modified", colour: "text-accent" },
  deleted: { letter: "D", label: "Deleted", colour: "text-red-400" },
  renamed: { letter: "R", label: "Renamed", colour: "text-blue-400" },
  untracked: { letter: "U", label: "Untracked", colour: "text-green-400" },
  conflicted: { letter: "!", label: "Conflicted", colour: "text-red-400" },
};

export function ChangeList({ changes }: { changes: ChangeSet }) {
  if (changes.uncommitted.length === 0 && changes.committed.length === 0) {
    return <p className="p-3 text-ink-faint">No changes.</p>;
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto pb-2">
      <AssistNote />
      <Group title="Uncommitted" scope="uncommitted" files={changes.uncommitted} />
      {changes.base && (
        <Group
          title={`On this branch · vs ${changes.base}`}
          scope="committed"
          files={changes.committed}
        />
      )}
    </div>
  );
}

function Group({ title, scope, files }: { title: string; scope: Scope; files: FileChange[] }) {
  const viewing = useChangesStore((s) => s.viewing);
  const view = useChangesStore((s) => s.view);
  const review = useAssistStore((s) => s.review);
  // One lookup per group; a selector that built this map would rebuild it on every render.
  const judged = useMemo(
    () =>
      new Map(
        (review?.files ?? [])
          .filter((file) => file.scope === scope)
          .map((file) => [file.path, file] as const),
      ),
    [review, scope],
  );
  if (files.length === 0) return null;

  return (
    <section aria-label={title}>
      <h3 className="sticky top-0 flex items-center justify-between bg-surface px-3 py-1.5 text-[11px] font-semibold tracking-wider text-ink-faint uppercase">
        {title}
        <span className="font-normal">{files.length}</span>
      </h3>
      <ul>
        {files.map((change) => {
          const kind = KIND[change.kind];
          const selected =
            viewing?.kind === "diff" &&
            viewing.scope === scope &&
            viewing.change.path === change.path;
          const slash = change.path.lastIndexOf("/");
          return (
            <li key={change.path}>
              <button
                type="button"
                aria-current={selected}
                title={change.oldPath ? `${change.oldPath} → ${change.path}` : change.path}
                onClick={() => void view({ kind: "diff", change, scope })}
                className="flex h-7 w-full items-center gap-2 px-3 text-left hover:bg-raised aria-[current=true]:bg-raised"
              >
                <span
                  className={`w-3 shrink-0 text-center font-mono text-[11px] ${kind.colour}`}
                  title={kind.label}
                >
                  {kind.letter}
                </span>
                <span className={`truncate ${change.kind === "deleted" ? "line-through" : ""}`}>
                  {change.path.slice(slash + 1)}
                </span>
                <span className="min-w-0 flex-1 truncate text-[11px] text-ink-faint" dir="rtl">
                  {slash > 0 ? change.path.slice(0, slash) : ""}
                </span>
                <AssistBadges review={judged.get(change.path)} />
                <span className="shrink-0 font-mono text-[11px]">
                  {change.additions != null && (
                    <span className="text-green-400">+{change.additions}</span>
                  )}{" "}
                  {change.deletions != null && (
                    <span className="text-red-400">−{change.deletions}</span>
                  )}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
