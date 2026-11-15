import { useEffect, useId, useRef, useState } from "react";
import { ipc } from "@/lib/ipc";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { linkText } from "@/features/sidebar/otherComputersText";
import { thereKey, useOtherComputersStore } from "@/stores/otherComputers";
import { recall, useProjectsStore } from "@/stores/projects";
import { RunOn } from "./RunOn";
import { offeredThere, PROJECT_THERE, startThere } from "./there";

const control =
  "h-9 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";

/**
 * The start of a workspace on another computer: say what you want, pick which of that computer's
 * agents does it, press Enter. The workspace, its branch and its worktree are made there, as that
 * computer's own window makes them, and nothing is made on this computer. If starting fails, the
 * message stays, and the start is not tried again by itself.
 *
 * Its title says which computer, and is where another is chosen. Opened from one of this
 * computer's projects (`from`), the project there is the one offered for it, shown and changeable
 * before the start, and remembered once something starts.
 */
export function RemoteComposer({
  hostId,
  projectId,
  from = null,
}: {
  hostId: string;
  projectId: string | null;
  from?: string | null;
}) {
  const view = useOtherComputersStore((s) => s.views[hostId]);
  const hostName = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );
  const agents = useOtherComputersStore((s) => s.harnesses[hostId]);
  const problem = useOtherComputersStore((s) => s.startProblem);
  const local = useProjectsStore((s) => s.projects.find((candidate) => candidate.id === from));
  const localProjects = useProjectsStore((s) => s.projects);
  const ui = useProjectsStore((s) => s.ui);
  const [agent, setAgent] = useState<string | null>(null);
  // What was typed before the computer was chosen comes along. Read once: choosing another
  // computer is a new mount, and brings what is here by then.
  const [message, setMessage] = useState(
    () => useProjectsStore.getState().composingThere?.message ?? "",
  );
  const [busy, setBusy] = useState(false);
  const projectFieldId = useId();
  const messageRef = useRef<HTMLTextAreaElement>(null);
  // Taken before anything is awaited: a second Start while the first is on its way would start
  // a second agent on the same task.
  const starting = useRef(false);

  const name = view?.name || hostName || "another computer";
  const project = view?.projects.find((candidate) => candidate.id === projectId);
  const online = view?.link.kind === "online";
  const mayStart = online && !!view?.mayStart;

  useEffect(() => {
    void useOtherComputersStore.getState().loadHarnesses(hostId);
    useOtherComputersStore.getState().couldNotStart(null);
  }, [hostId]);
  useEffect(() => messageRef.current?.focus(), [hostId, projectId]);

  const chosen = agent ?? agents?.[0]?.id ?? null;
  const ready =
    !busy && mayStart && chosen !== null && message.trim() !== "" && project !== undefined;

  // Why the project in the list is the one offered, for the project here it was opened in.
  const offered = local ? offeredThere(view, ui, hostId, local) : null;
  const why = !local
    ? null
    : !project
      ? null
      : offered?.projectId !== project.id
        ? "remembered after you start"
        : offered.how === "remembered"
          ? "as you chose last time"
          : "same name as here · remembered after you start";

  // What This computer goes back to: the project it was opened in, or one here of the same name.
  const here =
    local ??
    localProjects.find((candidate) => candidate.name === project?.name) ??
    localProjects.find((candidate) => candidate.name.toLowerCase() === project?.name.toLowerCase());

  const start = async () => {
    if (!ready || !project || starting.current) return;
    starting.current = true;
    setBusy(true);
    const started = await startThere(hostId, () =>
      ipc.otherHostStartWorkspace(hostId, project.id, message, chosen),
    );
    if (started && from) {
      const projects = useProjectsStore.getState();
      projects.remember(PROJECT_THERE, {
        ...recall<Record<string, string>>(projects.ui, PROJECT_THERE, {}),
        [thereKey(hostId, from)]: project.id,
      });
    }
    // Started, it has left for the new workspace; not started, the message is still here.
    starting.current = false;
    setBusy(false);
  };

  const chooseProject = (id: string) =>
    useProjectsStore.getState().composeThere({ hostId, projectId: id || null, from, message });

  /** Another computer, or this one: what was typed goes along, and so does the project. */
  const runOn = (target: string | null) => {
    const projects = useProjectsStore.getState();
    if (target === null) {
      if (here) projects.compose(here.id, undefined, message || undefined);
      return;
    }
    const like = local ?? (project ? { id: null, name: project.name } : null);
    const there = like
      ? offeredThere(useOtherComputersStore.getState().views[target], projects.ui, target, like)
      : null;
    projects.composeThere({ hostId: target, projectId: there?.projectId ?? null, from, message });
  };

  return (
    <div className="flex h-full items-center justify-center overflow-y-auto p-6">
      <form
        aria-label={`New workspace on ${name}`}
        className="w-full max-w-2xl"
        onSubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <div className="mb-3 text-center text-ink-muted">
          <h1 className="inline">
            New workspace in{" "}
            <span className="font-medium text-ink">
              {local?.name ?? project?.name ?? "a project"}
            </span>{" "}
            on
          </h1>{" "}
          <RunOn
            hostId={hostId}
            here={here ? null : `no project called ${project?.name ?? local?.name ?? "that"} here`}
            onChoose={runOn}
            disabled={busy}
          />
        </div>
        {view && view.projects.length > 0 && (
          <div className="mb-2 flex flex-wrap items-center justify-center gap-2 text-detail text-ink-faint">
            <label htmlFor={projectFieldId}>Project on {name}</label>
            <select
              id={projectFieldId}
              value={project?.id ?? ""}
              disabled={busy}
              onChange={(event) => chooseProject(event.target.value)}
              className="h-7 max-w-56 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50"
            >
              {!project && <option value="">choose one…</option>}
              {view.projects.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {candidate.name}
                </option>
              ))}
            </select>
            {why && <span>{why}</span>}
          </div>
        )}
        <textarea
          ref={messageRef}
          aria-label="What should the agent work on?"
          placeholder="What should the agent work on?"
          value={message}
          rows={5}
          disabled={busy}
          onChange={(event) => setMessage(event.target.value)}
          onKeyDown={(event) => {
            // Enter sends, Shift+Enter breaks the line — and never while an IME is composing.
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void start();
            } else if (event.key === "Escape") {
              useProjectsStore.getState().composeThere(null);
            }
          }}
          className="w-full resize-none rounded-lg border border-line bg-surface p-3 text-[15px] leading-relaxed outline-none select-text focus:border-accent disabled:opacity-60"
        />

        <div className="mt-2 flex flex-wrap items-center gap-2">
          <span className="relative flex items-center">
            {chosen && (
              <HarnessIcon
                id={chosen}
                label={agents?.find((a) => a.id === chosen)?.name}
                className="pointer-events-none absolute left-2"
              />
            )}
            <select
              aria-label="Harness"
              value={chosen ?? ""}
              disabled={busy || !agents || agents.length === 0}
              onChange={(event) => setAgent(event.target.value)}
              className={`${control} ${chosen ? "pl-7" : ""}`}
            >
              {(agents ?? []).map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </select>
          </span>
          <button
            type="submit"
            disabled={!ready}
            className="ml-auto h-9 rounded bg-accent px-4 font-medium text-canvas disabled:opacity-40"
          >
            {busy ? "Starting…" : "Start"}
          </button>
        </div>

        <p className="mt-2 text-detail text-ink-faint">
          The agents listed are {name}&rsquo;s own. It names the branch after your message, starts
          from its default branch, adds its project memory and runs its setup, as its settings say.
        </p>

        <div className="mt-3 min-h-10 text-center">
          {problem ? (
            <p role="alert" className="text-red-400 select-text">
              {problem}
            </p>
          ) : view && !online ? (
            <p role="alert" className="text-amber-300">
              {name} is {linkText(view.link)}. Nothing can be started there until it is back, and
              nothing is queued for it.
            </p>
          ) : view && !view.mayStart ? (
            <p role="alert" className="text-amber-300">
              {name} lets this computer watch, not start work.
            </p>
          ) : view && !project ? (
            <p role="alert" className="text-amber-300">
              {local
                ? `${name} has no project called ${local.name}.`
                : `${name} no longer shows that project.`}{" "}
              Add it there, with Add project in its window or{" "}
              <span className="font-mono">ys project add</span> in its terminal, and it is offered
              here.{view.projects.length > 0 && " Or choose another of its projects above."}
            </p>
          ) : agents?.length === 0 ? (
            <p role="alert" className="text-red-400">
              {name} has no agent it can start. Install one there first.
            </p>
          ) : busy ? (
            <p className="text-ink-faint">
              Making the workspace and starting the agent on {name}. Its setup can take a while.
            </p>
          ) : (
            <p className="text-ink-faint">
              Enter to start · Shift+Enter for a new line · Esc to cancel. A new branch and git
              worktree are created on {name} when you start. Nothing is created on this computer.
            </p>
          )}
        </div>
      </form>
    </div>
  );
}
