import { useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { LineNote, LineSelection } from "@/features/changes/CodeView";
import { DiffBody, Note } from "@/features/changes/DiffBody";
import { CHANGE_KIND, DIFF_MODE_KEY, type DiffMode } from "@/features/changes/viewing";
import {
  errorMessage,
  ipc,
  type FileChange,
  type FileDiff,
  type PullRequestChanges,
} from "@/lib/ipc";
import { recall, useProjectsStore } from "@/stores/projects";
import { usePullRequestsStore, type Target } from "@/stores/pullRequests";
import { LineThread } from "./LineThread";
import { NoteDialog } from "./NoteDialog";
import type { Row } from "./rows";
import { placeOf, placed, threadCounts, threadsOf, unplaced, type Thread } from "./threads";

const quiet =
  "h-7 shrink-0 rounded px-2 whitespace-nowrap text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40 disabled:hover:bg-transparent";

/** One file's two sides, for the file in view: asked for when it is chosen. */
function useFileDiff(
  projectId: string,
  changes: PullRequestChanges | null,
  file: FileChange | null,
): { diff: FileDiff | null; error: string | null } {
  // Kept with what it was asked about, so a file chosen since is "loading", not the last one.
  const [answer, setAnswer] = useState<{
    about: string;
    diff: FileDiff | null;
    error: string | null;
  } | null>(null);
  const about = changes && file ? `${changes.baseOid}..${changes.headOid}:${file.path}` : null;
  const base = changes?.baseOid;
  const head = changes?.headOid;
  const path = file?.path;
  const oldPath = file?.oldPath ?? null;

  useEffect(() => {
    if (!about || !base || !head || !path) return;
    let stale = false;
    ipc.pullRequestDiff(projectId, { baseOid: base, headOid: head }, { path, oldPath }).then(
      (diff) => !stale && setAnswer({ about, diff, error: null }),
      (reason: unknown) => !stale && setAnswer({ about, diff: null, error: errorMessage(reason) }),
    );
    return () => {
      stale = true;
    };
  }, [projectId, about, base, head, path, oldPath]);

  return answer && answer.about === about
    ? { diff: answer.diff, error: answer.error }
    : { diff: null, error: null };
}

/**
 * A pull request's diff: the files it changes, and each one's two sides in the viewer the
 * changes panel uses. Nothing is checked out for it — the commits are fetched into refs of
 * Yardsort's own and read from there. See `crate::publish::pull_requests`.
 */
export function Code({ row, now }: { row: Row; now: number }) {
  const { pr } = row;
  const projectId = row.project.id;
  const state = usePullRequestsStore((s) => s.changes[row.key]);
  const comments = usePullRequestsStore((s) => s.lineComments[row.key]?.comments);
  const ui = useProjectsStore((s) => s.ui);
  const mode = recall<DiffMode>(ui, DIFF_MODE_KEY, "inline");
  const [chosen, setChosen] = useState<string | null>(null);
  const [selection, setSelection] = useState<LineSelection | null>(null);
  const [noting, setNoting] = useState<LineSelection | null>(null);
  const target: Target = { key: row.key, projectId, pr, viewer: row.viewer };

  useEffect(() => {
    const store = usePullRequestsStore.getState();
    const about: Target = { key: row.key, projectId, pr, viewer: null };
    void store.loadChanges(about);
    // The comments on lines come with the diff, and again when the list says the pull request
    // changed: a comment moves `updatedAt`.
    void store.loadLineComments(about);
    // `pr` is a new object on every poll; both compare what matters in it themselves.
  }, [row.key, projectId, pr]);

  const changes = state?.changes ?? null;
  const files = changes?.files ?? [];
  // A file that a later push removed from the diff is no longer something to look at.
  const index = files.findIndex((file) => file.path === chosen);
  const file = index >= 0 ? files[index]! : null;
  const { diff, error: diffError } = useFileDiff(projectId, changes, file);
  const counts = useMemo(() => threadCounts(comments ?? []), [comments]);
  const threads = useMemo(
    () => (file ? threadsOf(comments ?? [], file.path) : []),
    [comments, file],
  );
  const onLines = useMemo(() => placed(threads), [threads]);
  const elsewhere = useMemo(() => unplaced(threads), [threads]);
  const notes: LineNote[] = onLines.map((thread) => ({
    key: thread.root.id,
    line: thread.root.line!,
    side: thread.root.side === "left" ? "old" : "new",
  }));
  const threadByKey = new Map(threads.map((thread) => [thread.root.id, thread]));
  const renderNote = (key: string) => {
    const thread = threadByKey.get(key);
    return thread ? <LineThread thread={thread} now={now} /> : null;
  };

  if (!changes) {
    return state?.error ? (
      <Failed
        error={state.error}
        url={pr.url}
        onRetry={() => void usePullRequestsStore.getState().loadChanges(target, true)}
      />
    ) : (
      <Note>Fetching the pull request&rsquo;s commits…</Note>
    );
  }

  const added = files.reduce((sum, each) => sum + (each.additions ?? 0), 0);
  const removed = files.reduce((sum, each) => sum + (each.deletions ?? 0), 0);

  if (!file) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        {state?.error && (
          <Failed
            error={state.error}
            url={pr.url}
            onRetry={() => void usePullRequestsStore.getState().loadChanges(target, true)}
          />
        )}
        <p className="shrink-0 border-b border-line px-4 py-2 text-ink-muted">
          {files.length === 1 ? "1 file changed" : `${files.length} files changed`}
          <span className="ml-2 font-mono text-[12px]">
            <span className="text-green-400">+{added}</span>{" "}
            <span className="text-red-400">−{removed}</span>
          </span>
          {state?.loading && <span className="ml-2 text-ink-faint">Fetching again…</span>}
        </p>
        {files.length === 0 ? (
          <Note>No files differ between this pull request and where it left its base.</Note>
        ) : (
          <ul aria-label="Changed files" className="min-h-0 flex-1 overflow-y-auto py-1">
            {files.map((each) => (
              <FileRow
                key={each.path}
                file={each}
                threads={counts.get(each.path) ?? 0}
                onOpen={() => setChosen(each.path)}
              />
            ))}
          </ul>
        )}
      </div>
    );
  }

  const open = (next: string | null) => {
    setSelection(null);
    setChosen(next);
  };
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-line px-2 py-1">
        <button type="button" onClick={() => open(null)} className={quiet}>
          <span aria-hidden>← </span>All files
        </button>
        <select
          aria-label="Files"
          value={file.path}
          onChange={(event) => open(event.target.value)}
          className="h-7 min-w-0 flex-1 rounded border border-line bg-canvas px-2 font-mono text-[12px] text-ink outline-none focus:border-accent"
        >
          {files.map((each) => (
            <option key={each.path} value={each.path}>
              {CHANGE_KIND[each.kind].letter} {each.path}
            </option>
          ))}
        </select>
        <button
          type="button"
          aria-label="Previous file"
          title="Previous file"
          disabled={index === 0}
          onClick={() => open(files[index - 1]?.path ?? chosen)}
          className={quiet}
        >
          <span aria-hidden>‹</span>
        </button>
        <span className="shrink-0 text-[11px] text-ink-faint tabular-nums">
          {index + 1} of {files.length}
        </span>
        <button
          type="button"
          aria-label="Next file"
          title="Next file"
          disabled={index === files.length - 1}
          onClick={() => open(files[index + 1]?.path ?? chosen)}
          className={quiet}
        >
          <span aria-hidden>›</span>
        </button>
        <button
          type="button"
          title={
            mode === "split"
              ? "Show removals inline, in one pane"
              : "Show the two versions side by side"
          }
          onClick={() =>
            useProjectsStore
              .getState()
              .remember(DIFF_MODE_KEY, mode === "split" ? "inline" : "split")
          }
          className={quiet}
        >
          {mode === "split" ? "Inline" : "Side by side"}
        </button>
        <button
          type="button"
          disabled={!selection}
          title={
            selection
              ? "Write a note about the selected lines for an agent, or for GitHub"
              : "Select lines in the diff first"
          }
          onClick={() => selection && setNoting(selection)}
          className={quiet}
        >
          Note on lines…
        </button>
      </div>
      {file.oldPath && (
        <p className="shrink-0 border-b border-line px-3 py-1 font-mono text-[11px] text-ink-faint">
          {file.oldPath} → {file.path}
        </p>
      )}
      <div className="min-h-0 flex-1 bg-canvas">
        {diffError ? (
          <Note alert>{diffError}</Note>
        ) : (
          <DiffBody
            path={file.path}
            diff={diff}
            mode={mode}
            notes={notes}
            renderNote={renderNote}
            onSelect={setSelection}
          />
        )}
      </div>
      {elsewhere.length > 0 && <Elsewhere threads={elsewhere} now={now} />}
      {noting && changes && (
        <NoteDialog
          row={row}
          excerpt={{
            path: file.path,
            side: noting.side === "old" ? "left" : "right",
            from: noting.from,
            to: noting.to,
            text: noting.text,
          }}
          headOid={changes.headOid}
          onClose={() => setNoting(null)}
        />
      )}
    </div>
  );
}

