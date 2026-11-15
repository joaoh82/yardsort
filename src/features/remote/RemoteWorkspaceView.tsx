import { useEffect, useState } from "react";
import type { OtherHarness, OtherSession } from "@/lib/ipc";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { ComputerIcon } from "@/features/sidebar/OtherComputersTree";
import { clock } from "@/features/sidebar/otherComputersText";
import { StatusDot } from "@/features/terminal/StatusDot";
import { timeAgo } from "@/features/terminal/sessionText";
import {
  findThere,
  tabsThere,
  thereKey,
  type There,
  type ThereTab,
  useOtherComputersStore,
} from "@/stores/otherComputers";
import { ComputerBar, RemoteTerminal } from "./RemoteTerminal";
import { resumeThere, startAgentThere, tabActivity } from "./there";

/**
 * A workspace on another computer, in the center panel a local workspace gets: that computer and
 * who has control across the top, its terminals as tabs, the agents that computer can start, the
 * terminal, and its ended conversations when nothing runs. What is started here runs there, by
 * that computer's ids and with its own settings. Its name is in the bar and in the bottom line,
 * so none of it is taken for this computer's own.
 */
export function RemoteWorkspaceView({
  hostId,
  workspaceId,
}: {
  hostId: string;
  workspaceId: string;
}) {
  const views = useOtherComputersStore((s) => s.views);
  const hostName = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );
  const activeId = useOtherComputersStore((s) => s.active[thereKey(hostId, workspaceId)]);
  const harnesses = useOtherComputersStore((s) => s.harnesses[hostId]);
  const problem = useOtherComputersStore((s) => s.startProblem);
  // Started from here a moment ago: shown at once, before that computer's next reading lists it.
  const startedHere = useOtherComputersStore(
    (s) => activeId !== undefined && thereKey(hostId, activeId) in s.startedHere,
  );
  const there = findThere(views, hostId, workspaceId);
  const view = views[hostId];
  const name = view?.name || hostName || "Another computer";
  const mayStart = !!view?.mayStart && view.link.kind === "online";

  useEffect(() => {
    if (mayStart) void useOtherComputersStore.getState().loadHarnesses(hostId);
  }, [mayStart, hostId]);

  if (!there) {
    return (
      <section aria-label={`A workspace on ${name}`} className="flex h-full min-h-0 flex-col">
        {view && <ComputerBar name={name} link={view.link} />}
        <p className="p-4 text-ink-faint">
          {view === undefined
            ? null
            : view === null || view.readAt === null
              ? `This computer cannot reach ${name} now. Settings → Remote → Other computers says why.`
              : startedHere
                ? `Starting on ${name}…`
                : `${name} no longer shows this workspace to this computer.`}
        </p>
      </section>
    );
  }

  const { workspace, project } = there;
  const listed = tabsThere(workspace);
  const tabs: ThereTab[] =
    startedHere && activeId !== undefined && !listed.some((tab) => tab.id === activeId)
      ? [...listed, STARTING(activeId)]
      : listed;
  const active = tabs.find((tab) => tab.id === activeId) ?? tabs[0];
  const stale = there.view.link.kind !== "online";
  const between = (
    <>
      <Tabs
        hostId={hostId}
        workspaceId={workspaceId}
        tabs={tabs}
        activeId={active?.id}
        harnesses={mayStart ? harnesses : undefined}
      />
      {problem && (
        <div
          role="alert"
          className="flex items-start gap-3 border-b border-line bg-raised px-3 py-2"
        >
          <p className="flex-1 text-red-400 select-text">{problem}</p>
          <button
            type="button"
            onClick={() => useOtherComputersStore.getState().couldNotStart(null)}
            className="text-ink-faint hover:text-ink"
          >
            Dismiss
          </button>
        </div>
      )}
      {stale && there.view.readAt !== null && (
        <p role="status" className="border-b border-line px-3 py-1 text-amber-300 select-text">
          {there.view.link.kind === "offline" ? `${there.view.link.why}. ` : ""}
          Shown as {name} last showed it, at {clock(there.view.readAt)}.
        </p>
      )}
      {active && <Ended hostId={hostId} there={there} tab={active} />}
    </>
  );

  return (
    <section aria-label={`${workspace.name} on ${name}`} className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1">
        {active ? (
          <RemoteTerminal
            key={active.id}
            hostId={hostId}
            hostName={name}
            link={there.view.link}
            terminal={{
              id: active.id,
              program: active.title,
              purpose: active.purpose,
              state: active.state,
            }}
            mayType={there.view.mayType}
            between={between}
          />
        ) : (
          <div className="flex h-full min-h-0 flex-col">
            <ComputerBar name={name} link={there.view.link} />
            {between}
            <div className="flex min-h-0 flex-1 flex-col items-center justify-center overflow-y-auto p-6 text-center">
              <p className="text-ink-muted">Nothing running in this workspace.</p>
              {mayStart && harnesses && harnesses.length > 0 && (
                <div className="mt-4 flex flex-wrap justify-center gap-2">
                  {harnesses.map((harness) => (
                    <StartAgent
                      key={harness.id}
                      hostId={hostId}
                      workspaceId={workspaceId}
                      harness={harness}
                      className="inline-flex items-center gap-1.5 rounded border border-line px-3 py-1.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
                    />
                  ))}
                </div>
              )}
              {mayStart && harnesses?.length === 0 && (
                <p className="mt-2 text-ink-faint">
                  {name} has no agent it can start. Install one there first.
                </p>
              )}
              <History hostId={hostId} there={there} />
            </div>
          </div>
        )}
      </div>
      <footer className="flex h-7 shrink-0 items-center gap-2 border-t border-line bg-surface px-3.5 font-mono text-small whitespace-nowrap text-ink-faint *:shrink-0">
        <ComputerIcon className="size-3.5" />
        <span className="text-ink-muted">{name}</span>
        <span>/</span>
        <span className="text-ink-muted">{project.name}</span>
        <span>/</span>
        <span className="text-ink-muted">{workspace.name}</span>
        {workspace.branch && <span title="Branch">· {workspace.branch}</span>}
        <span className="ml-auto min-w-0 shrink! truncate">through the relay</span>
      </footer>
    </section>
  );
}

