import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

type KeyHandler = (event: KeyboardEvent) => boolean;
type DragHandler = (event: { payload: unknown }) => void;

// A stand-in for xterm: records the handlers the view attaches so the test can drive them.
// xterm itself needs a real layout engine, which jsdom is not.
const fake = vi.hoisted(() => {
  const state = {
    keyHandler: null as KeyHandler | null,
    csi: new Map<string, () => boolean>(),
    paste: vi.fn(),
    focus: vi.fn(),
    selection: "",
    clearSelection: vi.fn(),
    selectAll: vi.fn(),
    mouseTrackingMode: "none",
    dragHandler: null as DragHandler | null,
    // The webview tells every listener about every drop, as the real one does.
    dragHandlers: [] as DragHandler[],
    terminals: [] as unknown[],
    unlistenDrag: vi.fn(),
    resizeHandler: null as ((size: { cols: number; rows: number }) => void) | null,
    resized: vi.fn(),
    reset: vi.fn(),
    /** What the view did to the terminal, in order: each write once it is parsed. */
    log: [] as string[],
    /** Writes not parsed yet, while `parseLater` holds them back as the real xterm can. */
    unparsed: [] as (() => void)[],
    parseLater: false,
  };
  class Terminal {
    constructor() {
      state.terminals.push(this);
    }
    cols = 80;
    rows = 24;
    options = {};
    unicode = { activeVersion: "" };
    parser = {
      registerCsiHandler: (id: { prefix?: string; final: string }, handler: () => boolean) => {
        state.csi.set(`${id.prefix ?? ""}${id.final}`, handler);
        return { dispose() {} };
      },
    };
    open() {}
    loadAddon() {}
    attachCustomKeyEventHandler(handler: KeyHandler) {
      state.keyHandler = handler;
    }
    onData() {
      return { dispose() {} };
    }
    onRender() {
      return { dispose() {} };
    }
    onResize(handler: (size: { cols: number; rows: number }) => void) {
      state.resizeHandler = handler;
      return { dispose() {} };
    }
    resize(cols: number, rows: number) {
      this.cols = cols;
      this.rows = rows;
      state.resized(cols, rows);
      state.log.push(`resize ${cols}x${rows}`);
      state.resizeHandler?.({ cols, rows });
    }
    reset = () => {
      state.log.push("reset");
      state.reset();
    };
    write(data: string | Uint8Array, parsed?: () => void) {
      const done = () => {
        state.log.push(`write ${typeof data === "string" ? data : new TextDecoder().decode(data)}`);
        parsed?.();
      };
      if (state.parseLater) state.unparsed.push(done);
      else done();
    }
    hasSelection = () => state.selection !== "";
    getSelection = () => state.selection;
    clearSelection = state.clearSelection;
    selectAll = state.selectAll;
    get modes() {
      return { mouseTrackingMode: state.mouseTrackingMode };
    }
    paste = state.paste;
    focus = state.focus;
    dispose() {}
  }
  return { state, Terminal };
});

