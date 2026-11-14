// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Chat or Terminal, remembered for each conversation (43). Chat is the default. Kept in a small
 * file of this phone's, by the conversation's id only, and never sent anywhere. The newest
 * choices are kept and the oldest dropped, so the file stays small.
 */

import type { ChoiceFile } from "./appearance";

export type View = "chat" | "terminal";

/** The most conversations a choice is remembered for. */
export const MAX_REMEMBERED = 200;

export class ViewChoices {
  /** Oldest first. */
  #chosen = new Map<string, View>();
  #file: ChoiceFile | null = null;
  #read = false;

  constructor(file: ChoiceFile | null = null) {
    this.#file = file;
  }

  /** Keep choices in `file` from now on, reading what it holds when first asked. */
  start(file: ChoiceFile): void {
    this.#file = file;
    this.#read = false;
    this.#chosen.clear();
  }

  /** What was chosen for `conversation`, or Chat. */
  get(conversation: string): View {
    this.#load();
    return this.#chosen.get(conversation) ?? "chat";
  }

  choose(conversation: string, view: View): void {
    this.#load();
    this.#chosen.delete(conversation);
    this.#chosen.set(conversation, view);
    while (this.#chosen.size > MAX_REMEMBERED) {
      const oldest = this.#chosen.keys().next().value as string;
      this.#chosen.delete(oldest);
    }
    try {
      this.#file?.write(JSON.stringify([...this.#chosen]));
    } catch {
      // It holds until the app is next opened.
    }
  }

  /** Read once, when first asked: a file that cannot be read is no choices, which costs a tap. */
  #load(): void {
    if (this.#read) return;
    this.#read = true;
    try {
      const kept: unknown = JSON.parse(this.#file?.read() ?? "[]");
      if (!Array.isArray(kept)) return;
      for (const entry of kept) {
        if (
          Array.isArray(entry) &&
          typeof entry[0] === "string" &&
          (entry[1] === "chat" || entry[1] === "terminal")
        ) {
          this.#chosen.set(entry[0], entry[1]);
        }
      }
    } catch {
      // Chat, then.
    }
  }
}

/** This phone's choices. Without a file, as in a test, they last as long as the app. */
export const viewChoices = new ViewChoices();
