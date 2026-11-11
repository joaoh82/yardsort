// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Signing in, staying signed in and signing out, for every profile on the phone. Screens read a
 * {@link Snapshot} and call these; none of them builds a request or holds a credential itself.
 *
 * - **Signing in** goes through the system browser and comes back to `yardsort://signed-in`
 *   (31 §5). The refresh credential is kept before anything uses the access that came with it.
 * - **Every refresh**, on opening a profile or later in a session already open, first asks the
 *   address who it is. Only when it answers with the server id the profile was signed in to is the
 *   refresh credential sent there: an address that now answers as another server never sees it.
 * - **A refresh credential the server refuses** means this phone was signed out there. The profile
 *   is deleted, and the phone says so. A refused access means less: the server says the same for
 *   access that has lapsed, so it is refreshed and tried once more first. **A server that cannot
 *   be reached** deletes nothing.
 * - **A computer that refuses this phone** says nothing about why. The server is asked: see
 *   {@link Accounts.explain}.
 */

import {
  type Channel,
  type Client,
  Client as ServerClient,
  ClientError,
  codeFromRedirect,
  type Crypto,
  type DevicePage,
  type GrantId,
  type GrantSummary,
  type HostId,
  NotKept,
  type OfferKey,
  openChannel,
  type Platform,
  type ProjectGrant,
  type Scope,
  type Secret,
  type ServerDescription,
  Session,
  type SessionOptions,
  type SocketFactory,
} from "@yardsort/remote-client";

import { Observed, type Shelf } from "./observed";
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
  /** Opens a relay socket. The platform's `WebSocket` unless given. */
  socket?: SocketFactory;
  /** Where what was last seen is kept. Without one, nothing is. */
  shelf?: Shelf;
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
  /** The active profile's computers that stopped letting this phone in, until the person has read it. */
  lost: readonly Lost[];
}

/** Why a computer stopped letting this phone in: it took the phone back, or left the account. */
export type Parted = "tookBack" | "removed";

/** A computer this phone used to reach, and no longer can. */
export interface Lost {
  host: HostId;
  name: string;
  why: Parted;
}

/** What a computer refusing this phone came to, as the server tells it (31 §5). */
export type Refusal =
  | Parted
  /** The server refused this phone's own credential. The profile is deleted by now. */
  | "signedOut"
  /** The server could not be asked, or does not know of anything changing. Nothing is deleted. */
  | "undetermined";

/** A computer that lets this phone in. */
export interface Computer {
  grant: GrantId;
  host: HostId;
  name: string;
  /** History, not presence: when the server last heard from it. */
  lastSeenAt: number | null;
  scopes: Scope[];
  projects: ProjectGrant | null;
}

/** The active profile's computers, and the requests still waiting for an answer at one. */
export interface ComputerList {
  computers: Computer[];
  waiting: GrantSummary[];
  /** Every computer on the account, whether or not it lets this phone in. */
  onAccount: ReadonlySet<HostId>;
}

/** What every pairing asks for: all a phone uses. The computer's person chooses (27 §7). */
export const ASKED_SCOPES: Scope[] = [
  "workspace:read",
  "activity:read",
  "memory:read",
  "terminal:read",
  "terminal:control",
];

/** A server looked up, before signing in to it. */
export interface Found {
  client: Client;
  description: ServerDescription;
}

/** The profile's address answered as another server. Its refresh credential was not sent. */
class OtherServer extends ClientError {
  constructor(readonly serverName: string) {
    super("invalid", `This address now answers as another server, “${serverName}”.`);
    this.name = "OtherServer";
  }
}

