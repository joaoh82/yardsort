// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Signing in, staying signed in and signing out, for every profile on the phone. Screens read a
 * {@link Snapshot} and call these; none of them builds a request or holds a credential itself.
 *
 * - **Signing in** goes through the system browser and comes back to `yardsort://signed-in`
 *   (31 §5). The refresh credential is kept before anything uses the access that came with it.
 * - **Opening a profile** first asks its address who it is. Only when it answers with the server
 *   id the profile was signed in to is the refresh credential sent there: an address that now
 *   answers as another server never sees it.
 * - **A credential the server refuses** means this phone was signed out there. The profile is
 *   deleted, and the phone says so. **A server that cannot be reached** deletes nothing.
 */

import {
  type Client,
  Client as ServerClient,
  ClientError,
  codeFromRedirect,
  type Crypto,
  type DevicePage,
  NotKept,
  type Platform,
  type ServerDescription,
  Session,
} from "@yardsort/remote-client";

import { type Profile, Profiles, type Vault } from "./profiles";

/** Where every sign-in comes back to. Every server allows it (31 §5). */
export const REDIRECT = "yardsort://signed-in";

/**
 * Opens `url` in the system browser for sign-in, and gives back the address it was sent to at
 * `redirect`, or `null` when the person closed it.
 */
export type Browser = (url: string, redirect: string) => Promise<string | null>;

/** What the app is, and what it runs on. */
export interface Env {
  crypto: Crypto;
  platform: Platform;
  /** `yardsort-mobile/0.1.0 (ios)`. */
  userAgent: string;
  fetch?: typeof fetch;
  /** Milliseconds since the epoch. */
  clock?: () => number;
}

/** Where the active profile stands. */
export type Standing =
  | { kind: "opening" }
  | { kind: "signedIn"; session: Session }
  | { kind: "unreachable"; message: string }
  /** The address now answers as another server. Nothing was sent to it. */
  | { kind: "otherServer"; name: string };

export interface Snapshot {
  /** Whether the profiles have been read from the vault yet. */
  loaded: boolean;
  profiles: readonly Profile[];
  active: Profile | null;
  /** `null` when no profile is active. */
  standing: Standing | null;
  /** Something the person should be told once, such as being signed out at the server. */
  notice: string | null;
}

/** A server looked up, before signing in to it. */
export interface Found {
  client: Client;
  description: ServerDescription;
}

export class Accounts {
  #profiles: Profiles | null = null;
  #standing: Standing | null = null;
  #notice: string | null = null;
  #snapshot: Snapshot;
  /** Sessions opened since the app started, by profile. Switching back reuses one. */
  #sessions = new Map<string, Session>();
  /**
   * Opening under way, by profile. Opening spends the refresh credential, so a second caller waits
   * for the first instead of spending the same one again.
   */
  #opening = new Map<string, Promise<void>>();
  #loading: Promise<void> | null = null;
  #listeners = new Set<() => void>();

  constructor(
    readonly env: Env,
    readonly vault: Vault,
  ) {
    this.#snapshot = this.#build();
  }

  // --- what screens read -----------------------------------------------------------------------

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** The same object until something changes, as `useSyncExternalStore` needs. */
  getSnapshot = (): Snapshot => this.#snapshot;

