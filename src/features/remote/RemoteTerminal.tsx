import { type ReactNode, useMemo, useState } from "react";
import {
  errorMessage,
  ipc,
  type OtherLink,
  type OtherTerminal,
  type RemoteTerminalSeen,
} from "@/lib/ipc";
import { ComputerIcon } from "@/features/sidebar/OtherComputersTree";
import { linkText } from "@/features/sidebar/otherComputersText";
import { thereKey, useOtherComputersStore } from "@/stores/otherComputers";
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
 * only once this computer has control. The person at that computer sees who has it. Taking it is
 * a press here, never a keystroke, except for a terminal this computer started: starting it was
 * already this person's act, so its control is asked for once, as soon as it can be.
 *
 * The bar above it names the computer and says who has control; `between` is what the workspace
 * puts under that bar: its tabs.
 */
export function RemoteTerminal(props: {
  hostId: string;
  hostName: string;
  link: OtherLink;
  terminal: OtherTerminal;
  mayType: boolean;
  between?: ReactNode;
}) {
  const { hostId, hostName, link, terminal, mayType, between } = props;
  const [state, setState] = useState<RemoteState>(WATCHING);
  // Not asked, after all: the button is usable again, and the person is told why.
  const refused = (error: unknown) =>
    setState((s) => ({ ...s, asking: false, notice: errorMessage(error) }));
  const source = useMemo(
    () => {
      const made: RemoteSource = new RemoteSource(hostId, terminal, (seen) => {
        setState((s) => seenBy(s, seen));
        // A terminal started from this computer: its control is asked for as soon as it says who
        // has it, which is when it is reached. Once: refused or taken back, it is a press again.
        const others = useOtherComputersStore.getState();
        if (
          seen.kind === "controller" &&
          others.taking.includes(thereKey(hostId, terminal.id)) &&
          others.views[hostId]?.mayType
        ) {
          others.asked(hostId, terminal.id);
          setState((s) => ({ ...s, asking: true, notice: null }));
          made.takeControl().catch(refused);
        }
      });
      return made;
    },
    // The same terminal is the same source, whatever else its listing says now.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [hostId, terminal.id],
  );
  const { controller, held, asking, notice, away, gone, exit } = state;
  const line = gone ?? away ?? notice;
  const take = () => {
    setState((s) => ({ ...s, asking: true, notice: null }));
    source.takeControl().catch(refused);
  };
  // Until it says who has control, it has not been reached yet.
  const reached = controller !== null && away === null && !gone && !exit;

  return (
    <section
      aria-label={`${terminal.program} on ${hostName}`}
      className="flex h-full min-h-0 flex-col"
    >
      <ComputerBar name={hostName} link={link}>
        <span role="status" aria-label="Control" className="text-small text-ink-muted">
          {mayType ? controllerText(state) : `${hostName} lets this computer watch, not type.`}
          {exit && ` Its program ended (${exit.code}).`}
        </span>
        {held ? (
          <button
            type="button"
            onClick={() => void source.release().catch(refused)}
            className="rounded border border-line px-2 py-0.5 text-small text-ink-muted hover:border-accent hover:text-ink"
          >
            Release
          </button>
        ) : (
          mayType &&
          !gone &&
          !exit && (
            <button
              type="button"
              disabled={asking || !reached}
              onClick={take}
              className="rounded border border-accent px-2 py-0.5 text-small text-ink hover:bg-accent/20 disabled:opacity-50"
            >
              {asking ? "Taking control…" : "Take control"}
            </button>
          )
        )}
      </ComputerBar>
      {between}
      {line && (
        <p
          role="alert"
          className={`border-b border-line px-3 py-1 select-text ${gone || away ? "text-amber-300" : "text-ink-muted"}`}
        >
          {line}
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        <TerminalSurface source={source} fits={held} typing={held} />
      </div>
    </section>
  );
}

/**
 * The bar across the top of a workspace on another computer: which computer, and how it stands.
 * It is where a local workspace has Run; what stands at its end is who has control.
 */
export function ComputerBar({
  name,
  link,
  children,
}: {
  name: string;
  link: OtherLink;
  children?: ReactNode;
}) {
  const online = link.kind === "online";
  return (
    <div
      role="group"
      aria-label={`${name}, another computer`}
      className="flex min-h-8 shrink-0 items-center gap-2 border-b border-line px-3 py-1 text-ink-muted"
    >
      <ComputerIcon />
      <span className="font-medium text-ink">{name}</span>
      {online && <span aria-hidden className="size-2 rounded-full bg-green-400" />}
      <span className={`text-small ${online ? "text-ink-faint" : "text-amber-300"}`}>
        {linkText(link)}
      </span>
      <div className="ml-auto flex min-w-0 items-center gap-3">{children}</div>
    </div>
  );
}
