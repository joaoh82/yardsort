import { useEffect, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { TerminalView } from "@/features/terminal/TerminalView";
import { hasCore, ipc } from "@/lib/ipc";
import { useCoordinatorStore } from "@/stores/coordinator";
import { launchable, useHarnessStore } from "@/stores/harnesses";
import { useLayoutStore } from "@/stores/layout";
import { useShortcutLabel } from "@/stores/preferences";

/** What the agent is told its terminal is, before the panel has measured it. */
const FIRST_SIZE = { cols: 80, rows: 30 };

/** The status bar's way in: the lower-right corner, on every screen. */
export function FloorManagerButton() {
  const open = useCoordinatorStore((s) => s.open);
  const running = useCoordinatorStore((s) => s.status?.session?.state.status === "running");
  const shortcut = useShortcutLabel("floorManager");
  return (
    <button
      type="button"
      aria-pressed={open}
      title={`Ask about every project, or have work started (${shortcut})`}
      onClick={() => useCoordinatorStore.getState().toggle()}
      className="flex items-center gap-1 rounded px-1.5 py-0.5 font-sans text-ink-muted hover:bg-raised hover:text-ink aria-pressed:bg-raised aria-pressed:text-ink"
    >
      {running && <span aria-hidden className="size-1.5 rounded-full bg-accent" />}
      Floor Manager
    </button>
  );
}

/**
 * The Floor Manager: one agent with every project in view, in a panel over the lower-right
 * corner. Not modal, the rest of the window keeps working beside it, and closing it leaves the
 * agent running, as closing the window does.
 */
export function FloorManagerPanel() {
  const status = useCoordinatorStore((s) => s.status);
  const error = useCoordinatorStore((s) => s.error);
  const busy = useCoordinatorStore((s) => s.busy);
  const session = status?.session ?? null;
  const running = session?.state.status === "running";
  // Asking is about one conversation: a new one is not asked about until Stop is pressed again.
  const [askingFor, setAskingFor] = useState<string | null>(null);
  const confirmStop = running && askingFor === session?.id;

  useEffect(() => {
    if (!hasCore()) return;
    void useHarnessStore.getState().load();
    const unlisten = ipc.onHostEvent((event) => useCoordinatorStore.getState().observe(event));
    return () => void unlisten.then((stop) => stop());
  }, []);

  const close = () => useCoordinatorStore.getState().setOpen(false);
  const button =
    "rounded border border-line px-2 py-0.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40";

  return (
    <aside
      aria-label="Floor Manager"
      className="fixed right-2 bottom-8 z-30 flex h-[min(40rem,calc(100vh-5rem))] w-[min(36rem,calc(100vw-1rem))] flex-col overflow-hidden rounded-lg border border-line bg-surface shadow-2xl shadow-black/50"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-line px-3 py-2">
        <h2 className="font-medium text-ink">Floor Manager</h2>
        {status?.harness && (
          <span className="flex items-center gap-1 text-xs text-ink-faint">
            <HarnessIcon id={status.harness} label={status.harness} />
            {status.harness} · {running ? "running" : "ended"}
          </span>
        )}
        <span className="flex-1" />
        {running &&
          (confirmStop ? (
            <>
              <span className="text-xs text-ink-muted">End its conversation?</span>
              <button
                type="button"
                disabled={busy}
                onClick={() => void useCoordinatorStore.getState().stop()}
                className={button}
              >
                End it
              </button>
              <button type="button" onClick={() => setAskingFor(null)} className={button}>
                Keep it
              </button>
            </>
          ) : (
            <button
              type="button"
              title="End the Floor Manager's conversation"
              onClick={() => setAskingFor(session?.id ?? null)}
              className={button}
            >
              Stop
            </button>
          ))}
        <button
          type="button"
          aria-label="Close the Floor Manager"
          title="Close. It keeps running."
          onClick={close}
          className="rounded px-1.5 text-ink-muted hover:bg-raised hover:text-ink"
        >
          ×
        </button>
      </header>
      {error && (
        <p role="alert" className="shrink-0 border-b border-line px-3 py-1.5 text-xs text-red-400">
          {error}
        </p>
      )}
      {session && !running && <StartBar again />}
      <div className="min-h-0 flex-1">
        {session ? (
          <TerminalView key={session.id} sessionId={session.id} agent />
        ) : status ? (
          <Welcome hasYs={status.hasYs} />
        ) : null}
      </div>
    </aside>
  );
}

/** Before the first start: what it is, and which agent to run it with. */
function Welcome({ hasYs }: { hasYs: boolean }) {
  return (
    <div className="flex h-full flex-col gap-3 overflow-y-auto p-4 text-ink-muted">
      <p className="text-ink">One agent that sees every project.</p>
      <p>Ask it, for example:</p>
      <ul className="list-disc space-y-1 pl-5">
        <li>What projects do we have, and what is running in them?</li>
        <li>Which pull requests are open and not reviewed?</li>
        <li>Check the open pull requests and start reviewers on them.</li>
        <li>Start this task: suggest a workflow for it.</li>
      </ul>
      <p>
        It reads through <code className="text-ink">ys</code>, the command line. It proposes before
        it starts an agent, a workflow or anything that writes to GitHub, and waits for your yes. It
        never deletes, merges or closes anything.
      </p>
      {!hasYs && (
        <p className="text-amber-400">
          This copy of Yardsort has no <code>ys</code> of its own, so the agent will use whichever
          one is on your PATH, if any.
        </p>
      )}
      <StartBar />
    </div>
  );
}

/** Choose the agent and start it: first time, or again after it ended. */
function StartBar({ again = false }: { again?: boolean }) {
  const harnesses = useHarnessStore((s) => s.harnesses);
  const loaded = useHarnessStore((s) => s.loaded);
  const chosen = useCoordinatorStore((s) => s.chosen);
  const ran = useCoordinatorStore((s) => s.status?.harness ?? null);
  const busy = useCoordinatorStore((s) => s.busy);
  const ready = launchable(harnesses);
  const wanted = chosen ?? ran;
  const harness = ready.find((h) => h.id === wanted) ?? ready[0];

  if (loaded && !harness) {
    return (
      <div className="flex items-center gap-2 border-b border-line bg-raised px-3 py-2 text-ink-muted">
        <span className="flex-1">No agent is installed and switched on to run it with.</span>
        <button
          type="button"
          onClick={() => useLayoutStore.getState().openSettings("harnesses")}
          className="rounded border border-line px-2 py-0.5 hover:border-accent hover:text-ink"
        >
          Settings → Harnesses
        </button>
      </div>
    );
  }
  return (
    <div
      role={again ? "status" : undefined}
      className={`flex shrink-0 items-center gap-2 ${again ? "border-b border-line bg-raised px-3 py-1.5" : ""}`}
    >
      {again && <span className="flex-1 text-ink-muted">The Floor Manager has ended.</span>}
      <label className="flex items-center gap-1.5 text-ink-muted">
        <span className={again ? "sr-only" : ""}>Agent</span>
        <select
          value={harness?.id ?? ""}
          onChange={(event) => useCoordinatorStore.getState().choose(event.target.value)}
          className="rounded border border-line bg-canvas px-1.5 py-0.5 text-ink"
        >
          {ready.map((h) => (
            <option key={h.id} value={h.id}>
              {h.label}
            </option>
          ))}
        </select>
      </label>
      <button
        type="button"
        disabled={!harness || busy}
        onClick={() => harness && void useCoordinatorStore.getState().start(harness.id, FIRST_SIZE)}
        className="rounded bg-accent px-3 py-0.5 font-medium text-canvas disabled:opacity-40"
      >
        {again ? "Start again" : "Start"}
      </button>
    </div>
  );
}
