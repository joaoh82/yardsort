import { useEffect, useState } from "react";
import type { OtherHostView as View, OtherSession, OtherTerminal, OtherWorkspace } from "@/lib/ipc";
import { clock, linkText } from "@/features/sidebar/otherComputersText";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";
import { RemoteTerminal } from "./RemoteTerminal";

/**
 * Another computer's workspaces, agents and activity, as this one last read them through the
 * relay, in the center panel. Its terminals open here to watch, and to type into once this
 * computer takes control, when that computer allows it; nothing else acts on it, and nothing here
 * acts on this computer. Its name is across the top, so none of it is taken for this computer's own.
 */
export function OtherHostView({ hostId }: { hostId: string }) {
  const view = useOtherComputersStore((s) => s.views[hostId]);
  const name = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );
  const [watching, setWatching] = useState<{ host: string; terminal: OtherTerminal } | null>(null);
  // Another host opened in this panel watches none of this one's terminals.
  const open = watching?.host === hostId ? watching.terminal : null;

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
            only once this computer takes control. Nothing here acts on this computer.
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
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {view === undefined ? null : view === null ? (
            <p className="text-ink-faint">
              This computer cannot reach {title} now. Settings → Remote → Other computers says why.
            </p>
          ) : (
            <Shown view={view} onWatch={(terminal) => setWatching({ host: hostId, terminal })} />
          )}
        </div>
      )}
    </section>
  );
}

type Watch = (terminal: OtherTerminal) => void;

function Shown({ view, onWatch }: { view: View; onWatch: Watch }) {
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
            <h3 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
              {project.name}
            </h3>
            <ul className="grid gap-2">
              {project.workspaces.map((workspace) => (
                <Workspace key={workspace.id} workspace={workspace} onWatch={onWatch} />
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

function Workspace({ workspace, onWatch }: { workspace: OtherWorkspace; onWatch: Watch }) {
  return (
    <li className="rounded border border-line px-3 py-2">
      <p className="flex items-baseline gap-2">
        <span className="text-ink">{workspace.name}</span>
        {workspace.branch && (
          <span className="font-mono text-[12px] text-ink-faint">{workspace.branch}</span>
        )}
        {workspace.archived && <span className="text-ink-faint">archived</span>}
      </p>
      {workspace.sessions.length + workspace.terminals.length > 0 && (
        <ul className="mt-1 grid gap-0.5 text-[12px]">
          {workspace.sessions.map((session) => (
            <Session key={session.id} session={session} onWatch={onWatch} />
          ))}
          {workspace.terminals.map((terminal) => (
            <Terminal key={terminal.id} terminal={terminal} onWatch={onWatch} />
          ))}
        </ul>
      )}
    </li>
  );
}

function Session({ session, onWatch }: { session: OtherSession; onWatch: Watch }) {
  const terminal = session.terminal;
  return (
    <li className="flex items-center gap-2">
      <span className="text-ink-muted">{session.harness}</span>
      <span className="min-w-0 flex-1 truncate text-ink">{session.title}</span>
      <span className="text-ink-faint">{session.state}</span>
      {terminal && (
        <WatchButton
          label={`Watch ${session.title}`}
          // An agent's terminal, named by its harness, as that computer's own tab is.
          onClick={() =>
            onWatch({
              id: terminal,
              program: session.harness,
              purpose: "agent",
              state: session.state,
            })
          }
        />
      )}
    </li>
  );
}

function WatchButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className="rounded border border-line px-2 py-0 text-ink-muted hover:border-accent hover:text-ink"
    >
      Watch
    </button>
  );
}

function Terminal({ terminal, onWatch }: { terminal: OtherTerminal; onWatch: Watch }) {
  return (
    <li className="flex items-center gap-2">
      <span className="text-ink-muted">{terminal.purpose}</span>
      <span className="min-w-0 flex-1 truncate font-mono text-ink">{terminal.program}</span>
      <span className="text-ink-faint">{terminal.state}</span>
      <WatchButton label={`Watch ${terminal.program}`} onClick={() => onWatch(terminal)} />
    </li>
  );
}
