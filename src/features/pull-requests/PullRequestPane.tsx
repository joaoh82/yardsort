import { useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import { selectWorkspace } from "@/features/keyboard/commands";
import type { MergeMethod } from "@/lib/ipc";
import { usePullRequestsStore, type Target } from "@/stores/pullRequests";
import { Summary } from "./Summary";
import { mergeBlocked } from "./appearance";
import { age, type Row } from "./rows";

const methods: { method: MergeMethod; label: string }[] = [
  { method: "squash", label: "Squash and merge" },
  { method: "merge", label: "Create a merge commit" },
  { method: "rebase", label: "Rebase and merge" },
];

const button =
  "h-7 rounded border border-line px-2.5 whitespace-nowrap hover:bg-raised disabled:opacity-40 disabled:hover:bg-transparent";
const quiet = "size-6 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink";

/**
 * One pull request, beside the list: who and when, what can be done to it, and its Summary —
 * description, checks, reviewers, conversation. The Code of the design arrives in a later slice,
 * and the two become tabs then.
 */
export function PullRequestPane({
  row,
  now,
  listHidden,
  onToggleList,
  onClose,
}: {
  row: Row;
  now: number;
  listHidden: boolean;
  onToggleList: () => void;
  onClose: () => void;
}) {
  const { pr, project } = row;
  const error = usePullRequestsStore((s) => s.error);
  const notice = usePullRequestsStore((s) => s.notice);

  return (
    <section
      aria-label={`Pull request #${pr.number}`}
      className="flex h-full min-h-0 flex-col bg-surface"
    >
      <header className="shrink-0 border-b border-line px-4 py-2">
        <div className="flex items-center gap-2">
          <p className="min-w-0 flex-1 truncate text-ink-muted">
            <span className="text-ink">{project.name}</span>
            {" · "}
            {pr.author ?? "ghost"}
            {pr.createdAt !== null && <> opened it {age(pr.createdAt, now)} ago</>}
          </p>
          <button
            type="button"
            aria-label={listHidden ? "Show the list" : "Hide the list"}
            title={listHidden ? "Show the list" : "Hide the list"}
            aria-pressed={listHidden}
            onClick={onToggleList}
            className={quiet}
          >
            <span aria-hidden>{listHidden ? "⇥" : "⇤"}</span>
          </button>
          <button
            type="button"
            aria-label="Close details"
            title="Close (Esc)"
            onClick={onClose}
            className={quiet}
          >
            ×
          </button>
        </div>
        <Actions row={row} />
        {(error || notice) && (
          <div className="mt-2 flex items-start gap-2">
            <p
              role={error ? "alert" : "status"}
              className={`min-w-0 flex-1 break-words select-text ${error ? "text-red-400" : "text-ink-muted"}`}
            >
              {error ?? notice}
            </p>
            <button
              type="button"
              onClick={() => usePullRequestsStore.getState().dismiss()}
              className="shrink-0 text-ink-faint hover:text-ink"
            >
              Dismiss
            </button>
          </div>
        )}
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-4 select-text">
        <Summary row={row} now={now} />
      </div>
    </section>
  );
}

function Actions({ row }: { row: Row }) {
  const { pr, workspace } = row;
  const busy = usePullRequestsStore((s) => s.busy);
  const acting = busy === row.key;
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [copied, setCopied] = useState(false);
  const mergeButton = useRef<HTMLButtonElement>(null);
  const target: Target = {
    key: row.key,
    projectId: row.project.id,
    pr,
    viewer: row.viewer,
  };
  const store = usePullRequestsStore.getState();
  const blocked = mergeBlocked(pr);

  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5">
      {workspace ? (
        <button
          type="button"
          title={`Go to the workspace ${workspace.name}`}
          onClick={() => selectWorkspace(workspace.id)}
          className={button}
        >
          Go to workspace
        </button>
      ) : (
        pr.state === "open" && (
          <button
            type="button"
            disabled={busy !== null}
            title="Fetch its branch and start an agent on it in a new workspace"
            onClick={() => void store.startWorkspace(target)}
            className={button}
          >
            {acting ? "Preparing…" : "Start workspace"}
          </button>
        )
      )}

      {pr.state === "open" && (
        <>
          <button
            ref={mergeButton}
            type="button"
            aria-haspopup="menu"
            aria-expanded={!!menu}
            disabled={busy !== null || blocked !== null}
            title={blocked ?? "Choose how to merge it"}
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              setMenu(menu ? null : { x: box.left, y: box.bottom + 4 });
            }}
            className={button}
          >
            Merge <span aria-hidden>▾</span>
          </button>
          {menu && (
            <ContextMenu
              at={menu}
              onClose={() => {
                setMenu(null);
                mergeButton.current?.focus();
              }}
              items={methods.map(({ method, label }) => ({
                label,
                onSelect: () => void store.merge(target, method, label),
              }))}
            />
          )}
          <button
            type="button"
            disabled={busy !== null}
            title="Close it without merging. Its branch is kept."
            onClick={() => void store.close(target)}
            className={button}
          >
            Close
          </button>
        </>
      )}

      {pr.state === "closed" && (
        <button
          type="button"
          disabled={busy !== null}
          onClick={() => void store.reopen(target)}
          className={button}
        >
          Reopen
        </button>
      )}

      <button
        type="button"
        onClick={() => void openUrl(pr.url).catch(console.error)}
        className={`${button} ml-auto`}
      >
        Open on GitHub <span aria-hidden>↗</span>
      </button>
      <button
        type="button"
        onClick={() =>
          void writeText(pr.url).then(
            () => setCopied(true),
            (reason: unknown) => console.error(reason),
          )
        }
        className={button}
      >
        {copied ? "Copied" : "Copy link"}
      </button>
    </div>
  );
}
