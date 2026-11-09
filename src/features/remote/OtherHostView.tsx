import { useEffect, useState } from "react";
import {
  ipc,
  errorMessage,
  type OtherHarness,
  type OtherHostView as View,
  type OtherSession,
  type OtherTerminal,
  type OtherWorkspace,
} from "@/lib/ipc";
import { clock, linkText } from "@/features/sidebar/otherComputersText";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";
import { RemoteTerminal } from "./RemoteTerminal";

/**
 * Another computer's workspaces, agents and activity, as this one last read them through the
 * relay, in the center panel. Its terminals open here to watch, and to type into once this
 * computer takes control, when that computer allows it. When it does, work can be started there
 * too: a new workspace in one of its projects, another agent in one of its workspaces, or one of
 * its ended conversations again, each by that computer's ids and with its own settings. Nothing
 * here acts on this computer. Its name is across the top, so none of it is taken for this
 * computer's own.
 */
export function OtherHostView({ hostId }: { hostId: string }) {
  const view = useOtherComputersStore((s) => s.views[hostId]);
  const name = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );
  const [watching, setWatching] = useState<{ host: string; terminal: OtherTerminal } | null>(null);
  const [asking, setAsking] = useState<{ host: string; start: Start } | null>(null);
  // Another host opened in this panel watches none of this one's terminals, and starts nothing.
  const open = watching?.host === hostId ? watching.terminal : null;
  const start = asking?.host === hostId ? asking.start : null;

  useEffect(() => {
    void useOtherComputersStore.getState().loadHost(hostId);
  }, [hostId]);

  const title = view?.name || name || "Another computer";
  return (
    <section aria-label={`${title}, another computer`} className="flex h-full min-h-0 flex-col">
      <header
        role="banner"
        aria-label="Another computer"
        className="flex items-center gap-3 border-b border-accent/40 bg-accent/10 px-4 py-2"
      >
        <div className="min-w-0 flex-1">
          <p className="text-ink">
            <span className="font-semibold">{title}</span>
            <span className="text-ink-muted"> · another computer</span>
          </p>
          <p className="text-[12px] text-ink-faint">
            What runs there, through the relay. Its terminals can be watched here, and typed into
            only once this computer takes control. What is started here runs there. Nothing here
            acts on this computer.
          </p>
        </div>
        <button
          type="button"
          onClick={() => useProjectsStore.getState().openRemoteHost(null)}
          className="rounded border border-line px-2 py-0.5 text-ink-muted hover:border-accent hover:text-ink"
        >
          Close
        </button>
      </header>
      {open && view ? (
        <div className="min-h-0 flex-1">
          <RemoteTerminal
            hostId={hostId}
            hostName={title}
            terminal={open}
            mayType={view.mayType}
            onBack={() => setWatching(null)}
          />
        </div>
      ) : start && view ? (
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          <StartForm
            hostId={hostId}
            hostName={title}
            start={start}
            onBack={() => setAsking(null)}
            onStarted={(terminal) => {
              setAsking(null);
              setWatching({ host: hostId, terminal });
              void useOtherComputersStore.getState().loadHost(hostId);
            }}
          />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {view === undefined ? null : view === null ? (
            <p className="text-ink-faint">
              This computer cannot reach {title} now. Settings → Remote → Other computers says why.
            </p>
          ) : (
            <Shown
              view={view}
              acts={{
                watch: (terminal) => setWatching({ host: hostId, terminal }),
                start: (start) => setAsking({ host: hostId, start }),
              }}
            />
          )}
        </div>
      )}
    </section>
  );
}

type Watch = (terminal: OtherTerminal) => void;

/** Work to start on that computer, by its ids. */
type Start =
  | { kind: "workspace"; projectId: string; projectName: string }
  | { kind: "session"; workspaceId: string; workspaceName: string }
  | { kind: "resume"; sessionId: string; title: string; harness: string };

/** What a row may do: watch a terminal, and, when that computer lets this one, start work. */
interface Acts {
  watch: Watch;
  start: (start: Start) => void;
  mayStart: boolean;
  mayResume: boolean;
}