export class Accounts {
  #profiles: Profiles | null = null;
  #observed = new Map<string, Observed>();
  #standing: Standing | null = null;
  #notice: string | null = null;
  /** By profile, and only in memory: a list read later finds nothing more to say. */
  #lost = new Map<string, Lost[]>();
  /** Computers being unpaired here, whose going is no news to anyone. */
  #unpairing = new Set<HostId>();
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
      lost: (profiles?.active && this.#lost.get(profiles.active.id)) || [],
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

  dismissLost(host: HostId): void {
    const id = this.#profiles?.active?.id;
    if (!id) return;
    this.#lost.set(id, this.#lost.get(id)?.filter((lost) => lost.host !== host) ?? []);
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
      if (old.address === profile.address && old.id !== profile.id) await this.#forget(old.id);
    }
    this.#sessions.set(
      profile.id,
      new Session(client, issued, profiles.keeper(profile.id), this.#options(profile, client)),
    );
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
   * How every session of `profile` runs. By the wall clock, since the phone's monotonic clock may
   * stop while it sleeps. And with its address asked who it is before each refresh, so a session
   * open while a server is replaced is held to what opening one is.
   */
  #options(profile: Profile, client: Client): SessionOptions {
    return {
      now: this.env.clock ?? Date.now,
      beforeRefresh: async () => {
        const { server, name } = await client.describe();
        if (server !== profile.server) throw new OtherServer(name);
      },
    };
  }

  /**
   * Open the active profile: refresh its credential, once its address has answered as the server
   * it signed in to. Called again to retry after `unreachable`.
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
      const refresh = await profiles.refresh(profile.id);
      if (!refresh) return await this.#signedOut(profile);
      const session = await Session.resume(
        client,
        refresh,
        profiles.keeper(profile.id),
        this.#options(profile, client),
      );
      this.#sessions.set(profile.id, session);
      standing = { kind: "signedIn", session };
    } catch (error) {
      if (error instanceof OtherServer) {
        standing = { kind: "otherServer", name: error.serverName };
      } else if (error instanceof NotKept) {
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
    await this.#forget(profile.id);
    this.#notice = `This phone was signed out of ${profile.serverName}. Nothing from it is kept here.`;
    this.#changed();
  }

  /**
   * Delete a profile here. Where the active profile stands is cleared only when that is the one
   * deleted: an answer that comes late for a profile switched away from says nothing about the
   * one in use now.
   */
  async #forget(id: string): Promise<void> {
    this.#sessions.delete(id);
    this.#lost.delete(id);
    await this.#need().forget(id);
    await this.#observedFor(id)?.forget();
    this.#observed.delete(id);
    if (!this.#need().active) this.#standing = null;
  }

