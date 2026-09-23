/**
 * Keystrokes and drops that the terminal turns into bytes for the program, kept out of the
 * component so they can be tested without xterm.
 */

/**
 * Shift+Enter in the kitty keyboard protocol's encoding. It is what kitty, Ghostty and foot send
 * for the key even to programs that never asked for the protocol, and every agent Yardsort runs
 * reads it as "new line" rather than "send" — which is why the key works in those terminals and,
 * without this, not here: xterm.js sends a plain carriage return for it, same as Enter.
 */
export const SHIFT_ENTER = "\x1b[13;2u";

/**
 * What to write for a keydown of Shift+Enter, or `null` for anything else — including when the
 * program should get the plain Enter xterm sends by default. A shell does not know the encoding
 * (bash prints `;2u`), so it goes only to programs that have identified themselves as speaking
 * it, or that were started as an agent.
 */
export function shiftEnterInput(event: KeyboardEvent, speaksCsiU: boolean): string | null {
  if (event.type !== "keydown" || event.key !== "Enter" || !event.shiftKey) return null;
  if (event.ctrlKey || event.altKey || event.metaKey) return null;
  return speaksCsiU ? SHIFT_ENTER : null;
}

/**
 * The text a terminal pastes when files are dropped on it: each path quoted for the shell of the
 * platform, separated by spaces, with a trailing space so typing can carry on. Agents read the
 * paths as files to look at; a shell gets arguments it can use as they are.
 */
export function droppedPathsText(paths: string[], windows: boolean): string {
  return paths.map((path) => (windows ? quoteWindows(path) : quotePosix(path))).join(" ") + " ";
}

// Backslash-escape what a POSIX shell would otherwise interpret, the way kitty and Ghostty do.
function quotePosix(path: string): string {
  return path.replace(/[\s!"#$&'()*;<>?[\\\]^`{|}~]/g, "\\$&");
}

// cmd.exe and PowerShell both take a double-quoted path, and a Windows file name cannot contain
// a double quote. Backslashes are path separators there, not escapes.
function quoteWindows(path: string): string {
  return /[\s&()^;,=|<>%']/.test(path) ? `"${path}"` : path;
}
