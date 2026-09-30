import { create } from "zustand";

export type SidePanel = "left" | "right";

export type SettingsSection = "harnesses" | "workspaces" | "assist" | "general" | "keyboard";

interface LayoutState {
  settingsSection: SettingsSection;
  openSettings: (section: SettingsSection) => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  tourOpen: boolean;
  setTourOpen: (open: boolean) => void;
  collapsed: Record<SidePanel, boolean>;
  settingsOpen: boolean;
  setSettingsOpen: (open: boolean) => void;
  toggle: (panel: SidePanel) => void;
  setCollapsed: (panel: SidePanel, collapsed: boolean) => void;
}

/**
 * Which side panels are collapsed. Panel *sizes* are persisted by the panel group itself; this
 * store only carries intent so that shortcuts and buttons can drive the panels.
 */
export const useLayoutStore = create<LayoutState>((set) => ({
  collapsed: { left: false, right: false },
  settingsOpen: false,
  settingsSection: "harnesses",
  openSettings: (settingsSection) =>
    set({ settingsSection, settingsOpen: true, paletteOpen: false }),
  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  tourOpen: false,
  setTourOpen: (tourOpen) =>
    set({ tourOpen, ...(tourOpen ? { settingsOpen: false, paletteOpen: false } : {}) }),
  setSettingsOpen: (settingsOpen) =>
    set({ settingsOpen, ...(settingsOpen ? { settingsSection: "harnesses" } : {}) }),
  toggle: (panel) =>
    set((state) => ({ collapsed: { ...state.collapsed, [panel]: !state.collapsed[panel] } })),
  setCollapsed: (panel, collapsed) =>
    set((state) =>
      state.collapsed[panel] === collapsed
        ? state
        : { collapsed: { ...state.collapsed, [panel]: collapsed } },
    ),
}));
