import { useMemo, useState } from "react";
import { errorMessage, ipc, type OtherTerminal, type RemoteTerminalSeen } from "@/lib/ipc";
import { TerminalSurface } from "@/features/terminal/TerminalView";
import type { TerminalSource } from "@/features/terminal/source";
import { controllerText, type RemoteState, seenBy, WATCHING } from "./remoteTerminalState";

/** Another computer's terminal through the relay, as it draws there. */
class RemoteSource implements TerminalSource {
  readonly key: string;
  #id: number | null = null;
  /** The number it was opened as, once it is: what is asked before then waits for it. */
  #opened = opening();

  constructor(
    readonly hostId: string,
    readonly terminal: OtherTerminal,
    readonly onSeen: (seen: RemoteTerminalSeen) => void,
  ) {
    this.key = `${hostId}/${terminal.id}`;
  }

  async attach(_size: unknown, sink: Parameters<TerminalSource["attach"]>[1]) {
    const opened = this.#opened;
    let id: number;
    try {
      id = await ipc.remoteTerminalOpen(this.hostId, this.terminal.id, sink.output, (seen) => {
        if (seen.kind === "reset") sink.reset(seen.size);
        else if (seen.kind === "resized") sink.size(seen.size);
        this.onSeen(seen);
      });
    } catch (error) {
      opened.fail(error);
      throw error;
    }
    this.#id = id;
    opened.open(id);
    return () => {
      this.#id = null;
      // Opened again, as when the renderer changes: what is asked meanwhile waits for that.
      this.#opened = opening();
      void ipc.remoteTerminalClose(id).catch(() => {});
    };
  }

  /** Its number, once it is open: a refusal when it could not be. */
  #open() {
    return this.#opened.promise;
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

  /** Ask for control. The answer comes as what the terminal shows; a rejection if not asked. */
  async takeControl() {
    await ipc.remoteTerminalTakeControl(await this.#open());
  }

  async release() {
    await ipc.remoteTerminalRelease(await this.#open());
  }
}

function opening() {
  let open!: (id: number) => void;
  let fail!: (error: unknown) => void;
  const promise = new Promise<number>((resolve, reject) => {
    open = resolve;
    fail = reject;
  });
  // Nobody may be waiting for it when it fails: the view says so itself.
  promise.catch(() => {});
  return { promise, open, fail };
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
  const { controller, held, asking, notice, away, gone, exit } = state;
  const line = gone ?? away ?? notice;
  // Not asked, after all: the button is usable again, and the person is told why.
  const refused = (error: unknown) =>
    setState((s) => ({ ...s, asking: false, notice: errorMessage(error) }));

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
            onClick={() => void source.release().catch(refused)}
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
              // Until it says who has control, it has not been reached yet.
              disabled={asking || away !== null || controller === null}
              onClick={() => {
                setState((s) => ({ ...s, asking: true, notice: null }));
                source.takeControl().catch(refused);
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