/** An agent started from here, until that computer's next reading says what it is. */
const STARTING = (id: string): ThereTab => ({
  id,
  title: "starting…",
  harness: null,
  about: null,
  purpose: "agent",
  state: "starting",
});

/** That workspace's terminals as tabs, and the agents its computer can start beside them. */
function Tabs({
  hostId,
  workspaceId,
  tabs,
  activeId,
  harnesses,
}: {
  hostId: string;
  workspaceId: string;
  tabs: ThereTab[];
  activeId: string | undefined;
  harnesses: OtherHarness[] | undefined;
}) {
  return (
    <div className="flex h-10 shrink-0 items-stretch border-b border-line bg-surface">
      <div role="tablist" aria-label="Terminals" className="flex min-w-0 items-stretch">
        {tabs.map((tab) => {
          const active = tab.id === activeId;
          return (
            <div
              key={tab.id}
              className={`flex items-center border-r border-line ${
                active ? "bg-canvas text-ink" : "text-ink-muted hover:bg-raised"
              }`}
            >
              <button
                type="button"
                role="tab"
                aria-selected={active}
                title={tab.about ?? undefined}
                onClick={() =>
                  useOtherComputersStore.getState().activate(hostId, workspaceId, tab.id)
                }
                className="flex h-full items-center gap-2 px-3"
              >
                <StatusDot activity={tabActivity(tab)} />
                {tab.harness && <HarnessIcon id={tab.harness} label={tab.harness} size={14} />}
                <span className="max-w-40 truncate">{tab.title}</span>
                {tab.state === "exited" && <span className="text-small text-ink-faint">ended</span>}
              </button>
            </div>
          );
        })}
      </div>
      {harnesses && (
        <div className="ml-auto flex items-center gap-1 pr-2">
          {harnesses.map((harness) => (
            <StartAgent
              key={harness.id}
              hostId={hostId}
              workspaceId={workspaceId}
              harness={harness}
              iconSize={12}
              className="flex items-center gap-1.5 rounded px-2 py-0.5 text-small text-ink-faint hover:bg-raised hover:text-ink disabled:opacity-40"
            />
          ))}
        </div>
      )}
    </div>
  );
}

