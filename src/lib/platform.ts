/** True on macOS, where the primary modifier is ⌘ rather than Ctrl. */
export const isMac =
  typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);

/** True on Windows, where paths are quoted rather than escaped for the shell. */
export const isWindows = typeof navigator !== "undefined" && /^win/i.test(navigator.platform);

/**
 * Whether the app's primary modifier ("Mod") is held.
 *
 * Mod is ⌘ on macOS and **Ctrl+Shift** everywhere else — the convention terminal emulators use
 * (Ctrl+Shift+C to copy), because plain Ctrl+letter belongs to the program in the terminal:
 * Ctrl+B is "back one character" in a shell and the tmux prefix. App shortcuts must never take
 * those away.
 */
export function isModKey(event: KeyboardEvent | MouseEvent): boolean {
  return isMac ? event.metaKey && !event.ctrlKey : event.ctrlKey && event.shiftKey;
}

/** Human-readable shortcut, e.g. `formatShortcut("B")` → "⌘B" or "Ctrl+Shift+B". */
export function formatShortcut(key: string, { alt = false } = {}): string {
  if (isMac) return `${alt ? "⌥" : ""}⌘${key}`;
  return `Ctrl+Shift+${alt ? "Alt+" : ""}${key}`;
}

/**
 * The same shortcut split into the keys it is made of, for drawing one cap each:
 * `["⌘", "B"]` on macOS, `["Ctrl", "Shift", "B"]` elsewhere.
 */
export function shortcutKeys(key: string, { alt = false } = {}): string[] {
  if (isMac) return [...(alt ? ["⌥"] : []), "⌘", key];
  return ["Ctrl", "Shift", ...(alt ? ["Alt"] : []), key];
}

/**
 * The letter a shortcut event stands for, lower-cased ("b" for Ctrl+Shift+B). Uses the character
 * the layout produces rather than the physical key, so shortcuts follow the key caps on Dvorak,
 * AZERTY and friends.
 */
export function shortcutKey(event: KeyboardEvent): string {
  return event.key.length === 1 ? event.key.toLowerCase() : event.key;
}
