// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Whether each computer can be reached right now, which only a channel to it can say (31 §5). The
 * server has no presence. A computer is online while a channel to it is open, offline when the
 * relay says so, and reconnecting in between. Offline never means its agents stopped.
 *
 * Channels are held only while someone is looking: the screen stops this when it is left or the
 * app goes to the background, and starts it again on the way back.
 *
 * **When the phone's network changes**, from Wi-Fi to cellular or back, a channel on the old one
 * can be dead without a word, and one waiting to try again would wait out its backoff on a network
 * that is no longer there. {@link Presence.networkChanged} checks each open channel at once and
 * tries the rest again now, from the first wait.
 */

import {
  type Channel,
  ChannelClosed,
  type ChannelEnd,
  ClientError,
  type HostId,
} from "@yardsort/remote-client";

import type { Parted, Refusal } from "./accounts";
import { asCode, type Diagnostics } from "./diagnostics";

export type Reach =
  | { kind: "connecting" }
  | { kind: "online" }
  | { kind: "reconnecting" }
  | { kind: "offline" }
  /** It refused this phone, and the server cannot say why. It is asked again, as after a drop. */
  | { kind: "unreachable" }
  /** The server says this computer no longer lets this phone in. */
  | { kind: "gone"; why: Parted };

export interface PresenceOptions {
  open(host: HostId): Promise<Channel>;
  /** What a computer refusing this phone came to: the refusal itself does not say (31 §5). */
  explain(host: HostId): Promise<Refusal>;
  /** How often to ask again after the relay said a computer is offline. */
  offlineRetryMs?: number;
  /** The first wait after losing a channel; it doubles up to {@link maxRetryMs}. */
  firstRetryMs?: number;
  maxRetryMs?: number;
  /** How long an open channel has to answer once the network changed. */
  networkCheckMs?: number;
  /** Where each change is recorded, with what the last channel ended with. */
  diagnostics?: Diagnostics;
}

interface Watch {
  live: boolean;
  channel: Channel | null;
  wake: (() => void) | null;
  /** The network changed while it waited: try again now, and from the first wait. */
  again: boolean;
}

type Outcome = "offline" | "dropped" | "refused" | "signedOut";

export class Presence {
  #reach: ReadonlyMap<HostId, Reach> = new Map();
  #watches = new Map<HostId, Watch>();
  /** Computers found gone: not asked again until the next start, or they would be asked forever. */
  #gone = new Set<HostId>();
  #listeners = new Set<() => void>();

  constructor(readonly options: PresenceOptions) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** The same object until something changes, as `useSyncExternalStore` needs. */
  getSnapshot = (): ReadonlyMap<HostId, Reach> => this.#reach;

  /** The channel held to `host` while it is online, for whoever is looking to read through. */
  channelOf(host: HostId): Channel | null {
    return this.#watches.get(host)?.channel ?? null;
  }

