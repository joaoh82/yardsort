import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { selectWorkspace } from "@/features/keyboard/commands";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import type { TaskComment } from "@/lib/ipc";
import { age } from "@/features/pull-requests/rows";
import { useTasksStore, type Target } from "@/stores/tasks";
import { Label } from "./TaskList";
import { at, stateColour, stateLabel, type Row } from "./rows";

// The Markdown renderer is a fair amount of code that nobody needs until they open a task, so
// it arrives then — as it does for a pull request.
const Markdown = lazy(() => import("@/lib/Markdown").then((m) => ({ default: m.Markdown })));

const heading = "text-[11px] font-semibold tracking-wider text-ink-muted uppercase";
const button =
  "h-7 rounded border border-line px-2.5 whitespace-nowrap hover:bg-raised disabled:opacity-40 disabled:hover:bg-transparent";
const quiet = "size-6 shrink-0 rounded text-ink-muted hover:bg-raised hover:text-ink";

const ago = (time: number, now: number) =>
  age(time, now) === "now" ? "just now" : `${age(time, now)} ago`;

/**
 * One task, beside the list: what it is, what its author says it is, what has been said, and a
 * way to hand it to an agent. Replying on it and closing it arrive in a later slice.
 *
 * The top of it comes from the list, which is already here. The description and the
 * conversation are asked for when the row is opened and again whenever the list says the task
 * changed.
 */
export function TaskPane({
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
  const { task, project, workspaces } = row;
  const state = useTasksStore((s) => s.details[row.key]);
  const busy = useTasksStore((s) => s.busy);
  const failed = useTasksStore((s) => s.error);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const goButton = useRef<HTMLButtonElement>(null);
  // Which row's link was copied, so the word does not follow to the next task opened.
  const [copied, setCopied] = useState<string | null>(null);
  const projectId = project.id;
  const target: Target = { key: row.key, projectId, task };

  useEffect(() => {
    void useTasksStore.getState().loadDetail({ key: row.key, projectId, task });
    // `task` is a new object on every poll; `loadDetail` itself compares what matters in it.
  }, [row.key, projectId, task]);

  const detail = state?.detail ?? null;
  const waiting = !detail && (state?.loading ?? true);
  const opened = at(task.createdAt);
  const left = detail ? task.comments - detail.comments.length : 0;

  return (
    <section aria-label={`Task ${task.key}`} className="flex h-full min-h-0 flex-col bg-surface">
      <header className="shrink-0 border-b border-line px-4 py-2">
        <div className="flex items-center gap-2">
          <p className="min-w-0 flex-1 truncate text-ink-muted">
            <span className="text-ink">{project.name}</span>
            {" · "}
            {task.author ?? "ghost"}
            {opened !== null && <> opened it {ago(opened, now)}</>}
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
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {workspaces.length === 1 && (
            <button
              type="button"
              title={`Go to the workspace ${workspaces[0]!.name}`}
              onClick={() => selectWorkspace(workspaces[0]!.id)}
              className={button}
            >
              Go to workspace
            </button>
          )}
          {workspaces.length > 1 && (
            <>
              <button
                ref={goButton}
                type="button"
                aria-haspopup="menu"
                aria-expanded={!!menu}
                title="The workspaces started from this task"
                onClick={(event) => {
                  const box = event.currentTarget.getBoundingClientRect();
                  setMenu(menu ? null : { x: box.left, y: box.bottom + 4 });
                }}
                className={button}
              >
                Go to workspace <span aria-hidden>▾</span>
              </button>
              {menu && (
                <ContextMenu
                  at={menu}
                  onClose={() => {
                    setMenu(null);
                    goButton.current?.focus();
                  }}
                  items={workspaces.map((workspace) => ({
                    label: workspace.name,
                    onSelect: () => selectWorkspace(workspace.id),
                  }))}
                />
              )}
            </>
          )}
          {task.state === "open" && (
            <button
              type="button"
              disabled={busy !== null}
              title="Open the composer with this task as the agent's first message. Nothing starts until you say so."
              onClick={() => void useTasksStore.getState().delegate(target)}
              className={button}
            >
              {busy === row.key
                ? "Preparing…"
                : workspaces.length > 0
                  ? "Delegate again"
                  : "Delegate"}
            </button>
          )}
          <button
            type="button"
            onClick={() => void openUrl(task.url).catch(console.error)}
            className={button}
          >
            Open on GitHub <span aria-hidden>↗</span>
          </button>
          <button
            type="button"
            onClick={() =>
              void writeText(task.url).then(
                () => setCopied(row.key),
                (reason: unknown) => console.error(reason),
              )
            }
            className={button}
          >
            {copied === row.key ? "Copied" : "Copy link"}
          </button>
        </div>
        {failed && (
          <div className="mt-2 flex items-start gap-2">
            <p role="alert" className="min-w-0 flex-1 break-words text-red-400 select-text">
              {failed}
            </p>
            <button
              type="button"
              onClick={() => useTasksStore.getState().dismiss()}
              className="shrink-0 text-ink-faint hover:text-ink"
            >
              Dismiss
            </button>
          </div>
        )}
      </header>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4 select-text">
        <div>
          <h2 className="text-[15px] leading-snug font-medium text-ink">
            {task.title} <span className="font-mono font-normal text-ink-muted">{task.key}</span>
          </h2>
          <p className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-ink-muted">
            <span className={`rounded-full px-2 leading-5 ${stateColour(task)}`}>
              {stateLabel(task)}
            </span>
            {task.needsAnswer && (
              <span className="rounded-full bg-amber-500/20 px-2 leading-5 text-amber-300">
                Needs an answer
              </span>
            )}
            {task.labels.map((label) => (
              <Label key={label.name} label={label} />
            ))}
          </p>
          <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[12px]">
            <dt className="text-ink-faint">Assigned to</dt>
            <dd>{task.assignees.length > 0 ? task.assignees.join(", ") : "No one"}</dd>
            {task.linkedPullRequests.length > 0 && (
              <>
                <dt className="text-ink-faint">Closed by</dt>
                <dd>
                  {task.linkedPullRequests.map((number) => `#${number}`).join(", ")}, when{" "}
                  {task.linkedPullRequests.length === 1 ? "it merges" : "they merge"}
                </dd>
              </>
            )}
          </dl>
        </div>

        {state?.error && (
          <div role="alert" className="flex items-start gap-2 text-red-400">
            <p className="min-w-0 flex-1 break-words select-text">
              Could not read this task: {state.error}
            </p>
            <button
              type="button"
              onClick={() => void useTasksStore.getState().loadDetail(target, true)}
              className="shrink-0 text-ink-muted underline hover:text-ink"
            >
              Retry
            </button>
          </div>
        )}

        <Section label="Description">
          {waiting ? (
            <Faint>Loading…</Faint>
          ) : !detail ? null : detail.body.trim() === "" ? (
            <Faint>No description.</Faint>
          ) : (
            <Words text={detail.body} />
          )}
        </Section>

        <Section label="Conversation">
          {waiting ? (
            <Faint>Loading…</Faint>
          ) : !detail ? null : detail.comments.length === 0 ? (
            <Faint>Nobody has commented yet.</Faint>
          ) : (
            <>
              {left > 0 && (
                <p className="mt-2 text-[11px] text-ink-faint">
                  The latest {detail.comments.length} of {task.comments} comments.{" "}
                  <button
                    type="button"
                    onClick={() => void openUrl(task.url).catch(console.error)}
                    className="underline hover:text-ink"
                  >
                    Read them all on GitHub <span aria-hidden>↗</span>
                  </button>
                </p>
              )}
              <ol className="mt-2 space-y-3">
                {detail.comments.map((comment, index) => (
                  <Comment key={comment.url ?? index} comment={comment} now={now} />
                ))}
              </ol>
            </>
          )}
        </Section>
      </div>
    </section>
  );
}

function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section aria-label={label} className="border-t border-line pt-3">
      <h3 className={heading}>{label}</h3>
      {children}
    </section>
  );
}

