/**
 * Whether a Tauri file-drag position falls on `element`: inside its box, and with nothing else
 * on top of it there.
 *
 * The position is typed as physical pixels but is not on every platform: wry passes GTK widget
 * coordinates on Linux and NSView points on macOS — logical, both — and `ScreenToClient` pixels
 * on Windows, and Tauri wraps all three as they are (tauri-runtime-wry 2.11, wry 0.55).
 *
 * Every drop target hears every native drop, since the webview reports them for the whole
 * window. The box alone is not enough once one target can sit over another (the Floor Manager's
 * panel over a workspace's terminal): both boxes hold the point, and both would take the drop.
 * The element actually at the point decides.
 */
export function isDragOver(
  element: HTMLElement,
  position: { x: number; y: number },
  windows: boolean,
): boolean {
  const scale = windows ? window.devicePixelRatio : 1;
  const x = position.x / scale;
  const y = position.y / scale;
  const rect = element.getBoundingClientRect();
  if (!(x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)) return false;
  // jsdom has no elementFromPoint; a real webview always does.
  if (typeof document.elementFromPoint !== "function") return true;
  const top = document.elementFromPoint(x, y);
  return top !== null && element.contains(top);
}
