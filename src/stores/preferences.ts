import { create } from "zustand";
import { errorMessage, hasCore, ipc } from "@/lib/ipc";
import {
  bindingError,
  bindingLabel,
  DEFAULT_BINDINGS,
  readBindings,
  type Bindings,
  type CommandId,
} from "@/lib/shortcuts";

export const BINDINGS_KEY = "keyboard.bindings";
export const TOUR_KEY = "onboarding.welcomeSeen";
interface Preferences {
  loaded: boolean;
  bindings: Bindings;
  bindingNotice: string | null;
  welcomeSeen: boolean;
  error: string | null;
  saving: boolean;
  load: () => Promise<void>;
  saveBindings: (bindings: Bindings) => Promise<boolean>;
  dismissWelcome: () => Promise<boolean>;
}
/** A cache of the core's SQLite UI preferences, never browser-local storage. */
export const usePreferencesStore = create<Preferences>((set, get) => ({
  loaded: false,
  bindings: { ...DEFAULT_BINDINGS },
  bindingNotice: null,
  welcomeSeen: false,
  error: null,
  saving: false,
  async load() {
    if (!hasCore()) return;
    try {
      const ui = await ipc.uiStateLoad();
      const { bindings, notice } = readBindings(ui[BINDINGS_KEY]);
      set({
        loaded: true,
        bindings,
        bindingNotice: notice,
        welcomeSeen: ui[TOUR_KEY] === "true",
        error: null,
      });
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },
  async saveBindings(bindings) {
    if (!get().loaded || get().saving) return false;
    const problem = bindingError(bindings);
    if (problem) {
      set({ error: problem });
      return false;
    }
    set({ saving: true, error: null });
    try {
      await ipc.uiStateSave(BINDINGS_KEY, JSON.stringify(bindings));
      set({ bindings: { ...bindings }, bindingNotice: null });
      return true;
    } catch (error) {
      set({ error: errorMessage(error) });
      return false;
    } finally {
      set({ saving: false });
    }
  },
  async dismissWelcome() {
    if (get().saving) return false;
    set({ saving: true, error: null });
    try {
      await ipc.uiStateSave(TOUR_KEY, "true");
      set({ welcomeSeen: true });
      return true;
    } catch (error) {
      set({ error: errorMessage(error) });
      return false;
    } finally {
      set({ saving: false });
    }
  },
}));
export function useShortcutLabel(id: CommandId): string {
  return bindingLabel(usePreferencesStore((s) => s.bindings[id]));
}
