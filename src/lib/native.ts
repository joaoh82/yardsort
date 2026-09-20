/** Native OS affordances: folder pickers, confirmation boxes, the file manager. */
import { ask, open } from "@tauri-apps/plugin-dialog";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

export const native = {
  /** Resolves to the chosen folder, or `null` if the user cancelled. */
  async pickFolder(title: string, defaultPath?: string): Promise<string | null> {
    const picked = await open({ title, defaultPath, directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  },

  confirm: (
    message: string,
    options: { title: string; okLabel: string; cancelLabel?: string },
  ): Promise<boolean> => ask(message, { cancelLabel: "Cancel", ...options, kind: "warning" }),

  revealInFileManager: (path: string): Promise<void> => revealItemInDir(path),

  /** A desktop notification. Asks for permission the first time; silently does nothing without. */
  async notify(title: string, body: string): Promise<void> {
    const allowed = (await isPermissionGranted()) || (await requestPermission()) === "granted";
    if (allowed) sendNotification({ title, body });
  },
};
