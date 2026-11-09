import { getCurrentWebview } from "@tauri-apps/api/webview";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useMemo, useRef, useState } from "react";
import { isDragOver } from "@/lib/dragPosition";
import type { SessionId, TermSize } from "@/lib/ipc";
import { isMac, isModKey, isWindows, shortcutKey } from "@/lib/platform";
import { useTerminalStore } from "@/stores/terminals";
import { ContextMenu } from "@/features/sidebar/ContextMenu";
import { clipboardKey, droppedPathsText, shiftEnterInput } from "./input";
import { attachRenderer, rendererPreference, type RendererKind } from "./renderer";
import { localSource, type TerminalSource } from "./source";
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
  /** It runs an agent, though no tab says so: the Floor Manager's terminal. */
  agent?: boolean;
}

/**
 * A view onto one PTY session. The session lives in the core; this component can be unmounted
 * and mounted again at will — on mount it receives a snapshot that repaints scrollback and
 * screen, then the live stream.
 */
export function TerminalView({ sessionId, rendererOverride, probe, agent = false }: Props) {
  const source = useMemo(() => localSource(sessionId, agent), [sessionId, agent]);
  return <TerminalSurface source={source} rendererOverride={rendererOverride} probe={probe} />;
}

interface SurfaceProps {
  source: TerminalSource;
  rendererOverride?: string | null;
  probe?: TerminalProbe;
  /**
   * Whether the view sets the terminal's size by fitting it, or follows the size the source says.
   * A local PTY always fits; another computer's terminal fits only while this one has control.
   */
  fits?: boolean;
  /** Whether typing goes to the source. Without it, keys, pastes and drops go nowhere. */
  typing?: boolean;
}

/** The terminal itself, drawing whatever `source` sends and typing into it. */
export function TerminalSurface({
  source,
  rendererOverride,
  probe,
  fits = true,
  typing = true,
}: SurfaceProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; selection: string } | null>(null);
  const probeRef = useRef(probe);
  useEffect(() => {
    probeRef.current = probe;
  }, [probe]);
  const fitsRef = useRef(fits);
  /** Fit now, or follow the source's size again: set by the effect below once it is mounted. */
  const refitRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    fitsRef.current = fits;
    refitRef.current?.();
  }, [fits]);
  const typingRef = useRef(typing);
  useEffect(() => {
    typingRef.current = typing;
    if (termRef.current) termRef.current.options.disableStdin = !typing;
  }, [typing]);

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
      disableStdin: !typingRef.current,
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
    termRef.current = term;
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
    const write = createInputWriter((data) => source.write(data));
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
    const isAgent = () => source.isAgent();

    term.attachCustomKeyEventHandler((event) => {
      // Watching only: the keys are this window's, and nothing goes to the terminal.
      if (!typingRef.current) return true;
      const newline = shiftEnterInput(event, speaksCsiU || isAgent());
      if (newline !== null) {
        write(newline);
        event.preventDefault();
        return false;
      }
      const clipboard = clipboardKey(event, term.hasSelection(), isMac);
      if (clipboard === "copy") {
        copyText(term.getSelection());
        // The next Ctrl+C is an interrupt again.
        term.clearSelection();
        event.preventDefault();
        return false;
      }
      if (clipboard === "paste") {
        pasteInto(term);
        event.preventDefault();
        return false;
      }
      if (event.type !== "keydown" || !isModKey(event)) return true;
      const key = shortcutKey(event);
      if (key === "c" && term.hasSelection()) {
        copyText(term.getSelection());
      } else if (key === "v") {
        pasteInto(term);
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
    // gets the file to read and a shell gets an argument. Position units differ by platform;
    // `isDragOver` accounts for that.
    const isOver = (position: { x: number; y: number }) =>
      isDragOver(container, position, isWindows);
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
        if (!over || payload.paths.length === 0 || !typingRef.current) return;
        term.paste(droppedPathsText(payload.paths, isWindows));
        term.focus();
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenDrop = unlisten;
      }, console.error);

    // --- size --------------------------------------------------------------------------------
    const currentSize = () => ({ cols: term.cols, rows: term.rows });
    // A resize the view made by following the source is not sent back to it.
    const onResize = term.onResize((size) => {
      if (fitsRef.current) void source.resize(size).catch(console.error);
    });
    // A hidden or collapsed container has no size; fitting to it would shrink the PTY to
    // nothing and make the program reflow for no reason.
    const fitToView = () => {
      if (container.clientWidth > 0 && container.clientHeight > 0) fit.fit();
    };
    let followed: TermSize | null = null;
    const follow = (size: TermSize) => {
      followed = size;
      if (!fitsRef.current && (term.cols !== size.cols || term.rows !== size.rows)) {
        term.resize(size.cols, size.rows);
      }
    };
    refitRef.current = () => {
      if (!fitsRef.current) {
        if (followed) follow(followed);
        return;
      }
      fitToView();
      // Fitting changes nothing when the size already fits; the source is told all the same.
      void source.resize(currentSize()).catch(console.error);
    };
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (fitsRef.current) fitToView();
      });
    });

    // --- attach ------------------------------------------------------------------------------
    let stop: (() => void) | null = null;
    void (async () => {
      fit.fit();
      const stopping = await source.attach(currentSize(), {
        output(bytes) {
          if (disposed) return;
          term.write(bytes, () => probeRef.current?.onOutput?.(bytes.byteLength));
        },
        reset(size) {
          if (disposed) return;
          term.reset();
          follow(size);
        },
        size(size) {
          if (!disposed) follow(size);
        },
      });
      if (disposed) return stopping();
      stop = stopping;
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
      stop?.();
      refitRef.current = null;
      termRef.current = null;
      setMenu(null);
      term.dispose();
    };
    // A source is known by its key: a new object for the same terminal is the same view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source.key, rendererOverride]);

  // Right-click offers Copy and Paste, unless the program reads the mouse itself (an agent's
  // TUI, vim); Shift+right-click opens the menu anyway, as Shift+drag still selects.
  const onContextMenu = (event: React.MouseEvent) => {
    const term = termRef.current;
    if (!term) return;
    event.preventDefault();
    if (term.modes.mouseTrackingMode !== "none" && !event.shiftKey) return;
    setMenu({ x: event.clientX, y: event.clientY, selection: term.getSelection() });
  };
  const withTerm = (action: (term: Terminal) => void) => {
    const term = termRef.current;
    if (!term) return;
    action(term);
    term.focus();
  };

  return (
    <>
      <div
        ref={containerRef}
        onContextMenu={onContextMenu}
        className="h-full w-full overflow-hidden bg-canvas p-2 data-[drop=over]:outline-2 data-[drop=over]:-outline-offset-2 data-[drop=over]:outline-accent"
      />
      {menu && (
        <ContextMenu
          at={menu}
          onClose={() => setMenu(null)}
          items={[
            {
              label: "Copy",
              disabled: !menu.selection,
              onSelect: () => withTerm(() => copyText(menu.selection)),
            },
            { label: "Paste", onSelect: () => withTerm(pasteInto) },
            { label: "Select all", onSelect: () => withTerm((term) => term.selectAll()) },
          ]}
        />
      )}
    </>
  );
}

function copyText(text: string) {
  void writeText(text).catch(console.error);
}

// `paste` applies bracketed-paste wrapping when the program asked for it.
function pasteInto(term: Terminal) {
  void readText().then((text) => text && term.paste(text), console.error);
}
