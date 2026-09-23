import { create } from "zustand";
import { errorMessage, hasCore, ipc, type DownloadProgress, type UpdateStatus } from "@/lib/ipc";

interface UpdatesState {
  status: UpdateStatus | null;
  checking: boolean;
  /** When the last check finished, successfully or not. `null` until one has. */
  checkedAt: number | null;
  /** Set while an update downloads and installs. */
  progress: DownloadProgress | null;
  installing: boolean;
  error: string | null;
  /** The update dialog is showing. */
  open: boolean;

  /**
   * Look for a newer release. A check the user asked for reports failures; the automatic one
   * stays quiet — being offline is not news.
   */
  check: (options?: { manual?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  show: (open: boolean) => void;
}

export const useUpdatesStore = create<UpdatesState>((set, get) => ({
  status: null,
  checking: false,
  checkedAt: null,
  progress: null,
  installing: false,
  error: null,
  open: false,

  async check({ manual = false } = {}) {
    if (!hasCore() || get().checking || get().installing) return;
    set({ checking: true, error: null });
    try {
      set({ status: await ipc.updateCheck() });
    } catch (error) {
      if (manual) set({ error: errorMessage(error) });
      else console.warn("Update check failed:", errorMessage(error));
    } finally {
      set({ checking: false, checkedAt: Date.now() });
    }
  },

  async install() {
    if (get().installing) return;
    set({ installing: true, error: null, progress: { downloaded: 0, total: null } });
    try {
      // On success the app restarts, so this only ever "returns" by failing.
      await ipc.updateInstall((progress) => set({ progress }));
    } catch (error) {
      set({ error: errorMessage(error), installing: false, progress: null });
    }
  },

  show: (open) => set({ open, ...(open ? {} : { error: null }) }),
}));

/** Check a few seconds after start, then once a day. */
export const FIRST_CHECK_MS = 8_000;
export const CHECK_EVERY_MS = 24 * 60 * 60 * 1000;

/** Whether the last check is old enough to be worth repeating — see `useUpdateChecks`. */
export const checkIsStale = (checkedAt: number | null, now = Date.now()) =>
  checkedAt === null || now - checkedAt >= CHECK_EVERY_MS;