  /** Hold a channel to each of `hosts`, and to nothing else. */
  watch(hosts: readonly HostId[]): void {
    for (const [host, watch] of this.#watches) {
      if (!hosts.includes(host)) this.#end(host, watch);
    }
    for (const host of hosts) {
      if (this.#watches.has(host) || this.#gone.has(host)) continue;
      const watch: Watch = { live: true, channel: null, wake: null, again: false };
      this.#watches.set(host, watch);
      void this.#run(host, watch);
    }
  }

  /**
   * The phone's network changed. Each open channel is asked whether it still answers, and closed if
   * it does not within {@link PresenceOptions.networkCheckMs}; each computer waiting to be tried
   * again is tried now.
   */
  networkChanged(): void {
    for (const watch of this.#watches.values()) {
      if (watch.channel) {
        watch.channel.check(this.options.networkCheckMs ?? 5_000);
      } else {
        watch.again = true;
        watch.wake?.();
      }
    }
  }

  /** Close every channel. Starting again asks about every computer afresh. */
  stop(): void {
    this.watch([]);
    this.#gone.clear();
    if (this.#reach.size === 0) return;
    this.#reach = new Map();
    this.#changed();
  }

  #end(host: HostId, watch: Watch): void {
    watch.live = false;
    watch.channel?.close();
    watch.wake?.();
    this.#watches.delete(host);
    if (!this.#reach.has(host)) return;
    const reach = new Map(this.#reach);
    reach.delete(host);
    this.#reach = reach;
    this.#changed();
  }

  async #run(host: HostId, watch: Watch): Promise<void> {
    const first = this.options.firstRetryMs ?? 2_000;
    const most = this.options.maxRetryMs ?? 30_000;
    let waitMs = first;
    this.#set(host, { kind: "connecting" });
    while (watch.live) {
      let outcome: Outcome;
      let cause: string | undefined;
      try {
        const channel = await this.options.open(host);
        if (!watch.live) {
          channel.close();
          return;
        }
        watch.channel = channel;
        this.#set(host, { kind: "online" });
        waitMs = first;
        const end = await channel.ended;
        watch.channel = null;
        if (!watch.live) return;
        outcome = fromEnd(end);
        cause = causeOfEnd(end);
      } catch (error) {
        if (!watch.live) return;
        outcome = fromError(error);
        cause = causeOfError(error);
      }
      if (outcome === "signedOut") {
        // The profile is deleted by now, and the screen holding this goes with it.
        this.options.diagnostics?.record({ what: "reach", host, reach: "gone", code: cause });
        return;
      }
      if (outcome === "refused") {
        this.#set(host, { kind: "reconnecting" }, cause);
        const refusal = await this.options.explain(host);
        if (!watch.live || refusal === "signedOut") return;
        if (refusal !== "undetermined") {
          this.#gone.add(host);
          this.#watches.delete(host);
          this.#set(host, { kind: "gone", why: refusal }, cause);
          return;
        }
        this.#set(host, { kind: "unreachable" }, cause);
        await this.#sleep(watch, waitMs);
        waitMs = Math.min(waitMs * 2, most);
      } else if (outcome === "offline") {
        this.#set(host, { kind: "offline" }, cause);
        await this.#sleep(watch, this.options.offlineRetryMs ?? 15_000);
      } else {
        this.#set(host, { kind: "reconnecting" }, cause);
        await this.#sleep(watch, waitMs);
        waitMs = Math.min(waitMs * 2, most);
      }
      if (watch.again) {
        watch.again = false;
        waitMs = first;
      }
    }
  }

  #sleep(watch: Watch, ms: number): Promise<void> {
    // The network changed while it was trying: no need to wait.
    if (watch.again) return Promise.resolve();
    return new Promise((resolve) => {
      const timer = setTimeout(resolve, ms);
      watch.wake = () => {
        clearTimeout(timer);
        resolve();
      };
    });
  }

  #set(host: HostId, reach: Reach, cause?: string): void {
    this.options.diagnostics?.record({ what: "reach", host, reach: reach.kind, code: cause });
    this.#reach = new Map(this.#reach).set(host, reach);
    this.#changed();
  }

  #changed(): void {
    for (const listener of this.#listeners) listener();
  }
}

function fromEnd(end: ChannelEnd): Outcome {
  if (end.reason === "revoked" || end.reason === "refused") return "refused";
  return end.reason === "hostOffline" ? "offline" : "dropped";
}

/** How a channel ended, as a code: never its words, which a server chose. */
function causeOfEnd(end: ChannelEnd): string | undefined {
  return asCode(end.code === null ? end.reason : `${end.reason}:${end.code}`);
}

/** What opening a channel failed with, as a code. */
function causeOfError(error: unknown): string | undefined {
  if (error instanceof ChannelClosed) return causeOfEnd(error.end);
  if (error instanceof ClientError)
    return asCode(error.code ? `${error.kind}:${error.code}` : error.kind);
  return "error";
}

function fromError(error: unknown): Outcome {
  if (error instanceof ChannelClosed) return fromEnd(error.end);
  if (!(error instanceof ClientError)) return "dropped";
  switch (error.code) {
    case "host_offline":
      return "offline";
    case "forbidden":
    case "not_found":
      return "refused";
    case "unauthenticated":
      return "signedOut";
    default:
      return "dropped";
  }
}