  #session(): { profile: Profile; session: Session } {
    const profile = this.#need().active;
    const standing = this.#standing;
    if (!profile || standing?.kind !== "signedIn")
      throw new ClientError("invalid", "Not signed in.");
    return { profile, session: standing.session };
  }

  /**
   * Run `call` with the active profile's access. Refused, it is refreshed and tried once more: the
   * server says the same for access that has lapsed, which a phone that slept can hand out. Only
   * a refusal after that means this phone was signed out, and deletes the profile.
   */
  async #withAccess<T>(call: (session: Session, access: Secret) => Promise<T>): Promise<T> {
    const { profile, session } = this.#session();
    return this.#withAccessOf(profile, session, call);
  }

  async #withAccessOf<T>(
    profile: Profile,
    opened: Session | Promise<Session>,
    call: (session: Session, access: Secret) => Promise<T>,
  ): Promise<T> {
    try {
      const session = await opened;
      const access = await session.access();
      try {
        return await call(session, access);
      } catch (error) {
        if (!refused(error)) throw error;
        return await call(session, await session.accessAfterRefusal(access));
      }
    } catch (error) {
      if (refused(error)) {
        await this.#signedOut(profile);
      } else if (error instanceof OtherServer) {
        // Opening it again asks again, from the credential kept.
        this.#sessions.delete(profile.id);
        this.#set(profile, { kind: "otherServer", name: error.serverName });
      }
      throw error;
    }
  }

  /** What the active profile last saw, when there is somewhere to keep it. */
  observed(): Observed | null {
    const id = this.#profiles?.active?.id;
    return id ? this.#observedFor(id) : null;
  }

  #observedFor(id: string): Observed | null {
    const { shelf } = this.env;
    if (!shelf) return null;
    let observed = this.#observed.get(id);
    if (!observed) {
      observed = new Observed(shelf, `observed.${id}`, this.env.clock ?? Date.now);
      this.#observed.set(id, observed);
    }
    return observed;
  }

  /** The account's devices, this phone among them. */
  devices(): Promise<DevicePage> {
    return this.#withAccess((session, access) => session.client.devices(access));
  }

  /** What this phone is set to receive. */
  notificationPreferences(): Promise<import("@yardsort/remote-client").NotificationPreferences> {
    return this.#withAccess((session, access) => session.client.notificationPreferences(access));
  }

  /** Replace what this phone is set to receive. */
  setNotificationPreferences(
    preferences: import("@yardsort/remote-client").NotificationPreferences,
  ): Promise<void> {
    return this.#withAccess((session, access) =>
      session.client.setNotificationPreferences(access, preferences),
    );
  }

  /** Ask for notices still queued. Does not ask the operating system for a token. */
  refreshQueuedNotifications(): Promise<void> {
    return this.#withAccess(async (session, access) => {
      await session.client.markNotificationsReachable(access);
    });
  }

  /**
   * Register a push token when the phone has one, and ask for anything still queued, for every
   * account this phone is signed in to. The one in use is first. One account that cannot be
   * reached does not stop the others. When none could, the last error is thrown.
   */
  async registerForNotifications(
    readToken: () => Promise<{ platform: "apns" | "fcm"; token: string } | null>,
  ): Promise<void> {
    const token = await readToken();
    const index = this.#need();
    const ordered = [
      ...(index.active ? [index.active] : []),
      ...index.all.filter((profile) => profile.id !== index.active?.id),
    ];
    if (ordered.length === 0) return;
    let last: unknown;
    let registered = false;
    for (const profile of ordered) {
      try {
        await this.#withAccessOf(profile, this.#sessionOf(profile), async (session, access) => {
          if (token) await session.client.registerPushToken(access, token);
          await session.client.markNotificationsReachable(access);
        });
        registered = true;
      } catch (error) {
        last = error;
      }
    }
    if (!registered && last !== undefined) throw last;
  }

  /**
   * One notification, for the screen a tap opens. The notice may belong to any account this
   * phone is signed in to, so each is asked, the one in use first. A miss on one account is not
   * a miss on the next.
   */
  async notice(id: string): Promise<import("@yardsort/remote-client").NoticeView> {
    const index = this.#need();
    const ordered = [
      ...(index.active ? [index.active] : []),
      ...index.all.filter((profile) => profile.id !== index.active?.id),
    ];
    if (ordered.length === 0) throw new ClientError("invalid", "Not signed in.");
    let last: unknown = new ClientError("invalid", "Not signed in.");
    for (const profile of ordered) {
      try {
        return await this.#withAccessOf(profile, this.#sessionOf(profile), (session, access) =>
          session.client.notice(access, id),
        );
      } catch (error) {
        last = error;
        if (
          error instanceof ClientError &&
          (error.code === "not_found" || error.code === "unauthenticated")
        ) {
          continue;
        }
        throw error;
      }
    }
    throw last;
  }

  /**
   * The computers that let this phone in. A grant's computer no longer on the account is not one:
   * the server has taken it off, and nothing can reach it.
   */
  computers(): Promise<ComputerList> {
    const observed = this.observed();
    const profile = this.#profiles?.active?.id;
    return this.#withAccess(async (session, access) => {
      const [{ grants }, { hosts }] = await Promise.all([
        session.client.grants(access),
        session.client.hosts(access),
      ]);
      const onAccount = new Map(hosts.map((host) => [host.id, host]));
      const computers: Computer[] = [];
      for (const grant of grants) {
        const host = onAccount.get(grant.host);
        if (grant.state !== "approved" || !host) continue;
        computers.push({
          grant: grant.id,
          host: grant.host,
          name: host.name,
          lastSeenAt: host.lastSeenAt,
          scopes: grant.scopes,
          projects: grant.projects,
        });
      }
      const dropped = (await observed?.keepComputers(computers)) ?? [];
      if (profile) this.#noteLost(profile, dropped, onAccount);
      return {
        computers,
        waiting: grants.filter((grant) => grant.state === "pending"),
        onAccount: new Set(onAccount.keys()),
      };
    });
  }

  /** Say so of computers this phone knew and the server no longer lists, unless it did that itself. */
  #noteLost(
    profile: string,
    dropped: readonly { host: HostId; name: string }[],
    onAccount: ReadonlyMap<HostId, unknown>,
  ): void {
    const lost = dropped
      .filter((computer) => !this.#unpairing.delete(computer.host))
      .map(({ host, name }): Lost => ({
        host,
        name,
        why: onAccount.has(host) ? "tookBack" : "removed",
      }));
    if (lost.length === 0) return;
    this.#lost.set(profile, [...(this.#lost.get(profile) ?? []), ...lost]);
    this.#changed();
  }

  /**
   * What became of a computer that refused this phone. The relay's close does not say, so the
   * server is asked, with access refreshed if it has lapsed. Reading the list deletes what was kept
   * of a computer that parted; being refused deletes the profile; anything else deletes nothing.
   */
  async explain(host: HostId): Promise<Refusal> {
    try {
      const { computers, onAccount } = await this.computers();
      if (computers.some((computer) => computer.host === host)) return "undetermined";
      return onAccount.has(host) ? "tookBack" : "removed";
    } catch (error) {
      return refused(error) ? "signedOut" : "undetermined";
    }
  }

  /** Redeem a computer's offer, and ask it to let this phone in. An offer goes once. */
  pair(offer: OfferKey): Promise<GrantSummary> {
    return this.#withAccess((session, access) =>
      session.client.pair(access, { offer, scopes: ASKED_SCOPES }),
    );
  }

  /** Where a request or a pairing stands now. */
  grant(id: GrantId): Promise<GrantSummary> {
    return this.#withAccess((session, access) => session.client.grant(access, id));
  }

  /**
   * Take back a pairing, or a request still waiting. This phone stays signed in. `host` is the
   * computer of a pairing: it closes the channel to it before this has its answer.
   */
  async unpair(id: GrantId, host?: HostId): Promise<void> {
    if (host) this.#unpairing.add(host);
    try {
      await this.#withAccess((session, access) => session.client.revokeGrant(access, id));
    } catch (error) {
      if (host) this.#unpairing.delete(host);
      throw error;
    }
  }

  /** A channel to `host` through the relay, under the grant the server has in force. */
  openChannel(host: HostId): Promise<Channel> {
    return this.#withAccess((session, access) =>
      openChannel(session.client, access, host, {
        client: this.env.userAgent,
        ...(this.env.socket ? { socket: this.env.socket } : {}),
      }),
    );
  }

  /**
   * Sign out of a profile, the active one unless another is named: revoke this phone at that
   * server, which takes back every computer's grant, then delete the profile here. When the server
   * cannot be reached this throws, and nothing is deleted: see {@link forgetHere}.
   */
  async signOut(id?: string): Promise<void> {
    const profile = id ? this.#need().all.find((p) => p.id === id) : this.#session().profile;
    if (!profile) return;
    try {
      await this.#withAccessOf(profile, this.#sessionOf(profile), (session, access) =>
        session.client.revokeDevice(access, session.device),
      );
    } catch (error) {
      // Signed out already: #withAccess has forgotten the profile.
      if (refused(error)) return;
      throw error;
    }
    await this.#forget(profile.id);
    this.#changed();
  }

  /** A profile's session, opened for this when it is not the one in use. Nothing shows it. */
  async #sessionOf(profile: Profile): Promise<Session> {
    await this.#opening.get(profile.id);
    const open = this.#sessions.get(profile.id);
    if (open) return open;
    const profiles = this.#need();
    const refresh = await profiles.refresh(profile.id);
    if (!refresh) {
      throw ClientError.from({
        code: "unauthenticated",
        message: "This phone holds no sign-in there.",
        retryable: false,
      });
    }
    const client = this.clientFor(profile.address);
    let session: Session;
    try {
      session = await Session.resume(
        client,
        refresh,
        profiles.keeper(profile.id),
        this.#options(profile, client),
      );
    } catch (error) {
      if (!(error instanceof NotKept)) throw error;
      session = error.session;
    }
    this.#sessions.set(profile.id, session);
    return session;
  }

  /**
   * Delete a profile from this phone only. The server still counts this phone as signed in until
   * it is revoked from another of the account's devices.
   */
  async forgetHere(id: string): Promise<void> {
    await this.#forget(id);
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
