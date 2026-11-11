// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * When the phone's network changes: another kind (Wi-Fi, cellular), or connected again or not. The
 * platform says more often than that, with the same state again; only a change is passed on. What
 * hears it is each screen's {@link Presence}, which then checks its channels at once rather than
 * waiting out a backoff (35 §17, slice 8).
 */

import type { Diagnostics } from "./diagnostics";

/** What the platform says about the network, as much as this needs. */
export interface NetworkState {
  type?: string | undefined;
  isConnected?: boolean | undefined;
  isInternetReachable?: boolean | undefined;
}

/** Subscribe to the platform's network states; returns how to stop. */
export type NetworkSource = (heard: (state: NetworkState) => void) => () => void;

/** One subscription to `source` for however many listeners, passing on only real changes. */
export class NetworkChanges {
  #listeners = new Set<() => void>();
  #stop: (() => void) | null = null;
  #last: string | null = null;

  constructor(
    readonly source: NetworkSource,
    /** Where each change is recorded: the kind of network and whether it is connected. */
    readonly diagnostics?: Diagnostics,
  ) {}

  /** Hear each change from now on; returns how to stop hearing them. */
  listen(changed: () => void): () => void {
    this.#listeners.add(changed);
    this.#stop ??= this.source((state) => this.#heard(state));
    return () => {
      this.#listeners.delete(changed);
      if (this.#listeners.size === 0) {
        this.#stop?.();
        this.#stop = null;
        this.#last = null;
      }
    };
  }

  #heard(state: NetworkState): void {
    const now = `${state.type ?? "?"}:${String(state.isConnected)}:${String(state.isInternetReachable)}`;
    const before = this.#last;
    this.#last = now;
    // The first state heard is where it starts, not a change.
    if (before === null || before === now) return;
    this.diagnostics?.record({ what: "network", network: now });
    for (const listener of this.#listeners) listener();
  }
}
