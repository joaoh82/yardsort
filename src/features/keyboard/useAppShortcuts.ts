import { useEffect } from "react";
import { COMMANDS, eventBinding } from "@/lib/shortcuts";
import { usePreferencesStore } from "@/stores/preferences";
import { commandEnabled, runCommand } from "./commands";

export function useAppShortcuts() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"]')
      )
        return;
      // Modified arrows belong to caret movement/selection in text controls. xterm's
      // hidden textarea is terminal input: its Mod keys still belong to the app.
      const target = event.target;
      if (
        event.key.startsWith("Arrow") &&
        target instanceof Element &&
        !target.closest(".xterm-helper-textarea") &&
        target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')
      )
        return;
      const binding = eventBinding(event);
      if (!binding) return;
      const bindings = usePreferencesStore.getState().bindings;
      const command = COMMANDS.find((c) => bindings[c.id] === binding);
      if (!command || !commandEnabled(command.id)) return;
      event.preventDefault();
      runCommand(command.id);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
