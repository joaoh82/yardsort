// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Giving an agent a picture from the phone: it is put on the computer, outside every worktree,
 * and its path there is pasted into the terminal, as dropping a file on the desktop's terminal
 * pastes one. An agent that takes pictures reads a pasted path to one as the picture.
 */

import type { Platform } from "@yardsort/remote-client";

/** A picture the person chose, ready to send. */
export interface Picked {
  /** What it is called, for the file's name on the computer. */
  name: string;
  bytes: Uint8Array;
}

/** Ask the person for a picture. `null` when they chose none. */
export type PickImage = () => Promise<Picked | null>;

/** The longest side a picture is sent at. Larger only costs time: agents scale them down. */
export const LONGEST_SIDE = 2048;

/**
 * A path as the desktop's terminal pastes a dropped file: quoted for the computer's shell, with a
 * space after it so typing can carry on.
 */
export function pastedPath(path: string, platform: Platform): string {
  if (platform === "windows") {
    return `${/[\s&()^;,=|<>%']/.test(path) ? `"${path}"` : path} `;
  }
  return `${path.replace(/[\s!"#$&'()*;<>?[\\\]^`{|}~]/g, "\\$&")} `;
}
