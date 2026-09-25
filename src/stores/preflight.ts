import { create } from "zustand";
import { errorMessage, hasCore, ipc, type Preflight, type YsStatus } from "@/lib/ipc";
import { useAppStore } from "./app";
import { useHarnessStore } from "./harnesses";
import { useProjectsStore } from "./projects";

interface PreflightState {
  report: Preflight | null;
  checking: boolean;
  error: string | null;
  /** Look at the machine. With `reload`, re-read the login shell's environment first. */
  check: (reload?: boolean) => Promise<void>;
  /** Put `ys` on `PATH`. Rejects as the core does — `ys_exists` means ask, then pass `replace`. */
  installYs: (replace?: boolean) => Promise<void>;
}

/** Whether git and at least one agent are here — see `src-tauri/src/preflight.rs`. */
export const usePreflightStore = create<PreflightState>((set, get) => ({
  report: null,
  checking: false,
  error: null,

  async check(reload = false) {
    if (!hasCore() || get().checking) return;
    set({ checking: true, error: null });
    try {
      const was = get().report;
      const report = await ipc.preflight(reload);
      set({ report });
      if (!reload) return;
      // The environment was re-read: everything that depends on it should catch up too.
      useAppStore.setState({ env: report.env });
      void useHarnessStore.getState().reload();
      if (!was?.git.path && report.git.path) void useProjectsStore.getState().load();
    } catch (error) {
      set({ error: errorMessage(error) });
    } finally {
      set({ checking: false });
    }
  },

  async installYs(replace = false) {
    const ys: YsStatus = await ipc.ysInstall(replace);
    const report = get().report;
    if (report) set({ report: { ...report, ys } });
  },
}));
