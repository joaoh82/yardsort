// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A signed-in device's credentials, refreshed when its access is about to lapse: the twin of
 * `remote_client::Session`.
 *
 * Each refresh credential works once. The server's answer carries the next one, and presenting a
 * spent one signs the device out unless it is the same retry within 30 seconds (27 §21). So:
 *
 * - **One refresh at a time.** Callers that need access while a refresh is under way wait for its
 *   answer instead of sending the same credential again.
 * - **The new credential is kept before anything uses the access** that came with it. `keep`
 *   saves it (the app's credential store); access is handed out only once `keep` has finished.
 *   When `keep` fails, the session still holds the new credential, so this process can carry on,
 *   and the failure is reported as {@link NotKept}, which carries the session.
 */

import type { Credentials } from "./account.ts";
import { type Client, ClientError } from "./client.ts";
import type { DeviceId } from "./protocol.ts";
import type { Secret } from "./secret.ts";

/** An access credential is refreshed this long before it would lapse. */
export const REFRESH_MARGIN_MS = 60_000;

/**
 * The server issued a new refresh credential and `keep` could not save it. The one before is
 * spent, so {@link session} is the only holder of the new one: carry on with it, and keep its
 * {@link Session.refreshCredential} when that works again.
 */
export class NotKept extends ClientError {
  constructor(
    readonly session: Session,
    why: string,
  ) {
    super("notKept", `The new sign-in could not be kept: ${why}`);
    this.name = "NotKept";
  }
}

/** Saves a new refresh credential, in place of the one just spent. */
export type Keep = (refresh: Secret) => void | Promise<void>;

export interface SessionOptions {
  /** A monotonic clock, in milliseconds. */
  now?: () => number;
}

export class Session {
  readonly device: DeviceId;
  #access: Secret;
  #refresh: Secret;
  #refreshAt: number;
  #flight: Promise<Secret> | null = null;
  readonly #now: () => number;

  /** From credentials the server just issued, which `keep` has kept. */
  constructor(
    readonly client: Client,
    credentials: Credentials,
    readonly keep: Keep,
    options: SessionOptions = {},
  ) {
    this.#now = options.now ?? (() => performance.now());
    this.device = credentials.device;
    this.#access = credentials.access;
    this.#refresh = credentials.refresh;
    this.#refreshAt = this.#due(credentials.accessExpiresInMs);
  }

  /**
   * From a refresh credential kept from before, refreshed at once. The one given is spent by the
   * time this returns, and `keep` has been given its replacement; when `keep` fails, the
   * {@link NotKept} thrown carries the session.
   */
  static async resume(
    client: Client,
    refresh: Secret,
    keep: Keep,
    options: SessionOptions = {},
  ): Promise<Session> {
    const issued = await client.refresh(refresh);
    const session = new Session(client, issued, keep, options);
    await session.#kept(issued.refresh);
    return session;
  }

  /** The refresh credential in force. */
  get refreshCredential(): Secret {
    return this.#refresh;
  }

  /** An access credential that works for at least another minute, refreshing first when needed. */
  access(): Promise<Secret> {
    if (this.#flight) return this.#flight;
    if (this.#now() < this.#refreshAt) return Promise.resolve(this.#access);
    const flight = this.#refreshNow().finally(() => {
      this.#flight = null;
    });
    this.#flight = flight;
    return flight;
  }

  async #refreshNow(): Promise<Secret> {
    const issued = await this.client.refresh(this.#refresh);
    this.#access = issued.access;
    this.#refresh = issued.refresh;
    this.#refreshAt = this.#due(issued.accessExpiresInMs);
    await this.#kept(issued.refresh);
    return issued.access;
  }

  async #kept(refresh: Secret): Promise<void> {
    try {
      await this.keep(refresh);
    } catch (error) {
      throw new NotKept(this, error instanceof Error ? error.message : String(error));
    }
  }

  #due(expiresInMs: number): number {
    return this.#now() + Math.max(0, expiresInMs - REFRESH_MARGIN_MS);
  }

  /** Sign out: revoke this device at the server, which takes back every grant it holds. */
  async signOut(): Promise<void> {
    await this.client.revokeDevice(await this.access(), this.device);
  }
}