function Shown({ view, acts: given }: { view: View; acts: Omit<Acts, "mayStart" | "mayResume"> }) {
  const acts: Acts = { ...given, mayStart: view.mayStart, mayResume: view.mayResume };
  const stale = view.link.kind !== "online";
  return (
    <div className="grid gap-5">
      <p role="status" className={stale ? "text-amber-300 select-text" : "text-ink-muted"}>
        {linkLine(view)}
      </p>
      {view.readAt === null ? null : view.projects.length === 0 ? (
        <p className="text-ink-faint">It shows no workspaces to this computer.</p>
      ) : (
        view.projects.map((project) => (
          <section key={project.id} aria-label={project.name} className="grid gap-2">
            <div className="flex items-center gap-2">
              <h3 className="flex-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
                {project.name}
              </h3>
              {acts.mayStart && (
                <ActButton
                  label={`New workspace in ${project.name}`}
                  onClick={() =>
                    acts.start({
                      kind: "workspace",
                      projectId: project.id,
                      projectName: project.name,
                    })
                  }
                >
                  New workspace
                </ActButton>
              )}
            </div>
            <ul className="grid gap-2">
              {project.workspaces.map((workspace) => (
                <Workspace key={workspace.id} workspace={workspace} acts={acts} />
              ))}
            </ul>
          </section>
        ))
      )}
      {view.activity.length > 0 && (
        <section aria-label="Activity" className="grid gap-2">
          <h3 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
            Activity
          </h3>
          <ul className="grid gap-0.5 text-[12px]">
            {view.activity.map((event) => (
              <li key={event.id} className="flex gap-3">
                <span className="w-24 shrink-0 text-ink-faint tabular-nums">
                  {clock(event.occurredAt)}
                </span>
                <span className="text-ink">{event.kind}</span>
                <span className="truncate text-ink-faint">{event.workspace}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function linkLine(view: View): string {
  const read = view.readAt === null ? null : clock(view.readAt);
  const where = [view.platform, view.appVersion && `Yardsort ${view.appVersion}`]
    .filter(Boolean)
    .join(", ");
  switch (view.link.kind) {
    case "online":
      return `Online${where ? `: ${where}` : ""}.`;
    case "connecting":
      return read ? `Connecting… What it showed at ${read}:` : "Connecting…";
    case "offline":
      return `${linkText(view.link)[0]?.toUpperCase()}${linkText(view.link).slice(1)}: ${view.link.why}.${
        read ? ` What it showed at ${read}:` : ""
      }`;
  }
}

function Workspace({ workspace, acts }: { workspace: OtherWorkspace; acts: Acts }) {
  return (
    <li className="rounded border border-line px-3 py-2">
      <div className="flex items-baseline gap-2">
        <span className="text-ink">{workspace.name}</span>
        {workspace.branch && (
          <span className="font-mono text-[12px] text-ink-faint">{workspace.branch}</span>
        )}
        {workspace.archived && <span className="text-ink-faint">archived</span>}
        <span className="flex-1" />
        {acts.mayStart && !workspace.archived && (
          <ActButton
            label={`New agent in ${workspace.name}`}
            onClick={() =>
              acts.start({
                kind: "session",
                workspaceId: workspace.id,
                workspaceName: workspace.name,
              })
            }
          >
            New agent
          </ActButton>
        )}
      </div>
      {workspace.sessions.length + workspace.terminals.length > 0 && (
        <ul className="mt-1 grid gap-0.5 text-[12px]">
          {workspace.sessions.map((session) => (
            <Session key={session.id} session={session} acts={acts} />
          ))}
          {workspace.terminals.map((terminal) => (
            <Terminal key={terminal.id} terminal={terminal} onWatch={acts.watch} />
          ))}
        </ul>
      )}
    </li>
  );
}

function Session({ session, acts }: { session: OtherSession; acts: Acts }) {
  const terminal = session.terminal;
  return (
    <li className="flex items-center gap-2">
      <span className="text-ink-muted">{session.harness}</span>
      <span className="min-w-0 flex-1 truncate text-ink">{session.title}</span>
      <span className="text-ink-faint">{session.state}</span>
      {terminal && (
        <ActButton
          label={`Watch ${session.title}`}
          // An agent's terminal, named by its harness, as that computer's own tab is.
          onClick={() =>
            acts.watch({
              id: terminal,
              program: session.harness,
              purpose: "agent",
              state: session.state,
            })
          }
        >
          Watch
        </ActButton>
      )}
      {acts.mayResume && session.resumable && (
        <ActButton
          label={`Resume ${session.title}`}
          onClick={() =>
            acts.start({
              kind: "resume",
              sessionId: session.id,
              title: session.title,
              harness: session.harness,
            })
          }
        >
          Resume
        </ActButton>
      )}
    </li>
  );
}

function ActButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className="rounded border border-line px-2 py-0 text-ink-muted hover:border-accent hover:text-ink"
    >
      {children}
    </button>
  );
}

function Terminal({ terminal, onWatch }: { terminal: OtherTerminal; onWatch: Watch }) {
  return (
    <li className="flex items-center gap-2">
      <span className="text-ink-muted">{terminal.purpose}</span>
      <span className="min-w-0 flex-1 truncate font-mono text-ink">{terminal.program}</span>
      <span className="text-ink-faint">{terminal.state}</span>
      <ActButton label={`Watch ${terminal.program}`} onClick={() => onWatch(terminal)}>
        Watch
      </ActButton>
    </li>
  );
}

/** What a start's form says, by what it starts. */
function wording(start: Start, hostName: string) {
  switch (start.kind) {
    case "workspace":
      return {
        title: `New workspace in ${start.projectName}`,
        about: `A new branch and worktree in ${start.projectName}, made on ${hostName} as its own window makes them, with an agent given the message.`,
        field: "What should it do?",
        hint: "The workspace's branch is named after it.",
        go: "Start workspace",
      };
    case "session":
      return {
        title: `New agent in ${start.workspaceName}`,
        about: `Another agent in ${start.workspaceName} on ${hostName}, with the same files as the others there.`,
        field: "First message, if any",
        hint: "Without one, the agent waits for you.",
        go: "Start agent",
      };
    case "resume":
      return {
        title: `Resume ${start.title}`,
        about: `The agent on ${hostName} picks up where it was, in a new terminal there.`,
        field: "A message to go on with, if any",
        hint: "Without one, the agent waits for you.",
        go: "Resume",
      };
  }
}

/**
 * Starting work on that computer: the agent chosen from its own list, and a message. Once it has
 * started, its terminal opens to watch. A start whose answer is lost is not tried again.
 */
function StartForm({
  hostId,
  hostName,
  start,
  onBack,
  onStarted,
}: {
  hostId: string;
  hostName: string;
  start: Start;
  onBack: () => void;
  onStarted: (terminal: OtherTerminal) => void;
}) {
  const words = wording(start, hostName);
  const picks = start.kind !== "resume";
  const [agents, setAgents] = useState<OtherHarness[] | null>(null);
  const [agent, setAgent] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    if (!picks) return;
    let stale = false;
    ipc.otherHostHarnesses(hostId).then(
      (listed) => {
        if (!stale) setAgents(listed);
      },
      (error: unknown) => {
        if (!stale) setProblem(errorMessage(error));
      },
    );
    return () => {
      stale = true;
    };
  }, [hostId, picks]);

  const chosen = agent ?? agents?.[0]?.id ?? null;
  const said = message.trim() ? message : null;
  const ready =
    !busy &&
    (picks ? chosen !== null : true) &&
    (start.kind === "workspace" ? said !== null : true);

  const go = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const started =
        start.kind === "workspace"
          ? await ipc.otherHostStartWorkspace(hostId, start.projectId, message, chosen)
          : start.kind === "session"
            ? await ipc.otherHostStartSession(hostId, start.workspaceId, chosen, said)
            : await ipc.otherHostResume(hostId, start.sessionId, said);
      onStarted({
        id: started.terminal,
        program: started.harness,
        purpose: "agent",
        state: "starting",
      });
    } catch (error) {
      setBusy(false);
      setProblem(errorMessage(error));
    }
  };

  return (
    <section aria-label={words.title} className="grid max-w-xl gap-3">
      <h3 className="font-semibold text-ink">{words.title}</h3>
      <p className="text-ink-muted">{words.about}</p>
      {picks && agents?.length === 0 && (
        <p className="text-ink-faint">
          {hostName} has no agent it can start. Install one there first.
        </p>
      )}
      {picks && agents && agents.length > 0 && (
        <label className="grid gap-1">
          <span className="text-[12px] text-ink-muted">Agent</span>
          <select
            value={chosen ?? ""}
            onChange={(e) => setAgent(e.target.value)}
            disabled={busy}
            className="rounded border border-line bg-surface px-2 py-1 text-ink"
          >
            {agents.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="grid gap-1">
        <label className="grid gap-1">
          <span className="text-[12px] text-ink-muted">{words.field}</span>
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            disabled={busy}
            rows={4}
            className="rounded border border-line bg-surface px-2 py-1 text-ink"
          />
        </label>
        <p className="text-[12px] text-ink-faint">{words.hint}</p>
      </div>
      {problem && (
        <p role="alert" className="text-red-300 select-text">
          {problem}
        </p>
      )}
      {busy && start.kind === "workspace" && (
        <p className="text-ink-faint">
          Making the workspace and starting the agent there. Its setup can take a while.
        </p>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          onClick={() => void go()}
          disabled={!ready}
          className="rounded bg-accent px-3 py-1 text-white disabled:opacity-50"
        >
          {busy ? `${words.go}…` : words.go}
        </button>
        <button
          type="button"
          onClick={onBack}
          disabled={busy}
          className="rounded border border-line px-3 py-1 text-ink-muted hover:text-ink"
        >
          Back
        </button>
      </div>
    </section>
  );
}
