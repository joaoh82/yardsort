// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Which projects are folded on a computer's screen: their workspaces put away under the project's
 * heading until it is pressed again. Remembered on this phone, for each computer, as the desktop
 * remembers its own sidebar. Kept in a small file by the computer's and the project's ids only,
 * and never sent anywhere. The newest are kept and the oldest dropped, so the file stays small.
 */

import type { ChoiceFile } from "./appearance";

/** The most projects a fold is remembered for. */
export const MAX_REMEMBERED = 200;

export class Folds {
  /** Oldest first. */
  #folded = new Set<string>();
  #file: ChoiceFile | null = null;
  #read = false;

  constructor(file: ChoiceFile | null = null) {
    this.#file = file;
  }

  /** Keep folds in `file` from now on, reading what it holds when first asked. */
  start(file: ChoiceFile): void {
    this.#file = file;
    this.#read = false;
    this.#folded.clear();
  }

  /** Whether `project` of `computer` is folded. Open unless it was folded here. */
  has(computer: string, project: string): boolean {
    this.#load();
    return this.#folded.has(key(computer, project));
  }

  /** Fold it when open, open it when folded. Says whether it is folded now. */
  toggle(computer: string, project: string): boolean {
    this.#load();
    const which = key(computer, project);
    const folded = !this.#folded.delete(which);
    if (folded) this.#folded.add(which);
    while (this.#folded.size > MAX_REMEMBERED) {
      const oldest = this.#folded.values().next().value as string;
      this.#folded.delete(oldest);
    }
    try {
      this.#file?.write(JSON.stringify([...this.#folded]));
    } catch {
      // It holds until the app is next opened.
    }
    return folded;
  }

  /** Read once, when first asked: a file that cannot be read is nothing folded, which costs a tap. */
  #load(): void {
    if (this.#read) return;
    this.#read = true;
    try {
      const kept: unknown = JSON.parse(this.#file?.read() ?? "[]");
      if (!Array.isArray(kept)) return;
      for (const entry of kept) {
        if (typeof entry === "string") this.#folded.add(entry);
      }
    } catch {
      // Everything open, then.
    }
  }
}

/** Ids hold no space, so one between them keeps two pairs from reading as the same. */
function key(computer: string, project: string): string {
  return `${computer} ${project}`;
}

/** This phone's folds. Without a file, as in a test, they last as long as the app. */
export const folds = new Folds();
