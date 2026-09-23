import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef, type RefObject } from "react";
import { isDragOver } from "./dragPosition";
import { isWindows } from "./platform";

/**
 * Report native file drags that pass over `target`. The webview swallows them, so they arrive as
 * Tauri events rather than DOM drag events, and every listener hears the whole window — `target`
 * is what decides the event is for this view. While one is over it, `data-drop="over"` is set.
 */
export function useFileDrop(
  target: RefObject<HTMLElement | null>,
  onDrop: (paths: string[]) => void,
  enabled = true,
) {
  const onDropRef = useRef(onDrop);
  useEffect(() => {
    onDropRef.current = onDrop;
  }, [onDrop]);

  useEffect(() => {
    const element = target.current;
    if (!element || !enabled) return;

    let disposed = false;
    let unlisten: (() => void) | null = null;
    const setHover = (over: boolean) => {
      if (over) element.dataset.drop = "over";
      else delete element.dataset.drop;
    };

    try {
      void getCurrentWebview()
        .onDragDropEvent(({ payload }) => {
          if (disposed || payload.type === "leave") return setHover(false);
          const over = isDragOver(element, payload.position, isWindows);
          if (payload.type !== "drop") return setHover(over);
          setHover(false);
          if (!over || payload.paths.length === 0) return;
          onDropRef.current(payload.paths);
        })
        .then((stop) => {
          if (disposed) stop();
          else unlisten = stop;
        }, console.error);
    } catch {
      // The webview API is absent outside the app (a test that renders the composer without
      // mocking drag). Drops do nothing there; that must not take the composer down with it.
    }

    return () => {
      disposed = true;
      unlisten?.();
      delete element.dataset.drop;
    };
  }, [target, enabled]);
}
