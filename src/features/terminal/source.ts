import { ipc, type SessionId, type TermSize } from "@/lib/ipc";
import { useTerminalStore } from "@/stores/terminals";

/** Where a terminal view sends what it is told to draw. */
export interface TerminalSink {
  /** The terminal's own bytes, for xterm unchanged. */
  output(bytes: Uint8Array): void;
  /** Start over at this size: clear the view, as a fresh snapshot follows. */
  reset(size: TermSize): void;
  /** The terminal is this size now. Followed only while the view does not fit it. */
  size(size: TermSize): void;
}

/**
 * What a terminal view draws and types into: a PTY on this computer, or a terminal on another one
 * through the relay. The view is the same for both: the same xterm, renderer and keys.
 */
export interface TerminalSource {
  /** The terminal, to tell one from another: a new key is a new view. */
  readonly key: string;
  /** Start the stream, the view being `size`. Resolves to what stops it. */
  attach(size: TermSize, sink: TerminalSink): Promise<() => void>;
  write(data: string): Promise<void>;
  /** The view fits this many cells. Asked only while the view fits the terminal. */
  resize(size: TermSize): Promise<void>;
  /** Whether an agent may be running in it, which reads Shift+Enter whether or not it says so. */
  isAgent(): boolean;
}

/**
 * A PTY of this computer's, whose size is always the view's. `agent` says it runs one though no tab
 * says so, as the Floor Manager's does.
 */
export function localSource(sessionId: SessionId, agent = false): TerminalSource {
  return {
    key: sessionId,
    async attach(size, sink) {
      // Size the PTY first so the snapshot is rendered for exactly this many cells.
      await ipc.ptyResize(sessionId, size);
      const id = await ipc.ptyAttach(sessionId, sink.output);
      return () => void ipc.ptyDetach(sessionId, id).catch(() => {});
    },
    write: (data) => ipc.ptyWrite(sessionId, data),
    resize(size) {
      useTerminalStore.getState().setLastSize(size);
      return ipc.ptyResize(sessionId, size);
    },
    isAgent: () =>
      agent ||
      useTerminalStore.getState().tabs.some((t) => t.id === sessionId && t.recordId !== null),
  };
}
