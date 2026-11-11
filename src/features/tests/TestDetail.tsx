import { useState } from "react";
import { timeAgo } from "@/features/terminal/sessionText";
import { ScheduleDialog } from "@/features/schedules/ScheduleDialog";
import { RunSmokeTestDialog } from "@/features/smoke/RunSmokeTestDialog";
import { statusColour, statusWords } from "@/features/smoke/words";
import type { TestListing, TestRun } from "@/lib/ipc";

/**
 * One test, open beside the list: its file (read-only — it is the repository's, edited in the
 * worktree), every run of it across workspaces, and **Run in…** a workspace of its project. A
 * run here opens its tab in its workspace.
 */
export function TestDetail({
  listing,
  catalog,
  runs,
  now,
  onOpenRun,
  onClose,
}: {
  listing: TestListing;
  catalog: TestListing[];
  runs: TestRun[];
  now: number;
  onOpenRun: (workspaceId: string | null, runId: string) => void;
  onClose: () => void;
}) {
  // The same test in other worktrees of its project: where it can also be run.
  const places = catalog
    .filter((other) => other.projectId === listing.projectId && other.item.id === listing.item.id)
    .map((other) => ({ id: other.workspaceId, name: other.workspaceName }));
  const [runIn, setRunIn] = useState(listing.workspaceId);
  const [running, setRunning] = useState(false);
  const [scheduling, setScheduling] = useState(false);
  // Every run of this test in its project, newest first, whichever workspace it ran in. Test ids
  // are repository-local, so a `health` in one project is not the `health` of another: scope by
  // project id too, or two projects' identical ids would show each other's runs.
  const history = runs.filter(
    (run) => run.testId === listing.item.id && run.projectId === listing.projectId,
  );

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-start gap-2 border-b border-line px-4 py-2.5">
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-semibold text-ink">{listing.item.name}</h2>
          <p className="truncate text-detail text-ink-faint">
            <code className="select-text">{listing.item.id}</code> · {listing.projectName} /{" "}
            {listing.workspaceName}
          </p>
        </div>
        <button
          type="button"
          aria-label="Close test"
          title="Close"
          onClick={onClose}
          className="size-7 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
        {listing.item.description && <p className="text-ink-muted">{listing.item.description}</p>}

        {listing.item.problems.length > 0 && (
          <ul aria-label="Problems" className="flex flex-col gap-1 text-red-400">
            {listing.item.problems.map((problem, i) => (
              <li key={i} className="select-text">
                {problem.message}
              </li>
            ))}
          </ul>
        )}
        {listing.item.browserMissing && (
          <p className="text-amber-300">
            This test opens a browser, and none was found on this machine. Install one, or name it
            in Settings → Projects → Smoke tests.
          </p>
        )}
        {listing.envMissing.length > 0 && (
          <p className="text-amber-300">
            It uses environment values the project has not set:{" "}
            <span className="select-text">{listing.envMissing.join(", ")}</span>. Set them in
            Settings → Projects → Smoke tests.
          </p>
        )}

        {/* Run in… a workspace of this test's project. */}
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-small font-semibold tracking-wider text-ink-faint uppercase">
            Run in
          </span>
          <select
            aria-label="Run in workspace"
            value={runIn}
            onChange={(event) => setRunIn(event.target.value)}
            className="h-8 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent"
          >
            {places.map((place) => (
              <option key={place.id} value={place.id}>
                {place.name}
              </option>
            ))}
          </select>
          <button
            type="button"
            disabled={!listing.item.runnable || places.length === 0}
            title={listing.item.runnable ? "Run this test" : "Fix the problems above first"}
            onClick={() => setRunning(true)}
            className="rounded bg-accent px-3 py-1.5 font-medium text-canvas disabled:opacity-40"
          >
            Run…
          </button>
          <button
            type="button"
            disabled={!listing.item.runnable || places.length === 0}
            title={
              listing.item.runnable
                ? "Run it at a set time, or again and again"
                : "Fix the problems above first"
            }
            onClick={() => setScheduling(true)}
            className="rounded border border-line px-3 py-1.5 text-ink-muted hover:bg-raised hover:text-ink disabled:opacity-40"
          >
            Schedule…
          </button>
        </div>

        {/* The file, read-only: it lives in the worktree, edited there. */}
        <div>
          <p className="mb-1 text-small font-semibold tracking-wider text-ink-faint uppercase">
            The file
          </p>
          <pre className="max-h-96 overflow-auto rounded border border-line bg-canvas p-3 text-detail text-ink-muted select-text">
            {listing.item.text}
          </pre>
        </div>

        {/* Its runs, across every workspace. */}
        <div>
          <p className="mb-1 text-small font-semibold tracking-wider text-ink-faint uppercase">
            Runs
          </p>
          {history.length === 0 ? (
            <p className="text-ink-faint">Never run.</p>
          ) : (
            <ul aria-label="Run history" className="flex flex-col">
              {history.map((run) => (
                <li key={run.id}>
                  <button
                    type="button"
                    onClick={() => onOpenRun(run.workspaceId, run.id)}
                    disabled={!run.workspaceId}
                    className="flex w-full items-baseline gap-2 rounded px-1 py-1 text-left hover:bg-raised disabled:opacity-50"
                  >
                    <span className={`shrink-0 text-small ${statusColour(run.status)}`}>
                      {statusWords(run)}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-detail text-ink-faint">
                      {run.workspaceName}
                    </span>
                    <span className="shrink-0 text-detail text-ink-faint tabular-nums">
                      {timeAgo(run.createdAt, now)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {scheduling && (
        <ScheduleDialog
          target="test"
          targetId={listing.item.id}
          workspaceId={runIn}
          onClose={() => setScheduling(false)}
        />
      )}
      {running && (
        <RunSmokeTestDialog
          workspaceId={runIn}
          testId={listing.item.id}
          onClose={() => setRunning(false)}
        />
      )}
    </div>
  );
}