const Faint = ({ children }: { children: ReactNode }) => (
  <p className="mt-2 text-ink-faint">{children}</p>
);

/** Markdown, with the plain text in its place for the moment the renderer takes to arrive. */
function Words({ text }: { text: string }) {
  return (
    <div className="mt-2">
      <Suspense fallback={<p className="whitespace-pre-wrap text-ink-muted">{text}</p>}>
        <Markdown text={text} />
      </Suspense>
    </div>
  );
}

function Comment({ comment, now }: { comment: TaskComment; now: number }) {
  const body = comment.body.trim();
  const written = at(comment.createdAt);
  return (
    <li className="rounded border border-line">
      <p className="flex items-baseline gap-1.5 border-b border-line bg-raised px-3 py-1.5 text-[12px]">
        <span className="min-w-0 truncate font-medium">{comment.author ?? "ghost"}</span>
        {(comment.bot || comment.maintainer) && (
          <span className="shrink-0 rounded-full border border-line px-1.5 text-[10px] leading-4 text-ink-muted">
            {comment.bot ? "bot" : "maintainer"}
          </span>
        )}
        {written !== null && (
          <span className="shrink-0 text-ink-faint" title={new Date(written).toLocaleString()}>
            {ago(written, now)}
          </span>
        )}
        {comment.url && (
          <button
            type="button"
            aria-label="Open on GitHub"
            title="Open on GitHub"
            onClick={() => void openUrl(comment.url!).catch(console.error)}
            className="ml-auto shrink-0 text-ink-faint hover:text-ink"
          >
            <span aria-hidden>↗</span>
          </button>
        )}
      </p>
      <div className="px-3 py-2">
        {comment.hidden !== null ? (
          <p className="text-ink-faint italic">
            Hidden on GitHub{comment.hidden ? ` as ${comment.hidden.replace(/_/g, " ")}` : ""}.
          </p>
        ) : body !== "" ? (
          <Suspense fallback={<p className="whitespace-pre-wrap text-ink-muted">{body}</p>}>
            <Markdown text={body} />
          </Suspense>
        ) : (
          <p className="text-ink-faint">Nothing written.</p>
        )}
      </div>
    </li>
  );
}
