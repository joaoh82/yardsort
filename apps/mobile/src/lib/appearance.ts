// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Light, dark, or as the phone is (33). Held here as well as told to the system, so the screens
 * change on the press rather than on the system's word that it changed.
 */

import { Appearance } from "react-native";

export const CHOICES = ["automatic", "light", "dark"] as const;
export type Choice = (typeof CHOICES)[number];

/**
 * Where the choice is kept. Read before the first screen is drawn, so it never flashes the other.
 * Both are done, or have thrown, by the time they return.
 */
export interface ChoiceFile {
  read(): string | null;
  write(choice: string): void;
}

const isChoice = (value: string | null): value is Choice =>
  CHOICES.some((choice) => choice === value);

export class AppearanceChoice {
  private choice: Choice = "automatic";
  private file: ChoiceFile | null = null;
  private readonly listeners = new Set<() => void>();

  /** A choice that cannot be read is no choice: losing one costs a press, and nothing else. */
  start(file: ChoiceFile): void {
    this.file = file;
    let kept: string | null = null;
    try {
      kept = file.read();
    } catch {
      // As the phone is.
    }
    if (isChoice(kept)) this.set(kept);
  }

  choose(choice: Choice): void {
    this.set(choice);
    try {
      this.file?.write(choice);
    } catch {
      // It holds until the app is next opened.
    }
  }

  get = (): Choice => this.choice;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private set(choice: Choice): void {
    this.choice = choice;
    // Alerts, the keyboard and the system's header are drawn from this.
    Appearance.setColorScheme(choice === "automatic" ? "unspecified" : choice);
    for (const listener of this.listeners) listener();
  }
}

export const appearance = new AppearanceChoice();
