import type { KeyboardEvent } from "react";
import { selectWorkspace } from "@/features/keyboard/commands";
import { checksColour, conflicting, pullRequestColour, pullRequestStateLabel } from "./appearance";
import { age, checksLabel, checksSummary, type Row } from "./rows";

const checkMark = { none: "", running: "●", passing: "✓", failing: "✕" } as const;

/** Up and Down between rows, Home and End to the ends. Enter is the button's own. */
function moveBetweenRows(event: KeyboardEvent<HTMLUListElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const rows = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("[data-pr-row]")];
  const index = rows.findIndex((row) => row === document.activeElement);
  if (index < 0) return;
  const target =
    event.key === "ArrowDown"
      ? rows[Math.min(index + 1, rows.length - 1)]
      : event.key === "ArrowUp"
        ? rows[Math.max(index - 1, 0)]
        : event.key === "Home"
          ? rows[0]
          : event.key === "End"
            ? rows[rows.length - 1]
            : undefined;
  if (!target) return;
  event.preventDefault();
  target.focus();
}

/** The rows: one pull request each, most recently changed first. */
export function PullRequestList({
  rows,
  selected,
  now,
  onSelect,
}: {
  rows: Row[];
  selected: string | null;
  now: number;
  onSelect: (key: string) => void;
}) {
  return (
    <ul
      aria-label="Pull requests"
      onKeyDown={moveBetweenRows}
      className="h-full min-h-0 overflow-y-auto"
    >
      {rows.map((row) => (
        <PullRequestRow
          key={row.key}
          row={row}
          selected={row.key === selected}
          now={now}
          onSelect={onSelect}
        />
      ))}
    </ul>
  );
}

function PullRequestRow({
  row,
  selected,
  now,
  onSelect,
}: {
  row: Row;
  selected: boolean;
  now: number;
  onSelect: (key: string) => void;
}) {
  const { pr, project, workspace } = row;
  const details = pr.details;
  const summary = checksSummary(pr);
  return (
    <li className={`border-b border-line ${selected ? "bg-raised" : ""}`}>
      <button
        type="button"
        data-pr-row={row.key}
        aria-current={selected ? "true" : undefined}
        onClick={() => onSelect(row.key)}
        className={`block w-full px-3 pt-2 text-left outline-none hover:bg-raised focus-visible:bg-raised ${workspace ? "pb-1" : "pb-2"}`}
      >
        <span className="flex items-center gap-2">
          <span className="min-w-0 truncate text-ink-muted">{project.name}</span>
          <span className="shrink-0 font-mono text-ink-muted">#{pr.number}</span>
          <span
            className={`ml-auto shrink-0 rounded-full px-2 text-[11px] leading-5 ${pullRequestColour(pr)}`}
          >
            {pullRequestStateLabel(pr)}
          </span>
        </span>
        <span className="mt-0.5 block truncate text-ink">{pr.title}</span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px] text-ink-faint">
          <span>{pr.author ?? "ghost"}</span>
          {summary && (
            <span className={checksColour[pr.checks]} aria-label={checksLabel(pr)}>
              <span aria-hidden>{checkMark[pr.checks]} </span>
              {summary}
            </span>
          )}
          {pr.createdAt !== null && (
            <span title={`Opened ${new Date(pr.createdAt).toLocaleString()}`}>
              {age(pr.createdAt, now)}
            </span>
          )}
          {details && (
            <span className="font-mono">
              <span className="text-green-400">+{details.additions}</span>{" "}
              <span className="text-red-400">−{details.deletions}</span>
            </span>
          )}
          {conflicting(pr) && (
            <span className="text-red-400" title={`Conflicts with ${details?.base || "its base"}`}>
              ⚠ conflicts
            </span>
          )}
        </span>
      </button>
      {workspace && (
        <button
          type="button"
          title={`Go to the workspace ${workspace.name}`}
          onClick={() => selectWorkspace(workspace.id)}
          className="mb-1.5 ml-3 max-w-[calc(100%-1.5rem)] truncate rounded px-1 text-[11px] text-ink-muted hover:bg-raised hover:text-ink"
        >
          <span aria-hidden>↳ </span>
          {workspace.name}
        </button>
      )}
    </li>
  );
}
