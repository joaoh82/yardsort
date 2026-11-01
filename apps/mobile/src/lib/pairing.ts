// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What a pairing needs besides the server: a link held until it can be used, where that link
 * belongs, and waiting for the person at the computer (31 §5).
 *
 * A link carries the offer's secret, so it stays out of the router and off the disk: it is held
 * here, in memory, and dropped once used. A link lost to a restart costs a new scan.
 */

import {
  Client,
  ClientError,
  type GrantId,
  type GrantSummary,
  OFFER_TTL_MS,
  type PairingLink,
} from "@yardsort/remote-client";

import type { Profile } from "./profiles";

/** A link on its way to being used. */
export interface HeldLink {
  link: PairingLink;
  /** Whether the person has chosen this: they scanned it, or said yes to it. */
  agreed: boolean;
}

/** The one link waiting, as `useSyncExternalStore` reads it. */
export class Incoming {
  #held: HeldLink | null = null;
  /** The link already being sent. An offer goes once, so it is handed out once. */
  #claimed: HeldLink["link"] | null = null;
  #listeners = new Set<() => void>();

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  get = (): HeldLink | null => this.#held;

  /** A newer link replaces one still waiting: the computer's newest offer replaces its older. */
  hold(link: PairingLink, agreed: boolean): void {
    this.#held = { link, agreed };
    this.#changed();
  }

  agree(): void {
    if (!this.#held) return;
    this.#held = { ...this.#held, agreed: true };
    this.#changed();
  }

  /** The link, to send, for the first to ask. Whoever gets it drops it when the answer comes. */
  claim(): HeldLink["link"] | null {
    const link = this.#held?.link ?? null;
    if (!link || this.#claimed === link) return null;
    this.#claimed = link;
    return link;
  }

  drop(): void {
    if (!this.#held) return;
    this.#held = null;
    this.#claimed = null;
    this.#changed();
  }

  #changed(): void {
    for (const listener of this.#listeners) listener();
  }
}

/** The app's one. A link can arrive before any screen exists. */
export const incoming = new Incoming();

/** Where a link belongs, among this phone's profiles. */
export type LinkPlace =
  /** The server in use. */
  | { kind: "here" }
  /** Another this phone is signed in to. Switching needs the person's word. */
  | { kind: "known"; profile: Profile }
  /** One this phone is not signed in to. Signing in there needs the person's word. */
  | { kind: "new"; address: string }
  /** An address the app would not talk to. */
  | { kind: "invalid" };

export function placeOf(
  link: PairingLink,
  active: Profile | null,
  profiles: readonly Profile[],
): LinkPlace {
  let address: string;
  try {
    address = new Client(link.server).address;
  } catch {
    return { kind: "invalid" };
  }
  if (active?.address === address) return { kind: "here" };
  const profile = profiles.find((p) => p.address === address);
  return profile ? { kind: "known", profile } : { kind: "new", address };
}

export interface Waiter {
  grant(id: GrantId): Promise<GrantSummary>;
}

export interface WaitOptions {
  /** Stops the waiting. It then resolves to `null`. */
  signal?: AbortSignal;
  everyMs?: number;
  /** Milliseconds since the epoch. */
  clock?: () => number;
}

/**
 * Ask every two seconds until the computer's person answers or the request runs out (31 §5). A
 * server out of reach for a moment is asked again; out of reach until the end of the wait throws.
 * A refusal of this phone's own credential comes straight through, as every call's does.
 */
export async function waitForAnswer(
  waiter: Waiter,
  asked: GrantSummary,
  { signal, everyMs = 2_000, clock = Date.now }: WaitOptions = {},
): Promise<GrantSummary | null> {
  const deadline = clock() + (asked.expiresInMs ?? OFFER_TTL_MS);
  for (;;) {
    await pause(everyMs, signal);
    if (signal?.aborted) return null;
    try {
      const grant = await waiter.grant(asked.id);
      if (signal?.aborted) return null;
      if (grant.state !== "pending") return grant;
    } catch (error) {
      const passing =
        error instanceof ClientError && (error.kind === "unreachable" || error.kind === "timedOut");
      if (!passing) throw error;
      if (clock() >= deadline) throw error;
    }
  }
}

function pause(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}
