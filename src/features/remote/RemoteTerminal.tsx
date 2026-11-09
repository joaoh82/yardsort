import { useMemo, useState } from "react";
import { ipc, type OtherTerminal, type RemoteTerminalSeen } from "@/lib/ipc";
import { TerminalSurface } from "@/features/terminal/TerminalView";
import type { TerminalSource } from "@/features/terminal/source";
import { controllerText, type RemoteState, seenBy, WATCHING } from "./remoteTerminalState";

/** Another computer's terminal through the relay, as it draws there. */
class RemoteSource implements TerminalSource {
  readonly key: string;
  #id: number | null = null;

  constructor(
    readonly hostId: string,
    readonly terminal: OtherTerminal,
    readonly onSeen: (seen: RemoteTerminalSeen) => void,
  ) {
    this.key = `${hostId}/${terminal.id}`;
  }

  async attach(_size: unknown, sink: Parameters<TerminalSource["attach"]>[1]) {
    const id = await ipc.remoteTerminalOpen(this.hostId, this.terminal.id, sink.output, (seen) => {
      if (seen.kind === "reset") sink.reset(seen.size);
      else if (seen.kind === "resized") sink.size(seen.size);
      this.onSeen(seen);
    });
    this.#id = id;
    return () => {
      this.#id = null;
      void ipc.remoteTerminalClose(id).catch(() => {});
    };
  }

  write(data: string) {
    return this.#id === null ? Promise.resolve() : ipc.remoteTerminalWrite(this.#id, data);
  }

  resize(size: { cols: number; rows: number }) {
    return this.#id === null ? Promise.resolve() : ipc.remoteTerminalResize(this.#id, size);
  }

  isAgent() {
    return this.terminal.purpose === "agent";
  }

  takeControl() {
    if (this.#id !== null) void ipc.remoteTerminalTakeControl(this.#id).catch(console.error);
  }

  release() {
    if (this.#id !== null) void ipc.remoteTerminalRelease(this.#id).catch(console.error);
  }
}

/**
 * One terminal of another computer: watched as it draws there, at its size there, and typed into
 * only once this computer takes control. The person at that computer sees who has it; taking it is
 * always a press here, never a keystroke.
 */
export function RemoteTerminal(props: {
  hostId: string;
  hostName: string;
  terminal: OtherTerminal;
  mayType: boolean;
  onBack: () => void;
}) {
  const { hostId, hostName, terminal, mayType, onBack } = props;
  const [state, setState] = useState<RemoteState>(WATCHING);
  const source = useMemo(
    () => new RemoteSource(hostId, terminal, (seen) => setState((s) => seenBy(s, seen))),
    // The same terminal is the same source, whatever else its listing says now.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [hostId, terminal.id],
  );
  const { held, asking, notice, away, gone, exit } = state;
  const line = gone ?? away ?? notice;

  return (
    <section
      aria-label={`${terminal.program} on ${hostName}`}
      className="flex h-full min-h-0 flex-col"
    >
      <div className="flex items-center gap-3 border-b border-line px-4 py-1.5">
        <button
          type="button"
          onClick={onBack}
          className="rounded border border-line px-2 py-0.5 text-ink-muted hover:border-accent hover:text-ink"
        >
          Back
        </button>
        <span className="font-mono text-ink">{terminal.program}</span>
        <span className="text-ink-faint">{terminal.purpose}</span>
        <span role="status" aria-label="Control" className="text-ink-muted">
          {controllerText(state)}
          {exit && ` Its program ended (${exit.code}).`}
        </span>
        <span className="flex-1" />
        {held ? (
          <button
            type="button"
            onClick={() => source.release()}
            className="rounded border border-line px-2 py-0.5 text-ink-muted hover:border-accent hover:text-ink"
          >
            Release control
          </button>
        ) : (
          mayType &&
          !gone &&
          !exit && (
            <button
              type="button"
              disabled={asking || away !== null}
              onClick={() => {
                setState((s) => ({ ...s, asking: true, notice: null }));
                source.takeControl();
              }}
              className="rounded border border-accent px-2 py-0.5 text-ink hover:bg-accent/20 disabled:opacity-50"
            >
              {asking ? "Taking control…" : "Take control"}
            </button>
          )
        )}
      </div>
      {line && (
        <p
          role="alert"
          className={`border-b border-line px-4 py-1 select-text ${gone || away ? "text-amber-300" : "text-ink-muted"}`}
        >
          {line}
        </p>
      )}
      {!mayType && (
        <p className="border-b border-line px-4 py-1 text-ink-faint">
          That computer lets this one watch, not type.
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        <TerminalSurface source={source} fits={held} typing={held} />
      </div>
    </section>
  );
}