  #build(): Snapshot {
    const profiles = this.#profiles;
    return {
      loaded: profiles !== null,
      profiles: profiles?.all ?? [],
      active: profiles?.active ?? null,
      standing: profiles?.active ? this.#standing : null,
      notice: this.#notice,
    };
  }

  #changed(): void {
    this.#snapshot = this.#build();
    for (const listener of this.#listeners) listener();
  }

  dismissNotice(): void {
    this.#notice = null;
    this.#changed();
  }

  // --- starting --------------------------------------------------------------------------------

  /** Read the profiles, and open the active one. Once: a second call waits for the first. */
  load(): Promise<void> {
    this.#loading ??= (async () => {
      this.#profiles = await Profiles.load(this.vault);
      this.#changed();
      await this.open();
    })().catch((error: unknown) => {
      // Not read: the next call tries again.
      this.#loading = null;
      throw error;
    });
    return this.#loading;
  }

  #need(): Profiles {
    if (!this.#profiles) throw new Error("The profiles have not been read yet.");
    return this.#profiles;
  }

  clientFor(address: string): Client {
    return new ServerClient(address, {
      crypto: this.env.crypto,
      userAgent: this.env.userAgent,
      ...(this.env.fetch ? { fetch: this.env.fetch } : {}),
    });
  }

  // --- signing in ------------------------------------------------------------------------------

  /** Ask a server who it is, before anything is sent to it. */
  async lookUp(address: string): Promise<Found> {
    const client = this.clientFor(address);
    return { client, description: await client.describe() };
  }

  /**
   * Sign in to a server looked up, as `deviceName`, and make it the active profile. Returns
   * `false` when the person closed the browser before finishing.
   */
  async signIn(found: Found, deviceName: string, browser: Browser): Promise<boolean> {
    const profiles = this.#need();
    const { client, description } = found;
    const { url, verifier } = await client.beginSignIn(REDIRECT);
    const returned = await browser(url, REDIRECT);
    if (returned === null) return false;
    const issued = await client.signIn(codeFromRedirect(returned), verifier, {
      name: deviceName.trim(),
      kind: "phone",
      platform: this.env.platform,
    });
    const profile: Profile = {
      id: newId(this.env.crypto),
      address: client.address,
      server: description.server,
      serverName: description.name,
      device: issued.device,
      deviceName: deviceName.trim(),
      signedInAt: (this.env.clock ?? Date.now)(),
    };
    try {
      await profiles.add(profile, issued.refresh);
    } catch (error) {
      // Nothing kept means nothing could sign this phone out later: undo the sign-in instead of
      // leaving a device at the server that nothing holds. Best effort.
      await client.revokeDevice(issued.access, issued.device).catch(() => {});
      throw error;
    }
    // A profile signed in again at the same address replaces the one before it.
    for (const old of profiles.all) {
      if (old.address === profile.address && old.id !== profile.id) await profiles.forget(old.id);
    }
    this.#sessions.set(profile.id, new Session(client, issued, profiles.keeper(profile.id)));
    this.#standing = { kind: "signedIn", session: this.#sessions.get(profile.id) as Session };
    this.#notice = null;
    this.#changed();
    return true;
  }

  // --- the active profile ----------------------------------------------------------------------

  /** Switch to another profile, and open it. */
  async activate(id: string): Promise<void> {
    await this.#need().activate(id);
    this.#standing = null;
    this.#changed();
    await this.open();
  }

  /**
   * Open the active profile: check its server is still the one it signed in to, and refresh its
   * credential. Called again to retry after `unreachable`.
   */
  open(): Promise<void> {
    const profile = this.#need().active;
    if (!profile) return Promise.resolve();
    let opening = this.#opening.get(profile.id);
    if (!opening) {
      opening = this.#open(profile).finally(() => this.#opening.delete(profile.id));
      this.#opening.set(profile.id, opening);
    }
    return opening;
  }

  async #open(profile: Profile): Promise<void> {
    const profiles = this.#need();
    const open = this.#sessions.get(profile.id);
    if (open) {
      this.#set(profile, { kind: "signedIn", session: open });
      return;
    }
    this.#set(profile, { kind: "opening" });
    const client = this.clientFor(profile.address);
    let standing: Standing;
    try {
      const description = await client.describe();
      if (description.server !== profile.server) {
        standing = { kind: "otherServer", name: description.name };
      } else {
        const refresh = await profiles.refresh(profile.id);
        if (!refresh) return await this.#signedOut(profile);
        const session = await Session.resume(client, refresh, profiles.keeper(profile.id));
        this.#sessions.set(profile.id, session);
        standing = { kind: "signedIn", session };
      }
    } catch (error) {
      if (error instanceof NotKept) {
        // The session holds the only copy of the new credential: carry on, and say so.
        this.#sessions.set(profile.id, error.session);
        standing = { kind: "signedIn", session: error.session };
        this.#notice =
          "The new sign-in could not be saved on this phone. It works until the app closes; then sign in again.";
      } else if (refused(error)) {
        return await this.#signedOut(profile);
      } else {
        standing = { kind: "unreachable", message: messageOf(error) };
      }
    }
    this.#set(profile, standing);
  }

  /** Set where `profile` stands, unless another profile became the active one meanwhile. */
  #set(profile: Profile, standing: Standing): void {
    if (this.#profiles?.active?.id !== profile.id) return;
    this.#standing = standing;
    this.#changed();
  }

  /** The server refused this phone's credential: forget the profile, and say so. */
  async #signedOut(profile: Profile): Promise<void> {
    this.#sessions.delete(profile.id);
    await this.#need().forget(profile.id);
    this.#standing = null;
    this.#notice = `This phone was signed out of ${profile.serverName}. Nothing from it is kept here.`;
    this.#changed();
  }

  #session(): { profile: Profile; session: Session } {
    const profile = this.#need().active;
    const standing = this.#standing;
    if (!profile || standing?.kind !== "signedIn")
      throw new ClientError("invalid", "Not signed in.");
    return { profile, session: standing.session };
  }

  /** Run `call` with the active profile's access, signing the profile out if the server refuses it. */
  async #withAccess<T>(call: (session: Session) => Promise<T>): Promise<T> {
    const { profile, session } = this.#session();
    try {
      return await call(session);
    } catch (error) {
      if (refused(error)) await this.#signedOut(profile);
      throw error;
    }
  }

  /** The account's devices, this phone among them. */
  devices(): Promise<DevicePage> {
    return this.#withAccess(async (session) => session.client.devices(await session.access()));
  }

  /**
   * Sign out: revoke this phone at the server, which takes back every computer's grant, then
   * delete the profile here. When the server cannot be reached this throws, and nothing is
   * deleted: see {@link forgetHere}.
   */
  async signOut(): Promise<void> {
    const { profile } = this.#session();
    try {
      await this.#withAccess((session) => session.signOut());
    } catch (error) {
      // Signed out already: #withAccess has forgotten the profile.
      if (refused(error)) return;
      throw error;
    }
    this.#sessions.delete(profile.id);
    await this.#need().forget(profile.id);
    this.#standing = null;
    this.#changed();
  }

  /**
   * Delete a profile from this phone only. The server still counts this phone as signed in until
   * it is revoked from another of the account's devices.
   */
  async forgetHere(id: string): Promise<void> {
    this.#sessions.delete(id);
    const wasActive = this.#need().active?.id === id;
    await this.#need().forget(id);
    if (wasActive) this.#standing = null;
    this.#changed();
  }
}

/** The server refused this phone's own credential. */
function refused(error: unknown): boolean {
  return error instanceof ClientError && error.code === "unauthenticated";
}

/** A message to show a person. A client error's is written for one; anything else is not. */
export function messageOf(error: unknown): string {
  if (error instanceof ClientError) return error.message;
  return "Something went wrong on this phone.";
}

/** 128 random bits, as 32 hexadecimal digits. */
function newId(crypto: Crypto): string {
  return Array.from(crypto.random(16), (b) => b.toString(16).padStart(2, "0")).join("");
}
