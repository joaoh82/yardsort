// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Whether each computer can be reached right now, which only a channel to it can say (31 §5). The
 * server has no presence. A computer is online while a channel to it is open, offline when the
 * relay says so, and reconnecting in between. Offline never means its agents stopped.
 *
 * Channels are held only while someone is looking: the screen stops this when it is left or the
 * app goes to the background, and starts it again on the way back.
 */

import {
  type Channel,
  ChannelClosed,
  type ChannelEnd,
  ClientError,
  type HostId,
} from "@yardsort/remote-client";

export type Reach =
  | { kind: "connecting" }
  | { kind: "online" }
  | { kind: "reconnecting" }
  | { kind: "offline" }
  /** The server or the computer no longer lets this phone in. The list should be asked again. */
  | { kind: "gone" };

export interface PresenceOptions {
  open(host: HostId): Promise<Channel>;
  /** How often to ask again after the relay said a computer is offline. */
  offlineRetryMs?: number;
  /** The first wait after losing a channel; it doubles up to {@link maxRetryMs}. */
  firstRetryMs?: number;
  maxRetryMs?: number;
}

interface Watch {
  live: boolean;
  channel: Channel | null;
  wake: (() => void) | null;
}

type Outcome = "offline" | "dropped" | "gone" | "signedOut";

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
      const watch: Watch = { live: true, channel: null, wake: null };
      this.#watches.set(host, watch);
      void this.#run(host, watch);
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
      } catch (error) {
        if (!watch.live) return;
        outcome = fromError(error);
      }
      if (outcome === "signedOut") {
        // The profile is deleted by now, and the screen holding this goes with it.
        return;
      }
      if (outcome === "gone") {
        this.#gone.add(host);
        this.#watches.delete(host);
        this.#set(host, { kind: "gone" });
        return;
      }
      if (outcome === "offline") {
        this.#set(host, { kind: "offline" });
        await this.#sleep(watch, this.options.offlineRetryMs ?? 15_000);
      } else {
        this.#set(host, { kind: "reconnecting" });
        await this.#sleep(watch, waitMs);
        waitMs = Math.min(waitMs * 2, most);
      }
    }
  }

  #sleep(watch: Watch, ms: number): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(resolve, ms);
      watch.wake = () => {
        clearTimeout(timer);
        resolve();
      };
    });
  }

  #set(host: HostId, reach: Reach): void {
    this.#reach = new Map(this.#reach).set(host, reach);
    this.#changed();
  }

  #changed(): void {
    for (const listener of this.#listeners) listener();
  }
}

function fromEnd(end: ChannelEnd): Outcome {
  if (end.reason === "revoked" || end.reason === "refused") return "gone";
  return end.reason === "hostOffline" ? "offline" : "dropped";
}

function fromError(error: unknown): Outcome {
  if (error instanceof ChannelClosed) return fromEnd(error.end);
  if (!(error instanceof ClientError)) return "dropped";
  switch (error.code) {
    case "host_offline":
      return "offline";
    case "forbidden":
    case "not_found":
      return "gone";
    case "unauthenticated":
      return "signedOut";
    default:
      return "dropped";
  }
}