vi.mock("@xterm/xterm", () => ({ Terminal: fake.Terminal }));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class {} }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("./renderer", () => ({ attachRenderer: () => {}, rendererPreference: () => "dom" }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: DragHandler) => {
      fake.state.dragHandler = handler;
      fake.state.dragHandlers.push(handler);
      return Promise.resolve(fake.state.unlistenDrag);
    },
  }),
}));
const clipboard = vi.hoisted(() => ({ readText: vi.fn(), writeText: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);
const core = vi.hoisted(() => ({
  ptyWrite: vi.fn(),
  ptyResize: vi.fn(),
  ptyAttach: vi.fn(),
  ptyDetach: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import type { TerminalSink, TerminalSource } from "./source";
import { TerminalSurface, TerminalView } from "./TerminalView";

const tab = (recordId: string | null): TerminalTab => ({
  id: "s1",
  workspaceId: "w1",
  title: "claude",
  exit: null,
  recordId,
  busy: false,
  attention: false,
  toldAbout: null,
  toldAt: null,
  busySince: null,
});

const shiftEnter = () => {
  const event = new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, cancelable: true });
  return { event, handled: !fake.state.keyHandler!(event) };
};

const mount = async () => {
  render(<TerminalView sessionId="s1" />);
  // The drag listener registers asynchronously.
  await vi.waitFor(() => expect(fake.state.dragHandler).not.toBeNull());
};

beforeEach(() => {
  vi.clearAllMocks();
  // jsdom has no matchMedia; the view reads it for the colour scheme.
  window.matchMedia = () =>
    ({
      matches: false,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
  fake.state.keyHandler = null;
  fake.state.dragHandler = null;
  fake.state.dragHandlers = [];
  fake.state.terminals = [];
  fake.state.csi.clear();
  fake.state.selection = "";
  fake.state.mouseTrackingMode = "none";
  clipboard.readText.mockResolvedValue("pasted text");
  clipboard.writeText.mockResolvedValue(undefined);
  core.ptyWrite.mockResolvedValue(undefined);
  core.ptyResize.mockResolvedValue(undefined);
  core.ptyAttach.mockResolvedValue(1);
  core.ptyDetach.mockResolvedValue(undefined);
});

describe("Shift+Enter", () => {
  it("sends the kitty encoding to an agent, so it starts a new line instead of sending", async () => {
    useTerminalStore.setState({ tabs: [tab("record-1")] });
    await mount();
    const { event, handled } = shiftEnter();
    expect(handled).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(core.ptyWrite).toHaveBeenCalledWith("s1", "\x1b[13;2u");
  });

  it("gives a shell a plain Enter until a program in it speaks the protocol", async () => {
    useTerminalStore.setState({ tabs: [tab(null)] });
    await mount();
    expect(shiftEnter().handled).toBe(false);
    expect(core.ptyWrite).not.toHaveBeenCalled();

    // `claude` started from the shell queries the kitty keyboard protocol (`CSI ? u`).
    fake.state.csi.get("?u")!();
    expect(shiftEnter().handled).toBe(true);
    expect(core.ptyWrite).toHaveBeenCalledWith("s1", "\x1b[13;2u");
  });
});

describe("dropping files", () => {
  const rect = { left: 0, top: 0, right: 800, bottom: 600 } as DOMRect;
  const at = (x: number, y: number) => ({ x, y });

  beforeEach(() => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect);
    useTerminalStore.setState({ tabs: [tab("record-1")] });
  });

  it("pastes the dropped paths, quoted for the shell, and marks the view while hovering", async () => {
    const { container } = render(<TerminalView sessionId="s1" />);
    await vi.waitFor(() => expect(fake.state.dragHandler).not.toBeNull());
    const view = container.firstElementChild as HTMLElement;

    fake.state.dragHandler!({ payload: { type: "over", position: at(100, 100) } });
    expect(view.dataset.drop).toBe("over");
    fake.state.dragHandler!({
      payload: { type: "drop", paths: ["/home/me/My Docs/plan.md"], position: at(100, 100) },
    });
    expect(view.dataset.drop).toBeUndefined();
    expect(fake.state.paste).toHaveBeenCalledWith("/home/me/My\\ Docs/plan.md ");
    expect(fake.state.focus).toHaveBeenCalled();
  });

  it("ignores a drop somewhere else in the window", async () => {
    await mount();
    fake.state.dragHandler!({
      payload: { type: "drop", paths: ["/home/me/plan.md"], position: at(900, 100) },
    });
    expect(fake.state.paste).not.toHaveBeenCalled();
  });

  it("goes only to the terminal on top where two overlap, as the Floor Manager's does", async () => {
    // The workspace's terminal fills the centre; the Floor Manager's floats over its corner.
    const { container } = render(
      <>
        <TerminalView sessionId="s1" />
        <TerminalView sessionId="coordinator" agent />
      </>,
    );
    await vi.waitFor(() => expect(fake.state.dragHandlers).toHaveLength(2));
    const behind = container.children[0] as HTMLElement;
    const onTop = container.children[1] as HTMLElement;
    document.elementFromPoint = vi.fn(() => onTop.firstElementChild ?? onTop);
    try {
      const drop = { type: "drop", paths: ["/home/me/plan.md"], position: at(700, 500) };
      for (const handler of fake.state.dragHandlers) handler({ payload: drop });
      expect(fake.state.paste).toHaveBeenCalledTimes(1);
      expect(fake.state.paste.mock.contexts[0]).toBe(fake.state.terminals[1]);

      // Over the panel's own header or welcome text, neither terminal is on top: no paste.
      fake.state.paste.mockClear();
      document.elementFromPoint = vi.fn(() => document.body);
      for (const handler of fake.state.dragHandlers) handler({ payload: drop });
      expect(fake.state.paste).not.toHaveBeenCalled();
      expect(behind.dataset.drop).toBeUndefined();
    } finally {
      delete (document as { elementFromPoint?: unknown }).elementFromPoint;
    }
  });

  it("stops listening when unmounted", async () => {
    const { unmount } = render(<TerminalView sessionId="s1" />);
    await vi.waitFor(() => expect(fake.state.dragHandler).not.toBeNull());
    unmount();
    expect(fake.state.unlistenDrag).toHaveBeenCalled();
  });
});

describe("clipboard", () => {
  const press = (key: string, init: KeyboardEventInit = {}) => {
    const event = new KeyboardEvent("keydown", { key, cancelable: true, ...init });
    return { event, handled: !fake.state.keyHandler!(event) };
  };

  beforeEach(() => useTerminalStore.setState({ tabs: [tab(null)] }));

  it("copies a selection with plain Ctrl+C and clears it, so the next one interrupts", async () => {
    await mount();
    fake.state.selection = "ls -la";
    const { event, handled } = press("c", { ctrlKey: true });
    expect(handled).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(clipboard.writeText).toHaveBeenCalledWith("ls -la");
    expect(fake.state.clearSelection).toHaveBeenCalled();
  });

  it("lets Ctrl+C through to the program when nothing is selected", async () => {
    await mount();
    expect(press("c", { ctrlKey: true }).handled).toBe(false);
    expect(clipboard.writeText).not.toHaveBeenCalled();
  });

  it("pastes with Shift+Insert", async () => {
    await mount();
    expect(press("Insert", { shiftKey: true }).handled).toBe(true);
    await vi.waitFor(() => expect(fake.state.paste).toHaveBeenCalledWith("pasted text"));
  });

  it("offers Copy and Paste on right-click", async () => {
    const { container } = render(<TerminalView sessionId="s1" />);
    await vi.waitFor(() => expect(fake.state.dragHandler).not.toBeNull());
    const view = container.firstElementChild as HTMLElement;

    fireEvent.contextMenu(view);
    expect(screen.getByRole("menuitem", { name: "Copy" })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "Paste" }));
    await vi.waitFor(() => expect(fake.state.paste).toHaveBeenCalledWith("pasted text"));
    expect(screen.queryByRole("menu")).toBeNull();

    fake.state.selection = "error: oops";
    fireEvent.contextMenu(view);
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy" }));
    expect(clipboard.writeText).toHaveBeenCalledWith("error: oops");
  });

  it("leaves right-click to a program that reads the mouse, unless Shift is held", async () => {
    const { container } = render(<TerminalView sessionId="s1" />);
    await vi.waitFor(() => expect(fake.state.dragHandler).not.toBeNull());
    const view = container.firstElementChild as HTMLElement;
    fake.state.mouseTrackingMode = "any";

    fireEvent.contextMenu(view);
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.contextMenu(view, { shiftKey: true });
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });
});

describe("a terminal whose size is not the view's", () => {
  // As another computer's terminal is, until this computer takes control of it.
  const following = () => {
    let sink: TerminalSink | null = null;
    const source: TerminalSource = {
      key: "there/t1",
      attach: vi.fn(async (_size, given: TerminalSink) => {
        sink = given;
        return () => {};
      }),
      write: vi.fn(async () => {}),
      resize: vi.fn(async () => {}),
      isAgent: () => false,
    };
    return { source, sink: () => sink! };
  };

  beforeEach(() => {
    fake.state.resizeHandler = null;
    fake.state.log = [];
    fake.state.unparsed = [];
    fake.state.parseLater = false;
  });

  it("resizes and starts over only once what came before is drawn, and draws what follows after", async () => {
    const { source, sink } = following();
    render(<TerminalSurface source={source} fits={false} typing={false} />);
    await vi.waitFor(() => expect(source.attach).toHaveBeenCalled());
    fake.state.parseLater = true;
    const bytes = (text: string) => new TextEncoder().encode(text);

    sink().output(bytes("1234567890X"));
    sink().size({ cols: 20, rows: 24 });
    sink().output(bytes("after"));
    sink().reset({ cols: 30, rows: 24 });
    sink().output(bytes("NEW"));
    // Nothing is parsed yet, so nothing has been resized under it.
    expect(fake.state.log).toEqual([]);

    // xterm parses each write when it gets to it; whatever came next is applied in its turn.
    while (fake.state.unparsed.length > 0) fake.state.unparsed.shift()!();
    expect(fake.state.log).toEqual([
      "write 1234567890X",
      "resize 20x24",
      "write after",
      "reset",
      "resize 30x24",
      "write NEW",
    ]);
  });

  it("follows the size it is told, sends nothing typed, and fits once let", async () => {
    const { source, sink } = following();
    const { rerender } = render(<TerminalSurface source={source} fits={false} typing={false} />);
    await vi.waitFor(() => expect(source.attach).toHaveBeenCalled());

    sink().reset({ cols: 120, rows: 40 });
    expect(fake.state.reset).toHaveBeenCalled();
    expect(fake.state.resized).toHaveBeenLastCalledWith(120, 40);
    sink().size({ cols: 100, rows: 30 });
    expect(fake.state.resized).toHaveBeenLastCalledWith(100, 30);
    // Following is not fitting: nothing is sent back.
    expect(source.resize).not.toHaveBeenCalled();
    // The keys are the window's while it only watches.
    const event = new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, cancelable: true });
    expect(fake.state.keyHandler!(event)).toBe(true);
    expect(source.write).not.toHaveBeenCalled();

    rerender(<TerminalSurface source={source} fits typing />);
    expect(source.resize).toHaveBeenCalledWith({ cols: 100, rows: 30 });
    // Its size now is the view's: what it says goes unfollowed.
    sink().size({ cols: 90, rows: 20 });
    expect(fake.state.resized).not.toHaveBeenLastCalledWith(90, 20);

    // Control gone again: back to the size it said last.
    rerender(<TerminalSurface source={source} fits={false} typing={false} />);
    expect(fake.state.resized).toHaveBeenLastCalledWith(90, 20);
  });
});
