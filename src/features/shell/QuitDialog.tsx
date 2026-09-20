import { useEffect, useId, useRef } from "react";
import { useModalFocus } from "@/lib/useModalFocus";
import { useProjectsStore } from "@/stores/projects";
import { useQuitStore } from "@/stores/quit";
import { useTerminalStore } from "@/stores/terminals";

/**
 * "Agents are still working." Closing the window leaves them running in the daemon, which is
 * the point of it — but it is not something to find out by accident, so the choice is explicit
 * and nothing is stopped unless it is asked for.
 */
export function QuitDialog() {
  const agents = useQuitStore((s) => s.agents);
  const deciding = useQuitStore((s) => s.deciding);
  const tabs = useTerminalStore((s) => s.tabs);
  const projects = useProjectsStore((s) => s.projects);
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  useModalFocus(dialogRef);

  const cancel = () => !deciding && void useQuitStore.getState().cancel();
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => event.key === "Escape" && cancel();
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  if (agents === null) return null;

  const where = agents.map((id) => {
    const tab = tabs.find((t) => t.id === id);
    const project = projects.find((p) => p.workspaces.some((w) => w.id === tab?.workspaceId));
    const workspace = project?.workspaces.find((w) => w.id === tab?.workspaceId);
    return {
      id,
      title: tab?.title ?? "agent",
      place: project && workspace ? `${project.name} / ${workspace.name}` : null,
    };
  });

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 pt-[14vh]">
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[70vh] w-[32rem] max-w-[calc(100vw-2rem)] flex-col rounded-lg border border-line bg-surface p-5 shadow-2xl shadow-black/50 outline-none"
      >
        <h2 id={titleId} className="text-base font-semibold">
          {agents.length === 1
            ? "1 agent is still working"
            : `${agents.length} agents are still working`}
        </h2>
        <p className="mt-1 text-ink-muted">
          They keep running in the background when you close Yardsort, and are waiting for you when
          you open it again.
        </p>

        <ul className="mt-3 min-h-0 flex-1 overflow-y-auto rounded bg-canvas p-3 text-[12px]">
          {where.map((agent) => (
            <li key={agent.id} className="flex gap-2 py-0.5">
              <span className="font-medium text-ink">{agent.title}</span>
              {agent.place && <span className="text-ink-faint">{agent.place}</span>}
            </li>
          ))}
        </ul>

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={deciding}
            onClick={cancel}
            className="rounded px-3 py-1.5 hover:bg-raised disabled:opacity-60"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={deciding}
            onClick={() => void useQuitStore.getState().quit(true)}
            className="rounded px-3 py-1.5 hover:bg-raised disabled:opacity-60"
          >
            Stop them
          </button>
          <button
            type="button"
            disabled={deciding}
            autoFocus
            onClick={() => void useQuitStore.getState().quit(false)}
            className="rounded bg-accent px-3 py-1.5 font-medium text-canvas hover:opacity-90 disabled:opacity-60"
          >
            Leave them running
          </button>
        </div>
      </div>
    </div>
  );
}
