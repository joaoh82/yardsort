// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import type { Vault } from "../../src/lib/profiles";

/** The credential store, as a map: what was kept is in {@link kept}. */
export class MemoryVault implements Vault {
  kept = new Map<string, string>();
  /** Writes to keys matching this fail, as a full or locked store would. */
  failing: RegExp | null = null;

  async get(key: string): Promise<string | null> {
    return this.kept.get(key) ?? null;
  }

  async set(key: string, value: string): Promise<void> {
    if (this.failing?.test(key)) throw new Error("The store refused it.");
    this.kept.set(key, value);
  }

  async delete(key: string): Promise<void> {
    this.kept.delete(key);
  }

  /** The keys of every credential kept. */
  credentials(): string[] {
    return [...this.kept.keys()].filter((k) => k.startsWith("yardsort.refresh."));
  }
}
