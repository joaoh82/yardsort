// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * R4's two budgets as one phone sees them, for a development build's readout (35 §3, "Measuring the
 * budgets"): how long the last keystroke took to come back, and how long the screen took to be live
 * again after the app came back to the foreground. Timing only: nothing typed or shown is kept.
 *
 * A keystroke counts from being sent to the next output arriving, which on a terminal that echoes
 * is its echo. Coming back counts from the app being in front again to the page having drawn what a
 * new attachment sent.
 */

export interface Timed {
  /** The last keystroke's round trip, in milliseconds. */
  echoMs: number | null;
  /** The last return to the foreground's time to a live screen, in milliseconds. */
  backMs: number | null;
}

export class Timings {
  #timed: Timed = { echoMs: null, backMs: null };
  #typedAt: number | null = null;
  #backAt: number | null = null;
  #listeners = new Set<() => void>();

  constructor(readonly now: () => number = () => performance.now()) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): Timed => this.#timed;

  /** A keystroke went. Timed from the first one still waiting for anything back. */
  typed(): void {
    this.#typedAt ??= this.now();
  }

  /** Output arrived. */
  heard(): void {
    if (this.#typedAt === null) return;
    this.#set({ echoMs: this.now() - this.#typedAt });
    this.#typedAt = null;
  }

  /** The app is in front again. What was typed before does not count. */
  back(): void {
    this.#backAt = this.now();
    this.#typedAt = null;
  }

  /** The page drew everything a live attachment sent it. */
  drawn(): void {
    if (this.#backAt === null) return;
    this.#set({ backMs: this.now() - this.#backAt });
    this.#backAt = null;
  }

  #set(change: Partial<Timed>): void {
    this.#timed = { ...this.#timed, ...change };
    for (const listener of this.#listeners) listener();
  }
}

/** What the readout says. */
export function timingWords({ echoMs, backMs }: Timed): string {
  const ms = (n: number) => (n < 1_000 ? `${Math.round(n)} ms` : `${(n / 1_000).toFixed(1)} s`);
  return [
    `Echo ${echoMs === null ? "not yet" : ms(echoMs)}`,
    `back to live ${backMs === null ? "not yet" : ms(backMs)}`,
  ].join(" · ");
}
