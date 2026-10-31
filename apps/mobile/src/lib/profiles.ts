// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The servers this phone is signed in to: one profile each, one active (31 §5).
 *
 * A profile is the server's address, the id it gave for itself, this phone's device id there and
 * its refresh credential. The credential is kept apart from the list, under the profile's own key,
 * and each profile's are its own: nothing of one is ever sent to another's server. Everything is
 * kept in the vault, which on the phone is the system's credential store, readable only while the
 * phone is unlocked and never carried to a backup or a new phone.
 */

import { type DeviceId, Secret, type ServerId } from "@yardsort/remote-client";

/** Where profiles are kept: SecureStore on the phone, a map in tests. */
export interface Vault {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

export interface Profile {
  /** This phone's own name for the profile, random. Never sent anywhere. */
  id: string;
  /** The server's address, as the client reads it: `https://remote.example.com/`. */
  address: string;
  /** What the server said it was when this phone signed in. Checked each time before a refresh. */
  server: ServerId;
  /** Its name, as it gave it. */
  serverName: string;
  /** This phone, at that server. */
  device: DeviceId;
  /** What this phone is called there. */
  deviceName: string;
  /** When it signed in, in milliseconds since the epoch. */
  signedInAt: number;
}

interface Index {
  version: 1;
  active: string | null;
  profiles: Profile[];
}

const INDEX_KEY = "yardsort.profiles";
const ID = /^[0-9a-f]{32}$/;

/** SecureStore keys are letters, digits, `.`, `-` and `_`. */
const refreshKey = (id: string) => `yardsort.refresh.${id}`;

export class Profiles {
  #index: Index;
  /** Writes, one after another, so a quick sign-in and switch cannot interleave. */
  #writing: Promise<unknown> = Promise.resolve();

  private constructor(
    readonly vault: Vault,
    index: Index,
  ) {
    this.#index = index;
  }

  /** What the vault holds. A list that does not read, or an entry that does not, is left out. */
  static async load(vault: Vault): Promise<Profiles> {
    const kept = await vault.get(INDEX_KEY);
    return new Profiles(vault, readIndex(kept));
  }

  get all(): readonly Profile[] {
    return this.#index.profiles;
  }

  get active(): Profile | null {
    return this.#index.profiles.find((p) => p.id === this.#index.active) ?? null;
  }

  /** The profile for this address, if there is one. */
  at(address: string): Profile | null {
    return this.#index.profiles.find((p) => p.address === address) ?? null;
  }

  /**
   * A profile just signed in, made the active one. Its refresh credential is kept before the
   * list names it, so a profile in the list always had one.
   */
  add(profile: Profile, refresh: Secret): Promise<void> {
    if (!ID.test(profile.id)) throw new Error("A profile's id is 32 hexadecimal digits.");
    return this.#write(async () => {
      await this.vault.set(refreshKey(profile.id), refresh.expose());
      const profiles = this.#index.profiles.filter((p) => p.id !== profile.id);
      await this.#save({ version: 1, active: profile.id, profiles: [...profiles, profile] });
    });
  }

  /** Keeps a profile's new refresh credential, in place of the one just spent. */
  keeper(id: string): (refresh: Secret) => Promise<void> {
    return (refresh) => this.#write(() => this.vault.set(refreshKey(id), refresh.expose()));
  }

  /** A profile's refresh credential, or `null` when none is kept. */
  async refresh(id: string): Promise<Secret | null> {
    await this.#writing.catch(() => {});
    const value = await this.vault.get(refreshKey(id));
    return value ? new Secret(value) : null;
  }

  activate(id: string): Promise<void> {
    return this.#write(async () => {
      if (!this.#index.profiles.some((p) => p.id === id)) throw new Error("No such profile.");
      await this.#save({ ...this.#index, active: id });
    });
  }

  /**
   * Delete a profile from this phone: its credential first, then its entry. Another profile, if
   * there is one, becomes the active one only when the person picks it.
   */
  forget(id: string): Promise<void> {
    return this.#write(async () => {
      await this.vault.delete(refreshKey(id));
      const profiles = this.#index.profiles.filter((p) => p.id !== id);
      const active = this.#index.active === id ? null : this.#index.active;
      await this.#save({ version: 1, active, profiles });
    });
  }

  #write<T>(write: () => Promise<T>): Promise<T> {
    const next = this.#writing.catch(() => {}).then(write);
    this.#writing = next;
    return next;
  }

  async #save(index: Index): Promise<void> {
    await this.vault.set(INDEX_KEY, JSON.stringify(index));
    this.#index = index;
  }
}

function readIndex(kept: string | null): Index {
  const empty: Index = { version: 1, active: null, profiles: [] };
  if (!kept) return empty;
  let value: unknown;
  try {
    value = JSON.parse(kept);
  } catch {
    return empty;
  }
  if (!isRecord(value) || value.version !== 1 || !Array.isArray(value.profiles)) return empty;
  const profiles = value.profiles.filter(isProfile);
  const active =
    typeof value.active === "string" && profiles.some((p) => p.id === value.active)
      ? value.active
      : null;
  return { version: 1, active, profiles };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isProfile(value: unknown): value is Profile {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    ID.test(value.id) &&
    typeof value.address === "string" &&
    typeof value.server === "string" &&
    typeof value.serverName === "string" &&
    typeof value.device === "string" &&
    typeof value.deviceName === "string" &&
    typeof value.signedInAt === "number"
  );
}
