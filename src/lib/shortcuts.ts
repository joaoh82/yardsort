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
  { id: "usage", label: "Usage: machine resources and tokens", key: null },
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
export interface LoadedBindings {
  bindings: Bindings;
  notice: string | null;
}

/** Saved choices take precedence over defaults introduced by a newer app version. */
export function readBindings(raw: string | undefined): LoadedBindings {
  const fallback = (notice: string | null): LoadedBindings => ({
    bindings: { ...DEFAULT_BINDINGS },
    notice,
  });
  if (raw === undefined) return fallback(null);
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return fallback(
      "Saved shortcuts could not be read. Defaults are active; save shortcuts to replace the unreadable data.",
    );
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return fallback(
      "Saved shortcuts are not a command map. Defaults are active; save shortcuts to replace the invalid data.",
    );
  }
  const saved = value as Record<string, unknown>;
  const bindings = { ...DEFAULT_BINDINGS };
  const used = new Set<string>();
  const needsDefault = new Set<CommandId>();
  const notices: string[] = [];
  for (const { id, label } of COMMANDS) {
    if (!Object.hasOwn(saved, id)) {
      needsDefault.add(id);
      continue;
    }
    const key = saved[id];
    if (!validBinding(key)) {
      needsDefault.add(id);
      notices.push(`${label} had an invalid saved binding; its default was considered instead.`);
      continue;
    }
    if (key && used.has(key)) {
      bindings[id] = null;
      notices.push(
        `${label} was left unassigned because its saved binding duplicates another saved shortcut.`,
      );
    } else {
      bindings[id] = key;
      if (key) used.add(key);
    }
  }
  for (const { id, label } of COMMANDS) {
    if (!needsDefault.has(id)) continue;
    const key = DEFAULT_BINDINGS[id];
    if (key && used.has(key)) {
      bindings[id] = null;
      notices.push(
        `${label} was left unassigned because its default conflicts with a saved shortcut.`,
      );
    } else {
      bindings[id] = key;
      if (key) used.add(key);
    }
  }
  return {
    bindings,
    notice: notices.length
      ? `${notices.join(" ")} Review and save shortcuts to keep these recovered bindings.`
      : null,
  };
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
