import { useEffect } from "react";
import type { OtherHostView as View, OtherSession, OtherTerminal, OtherWorkspace } from "@/lib/ipc";
import { clock, linkText } from "@/features/sidebar/otherComputersText";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { useProjectsStore } from "@/stores/projects";

/**
 * Another computer's workspaces, agents and activity, as this one last read them through the
 * relay, in the center panel. Read-only: nothing here acts on that computer or on this one. Its
 * name is across the top, so none of it is taken for this computer's own.
 */
export function OtherHostView({ hostId }: { hostId: string }) {
  const view = useOtherComputersStore((s) => s.views[hostId]);
  const name = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );

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
            What runs there, read through the relay. Read-only here: nothing on this page acts on
            that computer or on this one.
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
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {view === undefined ? null : view === null ? (
          <p className="text-ink-faint">
            This computer cannot reach {title} now. Settings → Remote → Other computers says why.
          </p>
        ) : (
          <Shown view={view} />
        )}
      </div>
    </section>
  );
}

function Shown({ view }: { view: View }) {
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
                <Workspace key={workspace.id} workspace={workspace} />
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

function Workspace({ workspace }: { workspace: OtherWorkspace }) {
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
            <Session key={session.id} session={session} />
          ))}
          {workspace.terminals.map((terminal) => (
            <Terminal key={terminal.id} terminal={terminal} />
          ))}
        </ul>
      )}
    </li>
  );
}

function Session({ session }: { session: OtherSession }) {
  return (
    <li className="flex gap-2">
      <span className="text-ink-muted">{session.harness}</span>
      <span className="min-w-0 flex-1 truncate text-ink">{session.title}</span>
      <span className="text-ink-faint">{session.state}</span>
    </li>
  );
}

function Terminal({ terminal }: { terminal: OtherTerminal }) {
  return (
    <li className="flex gap-2">
      <span className="text-ink-muted">{terminal.purpose}</span>
      <span className="min-w-0 flex-1 truncate font-mono text-ink">{terminal.program}</span>
      <span className="text-ink-faint">{terminal.state}</span>
    </li>
  );
}