/** Start one of that computer's agents in a workspace there, as the local tab bar's buttons do. */
function StartAgent({
  hostId,
  workspaceId,
  harness,
  className,
  iconSize = 16,
}: {
  hostId: string;
  workspaceId: string;
  harness: OtherHarness;
  className: string;
  iconSize?: number;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      disabled={busy}
      aria-label={`Start ${harness.name}`}
      title={`Start ${harness.name} in this workspace, on that computer`}
      onClick={() => {
        setBusy(true);
        void startAgentThere(hostId, workspaceId, harness.id).finally(() => setBusy(false));
      }}
      className={className}
    >
      <HarnessIcon id={harness.id} label={harness.name} size={iconSize} />
      {harness.name}
    </button>
  );
}

/** Over an agent's terminal there whose program has ended: carry on from here. */
function Ended({ hostId, there, tab }: { hostId: string; there: There; tab: ThereTab }) {
  const session = there.workspace.sessions.find((candidate) => candidate.terminal === tab.id);
  if (!session || tab.state !== "exited") return null;
  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b border-line bg-raised px-3 py-1.5"
    >
      <HarnessIcon id={session.harness} label={session.harness} />
      <span className="min-w-0 flex-1 truncate text-ink-muted">{session.harness} ended.</span>
      {there.view.mayResume && session.resumable && (
        <Resume hostId={hostId} session={session} primary />
      )}
    </div>
  );
}

/** The conversations a workspace there has had that are not running now. */
function History({ hostId, there }: { hostId: string; there: There }) {
  const past = there.workspace.sessions.filter((session) => session.terminal === null);
  if (past.length === 0) return null;
  return (
    <section aria-label="Previous sessions" className="mx-auto mt-6 w-full max-w-xl text-left">
      <h2 className="mb-2 text-small font-semibold tracking-wider text-ink-faint uppercase">
        Previous sessions
      </h2>
      <ul className="divide-y divide-line rounded-md border border-line">
        {past.map((session, index) => (
          <li key={session.id} className="flex items-center gap-3 px-3 py-2">
            <HarnessIcon id={session.harness} label={session.harness} />
            <div className="min-w-0 flex-1">
              <div className="truncate">
                <span className="font-medium">{session.harness}</span>
                {session.title && <span className="text-ink-muted"> — {session.title}</span>}
              </div>
              {session.startedAt !== null && (
                <div className="truncate text-small text-ink-faint">
                  started {timeAgo(session.startedAt)}
                </div>
              )}
            </div>
            {there.view.mayResume && session.resumable && (
              <Resume hostId={hostId} session={session} primary={index === 0} />
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function Resume({
  hostId,
  session,
  primary,
}: {
  hostId: string;
  session: OtherSession;
  primary?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      disabled={busy}
      aria-label={`Resume ${session.title || session.harness}`}
      title="Continue this conversation, on that computer"
      onClick={() => {
        setBusy(true);
        void resumeThere(hostId, session.id).finally(() => setBusy(false));
      }}
      className={`shrink-0 rounded px-3 py-1 disabled:opacity-40 ${
        primary
          ? "bg-accent font-medium text-canvas"
          : "border border-line text-ink-muted hover:border-accent hover:text-ink"
      }`}
    >
      {busy ? "Resuming…" : "Resume"}
    </button>
  );
}
