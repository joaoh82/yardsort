import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
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
import type { Row } from "./rows";

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
export function Code({ row }: { row: Row }) {
  const { pr } = row;
  const projectId = row.project.id;
  const state = usePullRequestsStore((s) => s.changes[row.key]);
  const ui = useProjectsStore((s) => s.ui);
  const mode = recall<DiffMode>(ui, DIFF_MODE_KEY, "inline");
  const [chosen, setChosen] = useState<string | null>(null);
  const target: Target = { key: row.key, projectId, pr, viewer: row.viewer };

  useEffect(() => {
    void usePullRequestsStore.getState().loadChanges({ key: row.key, projectId, pr, viewer: null });
    // `pr` is a new object on every poll; `loadChanges` compares the head commit itself.
  }, [row.key, projectId, pr]);

  const changes = state?.changes ?? null;
  const files = changes?.files ?? [];
  // A file that a later push removed from the diff is no longer something to look at.
  const index = files.findIndex((file) => file.path === chosen);
  const file = index >= 0 ? files[index]! : null;
  const { diff, error: diffError } = useFileDiff(projectId, changes, file);

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
              <FileRow key={each.path} file={each} onOpen={() => setChosen(each.path)} />
            ))}
          </ul>
        )}
      </div>
    );
  }

  const step = (by: -1 | 1) => setChosen(files[index + by]?.path ?? chosen);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-line px-2 py-1">
        <button type="button" onClick={() => setChosen(null)} className={quiet}>
          <span aria-hidden>← </span>All files
        </button>
        <select
          aria-label="Files"
          value={file.path}
          onChange={(event) => setChosen(event.target.value)}
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
          onClick={() => step(-1)}
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
          onClick={() => step(1)}
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
          <DiffBody path={file.path} diff={diff} mode={mode} />
        )}
      </div>
    </div>
  );
}

function FileRow({ file, onOpen }: { file: FileChange; onOpen: () => void }) {
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
