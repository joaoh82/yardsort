import { getCurrentWebview } from "@tauri-apps/api/webview";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { ipc, type SessionId } from "@/lib/ipc";
import { isMac, isModKey, isWindows, shortcutKey } from "@/lib/platform";
import { useTerminalStore } from "@/stores/terminals";
import { droppedPathsText, shiftEnterInput } from "./input";
import { attachRenderer, rendererPreference, type RendererKind } from "./renderer";
import { darkTheme, FONT_FAMILY, lightTheme } from "./theme";
import { createInputWriter } from "./writer";

export interface TerminalProbe {
  /** Called with every chunk of output, after it was handed to xterm. */
  onOutput?: (bytes: number) => void;
  /** Called once for every frame xterm paints — the moment output reaches the screen. */
  onPaint?: () => void;
  onRenderer?: (kind: RendererKind) => void;
}

interface Props {
  sessionId: SessionId;
  rendererOverride?: string | null;
  probe?: TerminalProbe;
}

/**
 * A view onto one PTY session. The session lives in the core; this component can be unmounted
 * and mounted again at will — on mount it receives a snapshot that repaints scrollback and
 * screen, then the live stream.
 */
export function TerminalView({ sessionId, rendererOverride, probe }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const probeRef = useRef(probe);
  useEffect(() => {
    probeRef.current = probe;
  }, [probe]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const prefersLight = window.matchMedia("(prefers-color-scheme: light)");
    const term = new Terminal({
      allowProposedApi: true, // required by the unicode11 addon
      cursorBlink: true,
      fontFamily: FONT_FAMILY,
      fontSize: 13,
      lineHeight: 1.2,
      macOptionIsMeta: true,
      scrollback: 10_000,
      theme: prefersLight.matches ? lightTheme : darkTheme,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(
      new WebLinksAddon((event, uri) => {
        // Plain clicks belong to the program (mouse reporting); links need the modifier.
        if (isMac ? event.metaKey : event.ctrlKey) void openUrl(uri).catch(console.error);
      }),
    );
    term.open(container);
    attachRenderer(term, rendererPreference(rendererOverride), (kind) => {
      useTerminalStore.getState().setRenderer(kind);
      probeRef.current?.onRenderer?.(kind);
    });

    // Only a benchmark listens; the call costs an optional chain per painted frame.
    const onRender = term.onRender(() => probeRef.current?.onPaint?.());

    const onThemeChange = () => {
      term.options.theme = prefersLight.matches ? lightTheme : darkTheme;
    };
    prefersLight.addEventListener("change", onThemeChange);

    let disposed = false;

    // --- input -------------------------------------------------------------------------------
    const write = createInputWriter((data) => ipc.ptyWrite(sessionId, data));
    const onData = term.onData(write);

    // Shift+Enter has no encoding older than the kitty keyboard protocol. A program that queries
    // (`CSI ? u`), pushes (`CSI > flags u`) or sets (`CSI = flags u`) the protocol reads it; so
    // does every agent, whether or not it announces itself — Grok does not. Agent tabs are known
    // by their conversation record; a shell has none until something in it speaks up.
    let speaksCsiU = false;
    const csiU = ["?", ">", "="].map((prefix) =>
      term.parser.registerCsiHandler({ prefix, final: "u" }, () => {
        speaksCsiU = true;
        return false;
      }),
    );
    const isAgent = () =>
      useTerminalStore.getState().tabs.some((t) => t.id === sessionId && t.recordId !== null);

    term.attachCustomKeyEventHandler((event) => {
      const newline = shiftEnterInput(event, speaksCsiU || isAgent());
      if (newline !== null) {
        write(newline);
        event.preventDefault();
        return false;
      }
      if (event.type !== "keydown" || !isModKey(event)) return true;
      const key = shortcutKey(event);
      if (key === "c" && term.hasSelection()) {
        void writeText(term.getSelection()).catch(console.error);
      } else if (key === "v") {
        // `paste` applies bracketed-paste wrapping when the program asked for it.
        void readText().then((text) => text && term.paste(text), console.error);
      } else {
        // Every other Mod combination is an app shortcut: keep it away from the PTY and let it
        // bubble to the window's handlers.
        return false;
      }
      event.preventDefault();
      return false;
    });

    // --- drops -------------------------------------------------------------------------------
    // The webview swallows native file drops and reports them as Tauri events instead of DOM
    // ones. A drop over this view pastes the paths, as a terminal emulator would, so an agent
    // gets the file to read and a shell gets an argument. The position is typed as physical
    // pixels but is not on every platform: wry passes GTK widget coordinates on Linux and NSView
    // points on macOS — logical, both — and `ScreenToClient` pixels on Windows, and Tauri wraps
    // all three as they are (tauri-runtime-wry 2.11, wry 0.55).
    const isOver = (position: { x: number; y: number }) => {
      const scale = isWindows ? window.devicePixelRatio : 1;
      const x = position.x / scale;
      const y = position.y / scale;
      const rect = container.getBoundingClientRect();
      return x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
    };
    const setHover = (over: boolean) => {
      if (over) container.dataset.drop = "over";
      else delete container.dataset.drop;
    };
    let unlistenDrop: (() => void) | null = null;
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (disposed || payload.type === "leave") return setHover(false);
        const over = isOver(payload.position);
        if (payload.type !== "drop") return setHover(over);
        setHover(false);
        if (!over || payload.paths.length === 0) return;
        term.paste(droppedPathsText(payload.paths, isWindows));
        term.focus();
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenDrop = unlisten;
      }, console.error);

    // --- size --------------------------------------------------------------------------------
    const currentSize = () => ({ cols: term.cols, rows: term.rows });
    const onResize = term.onResize((size) => {
      useTerminalStore.getState().setLastSize(size);
      void ipc.ptyResize(sessionId, size).catch(console.error);
    });
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        // A hidden or collapsed container has no size; fitting to it would shrink the PTY to
        // nothing and make the program reflow for no reason.
        if (container.clientWidth > 0 && container.clientHeight > 0) fit.fit();
      });
    });

    // --- attach ------------------------------------------------------------------------------
    let attachment: number | null = null;
    void (async () => {
      fit.fit();
      // Size the PTY first so the snapshot is rendered for exactly this many cells.
      await ipc.ptyResize(sessionId, currentSize());
      const id = await ipc.ptyAttach(sessionId, (bytes) => {
        if (disposed) return;
        term.write(bytes, () => probeRef.current?.onOutput?.(bytes.byteLength));
      });
      if (disposed) return void ipc.ptyDetach(sessionId, id).catch(() => {});
      attachment = id;
      observer.observe(container);
      term.focus();
    })().catch((error) => {
      if (!disposed) term.write(`\r\n\x1b[31m${String(error?.message ?? error)}\x1b[m\r\n`);
    });

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      prefersLight.removeEventListener("change", onThemeChange);
      unlistenDrop?.();
      for (const handler of csiU) handler.dispose();
      onData.dispose();
      onResize.dispose();
      onRender.dispose();
      if (attachment !== null) void ipc.ptyDetach(sessionId, attachment).catch(() => {});
      term.dispose();
    };
  }, [sessionId, rendererOverride]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full overflow-hidden bg-canvas p-2 data-[drop=over]:outline-2 data-[drop=over]:-outline-offset-2 data-[drop=over]:outline-accent"
    />
  );
}