/** The comments on this file that the forge no longer places on a line, or never did. */
function Elsewhere({ threads, now }: { threads: Thread[]; now: number }) {
  return (
    <details className="shrink-0 border-t border-line px-3 py-1.5">
      <summary className="cursor-pointer text-ink-muted">
        {threads.length === 1
          ? "1 comment the diff has moved on from"
          : `${threads.length} comments the diff has moved on from`}
        <span className="ml-2 text-[11px] text-ink-faint">
          {threads.map((thread) => placeOf(thread.root)).join(" · ")}
        </span>
      </summary>
      <div className="max-h-64 overflow-y-auto py-1">
        {threads.map((thread) => (
          <LineThread key={thread.root.id} thread={thread} now={now} />
        ))}
      </div>
    </details>
  );
}

function FileRow({
  file,
  threads,
  onOpen,
}: {
  file: FileChange;
  threads: number;
  onOpen: () => void;
}) {
  const kind = CHANGE_KIND[file.kind];
  const slash = file.path.lastIndexOf("/");
  return (
    <li>
      <button
        type="button"
        title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
        onClick={onOpen}
        className="flex h-7 w-full items-center gap-2 px-4 text-left hover:bg-raised"
      >
        <span
          className={`w-3 shrink-0 text-center font-mono text-[11px] ${kind.colour}`}
          title={kind.label}
        >
          {kind.letter}
        </span>
        <span className={`truncate ${file.kind === "deleted" ? "line-through" : ""}`}>
          {file.path.slice(slash + 1)}
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-ink-faint" dir="rtl">
          {slash > 0 ? file.path.slice(0, slash) : ""}
        </span>
        {threads > 0 && (
          <span
            aria-label={threads === 1 ? "1 comment thread" : `${threads} comment threads`}
            title={threads === 1 ? "1 comment thread" : `${threads} comment threads`}
            className="shrink-0 rounded-full border border-line px-1.5 text-[10px] leading-4 text-ink-muted tabular-nums"
          >
            <span aria-hidden>💬 </span>
            {threads}
          </span>
        )}
        <span className="shrink-0 font-mono text-[11px]">
          {file.additions != null && <span className="text-green-400">+{file.additions}</span>}{" "}
          {file.deletions != null && <span className="text-red-400">−{file.deletions}</span>}
        </span>
      </button>
    </li>
  );
}

/** The commits could not be fetched: what git said, a way to try again, and the forge's own diff. */
function Failed({ error, url, onRetry }: { error: string; url: string; onRetry: () => void }) {
  return (
    <div role="alert" className="shrink-0 border-b border-line p-4">
      <p className="break-words text-red-400 select-text">
        Could not fetch this pull request&rsquo;s commits: {error}
      </p>
      <p className="mt-2 flex gap-3 text-ink-muted">
        <button type="button" onClick={onRetry} className="underline hover:text-ink">
          Retry
        </button>
        <button
          type="button"
          onClick={() => void openUrl(`${url}/files`).catch(console.error)}
          className="underline hover:text-ink"
        >
          See the diff on GitHub <span aria-hidden>↗</span>
        </button>
      </p>
    </div>
  );
}
