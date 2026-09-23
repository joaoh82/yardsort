/**
 * Whether a Tauri file-drag position falls inside `element`.
 *
 * The position is typed as physical pixels but is not on every platform: wry passes GTK widget
 * coordinates on Linux and NSView points on macOS — logical, both — and `ScreenToClient` pixels
 * on Windows, and Tauri wraps all three as they are (tauri-runtime-wry 2.11, wry 0.55).
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
  return x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
}
