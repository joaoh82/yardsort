import { useEffect } from "react";
import { COMMANDS, eventBinding } from "@/lib/shortcuts";
import { usePreferencesStore } from "@/stores/preferences";
import { runCommand } from "./commands";

export function useAppShortcuts() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"]')
      )
        return;
      const binding = eventBinding(event);
      if (!binding) return;
      const bindings = usePreferencesStore.getState().bindings;
      const command = COMMANDS.find((c) => bindings[c.id] === binding);
      if (!command) return;
      event.preventDefault();
      runCommand(command.id);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
