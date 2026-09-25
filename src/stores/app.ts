import { create } from "zustand";
import { hasCore, ipc, type AppInfo, type DaemonStatus, type EnvInfo } from "@/lib/ipc";

interface AppState {
  info: AppInfo | null;
  env: EnvInfo | null;
  /** Where terminals live: the daemon, or this process when one could not be reached. */
  daemon: DaemonStatus | null;
  /** Mirrors the general setting, so event handlers need not ask the core each time. */
  notifyWhenQuiet: boolean;
  checkForUpdates: boolean;
  /** Mirrors the activity setting: whether a workspace offers its experimental timeline. */
  showTimeline: boolean;
  load: () => Promise<void>;
}

/** Facts about the running app and the environment it launches programs in. */
export const useAppStore = create<AppState>((set) => ({
  info: null,
  env: null,
  daemon: null,
  notifyWhenQuiet: true,
  // Off until the settings say otherwise, so nothing phones home before they are read.
  checkForUpdates: false,
  showTimeline: false,
  async load() {
    if (!hasCore()) return;
    set({ info: await ipc.appInfo(), daemon: await ipc.daemonStatus() });
    const settings = await ipc.settingsGet();
    set({
      notifyWhenQuiet: settings.notifyWhenQuiet,
      checkForUpdates: settings.checkForUpdates,
      showTimeline: settings.activity.showTimeline,
    });
    // Slower: the first call waits for the login shell to report its environment.
    set({ env: await ipc.envInfo() });
  },
}));
