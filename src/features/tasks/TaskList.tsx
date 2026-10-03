import type { KeyboardEvent } from "react";
import type { TaskLabel } from "@/lib/ipc";
import { selectWorkspace } from "@/features/keyboard/commands";
import { age } from "@/features/pull-requests/rows";
import { at, stateColour, stateLabel, type Row } from "./rows";

/** Up and Down between rows, Home and End to the ends. Enter is the button's own. */
function moveBetweenRows(event: KeyboardEvent<HTMLUListElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const rows = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("[data-task-row]")];
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

/** A label in its own colour, as its source shows it: tinted, with the name readable on both. */
export function Label({ label }: { label: TaskLabel }) {
  const colour = /^[0-9a-f]{6}$/i.test(label.color) ? `#${label.color}` : null;
  return (
    <span
      className="rounded-full border border-line px-1.5 leading-4 text-ink-muted"
      style={colour ? { borderColor: `${colour}99`, backgroundColor: `${colour}26` } : undefined}
    >
      {label.name}
    </span>
  );
}

/** The rows: one task each, most recently changed first. */
export function TaskList({
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
    <ul aria-label="Tasks" onKeyDown={moveBetweenRows} className="h-full min-h-0 overflow-y-auto">
      {rows.map((row) => (
        <TaskRow
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

function TaskRow({
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
  const { task, project, workspaces } = row;
  const updated = at(task.updatedAt);
  return (
    <li className={`border-b border-line ${selected ? "bg-raised" : ""}`}>
      <button
        type="button"
        data-task-row={row.key}
        aria-current={selected ? "true" : undefined}
        onClick={() => onSelect(row.key)}
        className={`block w-full px-3 pt-2 text-left outline-none hover:bg-raised focus-visible:bg-raised ${workspaces.length > 0 ? "pb-1" : "pb-2"}`}
      >
        <span className="flex items-center gap-2">
          <span className="min-w-0 truncate text-ink-muted">{project.name}</span>
          <span className="shrink-0 font-mono text-ink-muted">{task.key}</span>
          {task.needsAnswer && (
            <span className="ml-auto shrink-0 rounded-full bg-amber-500/20 px-2 text-[11px] leading-5 text-amber-300">
              Needs an answer
            </span>
          )}
          <span
            className={`${task.needsAnswer ? "" : "ml-auto"} shrink-0 rounded-full px-2 text-[11px] leading-5 ${stateColour(task)}`}
          >
            {stateLabel(task)}
          </span>
        </span>
        <span data-task-title className="mt-0.5 block truncate text-ink">
          {task.title}
        </span>
        <span className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-ink-faint">
          <span>{task.author ?? "ghost"}</span>
          {updated !== null && (
            <span title={`Updated ${new Date(updated).toLocaleString()}`}>{age(updated, now)}</span>
          )}
          {task.comments > 0 && (
            <span aria-label={`${task.comments} ${task.comments === 1 ? "comment" : "comments"}`}>
              <span aria-hidden>💬 </span>
              {task.comments}
            </span>
          )}
          {task.assignees.length > 0 && <span>→ {task.assignees.join(", ")}</span>}
          {task.linkedPullRequests.map((number) => (
            <span key={number} title="An open pull request will close it when it merges">
              PR #{number}
            </span>
          ))}
          {task.labels.map((label) => (
            <Label key={label.name} label={label} />
          ))}
        </span>
      </button>
      {workspaces.map((workspace) => (
        <button
          key={workspace.id}
          type="button"
          title={`Go to the workspace ${workspace.name}`}
          onClick={() => selectWorkspace(workspace.id)}
          className="mb-1.5 ml-3 block max-w-[calc(100%-1.5rem)] truncate rounded px-1 text-[11px] text-ink-muted hover:bg-raised hover:text-ink"
        >
          <span aria-hidden>↳ </span>
          {workspace.name}
        </button>
      ))}
    </li>
  );
}
