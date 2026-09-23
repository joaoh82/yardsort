import { render } from "@testing-library/react";
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
    dragHandler: null as DragHandler | null,
    unlistenDrag: vi.fn(),
  };
  class Terminal {
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
    onResize() {
      return { dispose() {} };
    }
    write() {}
    hasSelection = () => false;
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
      return Promise.resolve(fake.state.unlistenDrag);
    },
  }),
}));
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
import { TerminalView } from "./TerminalView";

const tab = (recordId: string | null): TerminalTab => ({
  id: "s1",
  workspaceId: "w1",
  title: "claude",
  exit: null,
  recordId,
  busy: false,
  attention: false,
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
  fake.state.csi.clear();
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

  it("stops listening when unmounted", async () => {
    const { unmount } = render(<TerminalView sessionId="s1" />);
    await vi.waitFor(() => expect(fake.state.dragHandler).not.toBeNull());
    unmount();
    expect(fake.state.unlistenDrag).toHaveBeenCalled();
  });
});
