import { formatShortcut, isMac, isModKey, shortcutKey } from "./platform";

export const COMMANDS = [
  { id: "palette", label: "Commands and workspaces", key: "k" },
  { id: "shortcuts", label: "Keyboard shortcuts", key: "/" },
  { id: "settings", label: "Settings", key: "," },
  { id: "openProject", label: "Open project…", key: "o" },
  { id: "newWorkspace", label: "New workspace", key: "n" },
  { id: "newTerminal", label: "New shell tab", key: "t" },
  { id: "closeTerminal", label: "Close active terminal", key: "w" },
  { id: "toggleLeft", label: "Toggle projects panel", key: "b" },
  { id: "toggleRight", label: "Toggle changes panel", key: "Alt+b" },
  { id: "focusProjects", label: "Focus projects", key: "l" },
  { id: "focusWorkspace", label: "Focus workspace / terminal", key: "e" },
  { id: "focusChanges", label: "Focus changes and files", key: "r" },
  { id: "previousWorkspace", label: "Previous workspace", key: "ArrowUp" },
  { id: "nextWorkspace", label: "Next workspace", key: "ArrowDown" },
  { id: "previousTerminal", label: "Previous terminal tab", key: "ArrowLeft" },
  { id: "nextTerminal", label: "Next terminal tab", key: "ArrowRight" },
  { id: "tour", label: "Take the welcome tour", key: null },
] as const;
export type CommandId = (typeof COMMANDS)[number]["id"];
export type Bindings = Record<CommandId, string | null>;
export const DEFAULT_BINDINGS = Object.fromEntries(COMMANDS.map((c) => [c.id, c.key])) as Bindings;

/** Store logical keys, with Mod implicit. Clipboard and editor keys remain reserved. */
export function validBinding(key: unknown): key is string | null {
  return (
    key === null ||
    (typeof key === "string" &&
      /^(Alt\+)?([a-z]|[,/]|Arrow(Up|Down|Left|Right))$/.test(key) &&
      !/^(Alt\+)?[cvs]$/.test(key))
  );
}
export function bindingError(bindings: Bindings): string | null {
  const used = new Map<string, string>();
  for (const command of COMMANDS) {
    const key = bindings[command.id];
    if (!validBinding(key))
      return "Use Mod with a letter, arrow, comma or slash. C, V and S are reserved for copy, paste and save.";
    if (key && used.has(key)) return `${command.label} conflicts with ${used.get(key)}.`;
    if (key) used.set(key, command.label);
  }
  return null;
}
export function readBindings(raw: string | undefined): Bindings {
  try {
    const value: unknown = JSON.parse(raw ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return { ...DEFAULT_BINDINGS };
    const bindings = { ...DEFAULT_BINDINGS };
    for (const { id } of COMMANDS) {
      if (Object.hasOwn(value, id)) bindings[id] = (value as Bindings)[id];
    }
    return bindingError(bindings) ? { ...DEFAULT_BINDINGS } : bindings;
  } catch {
    return { ...DEFAULT_BINDINGS };
  }
}
export function eventBinding(event: KeyboardEvent): string | null {
  if (event.isComposing || !isModKey(event) || (!isMac && event.metaKey)) return null;
  let key = shortcutKey(event);
  // Shift is part of Mod on Linux/Windows; punctuation arrives shifted on many layouts.
  if (key === "<") key = ",";
  if (key === "?") key = "/";
  return `${event.altKey ? "Alt+" : ""}${key}`;
}
export function bindingLabel(binding: string | null): string {
  if (!binding) return "Unassigned";
  const alt = binding.startsWith("Alt+");
  const key = alt ? binding.slice(4) : binding;
  return formatShortcut(key.length === 1 ? key.toUpperCase() : key.replace("Arrow", ""), { alt });
}
