// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What the phone can tell someone helping with a problem (35 §19): its versions, which server and
 * which device it is there, and how the sign-in, each computer and the network stood, with times
 * and error codes. The person reads all of it before sending it anywhere, on **Settings →
 * Diagnostics**.
 *
 * Nothing in it is anyone's content. Each happening is one of a few shapes holding ids, states and
 * codes, and never a name, a title, a path, a message from a server, or anything a terminal showed
 * or was typed into: a code that does not look like one is written as `other`. It is kept in
 * memory, the last {@link KEPT} of them, and goes with the app.
 */

import type { DeviceId, HostId, ServerId } from "@yardsort/remote-client";

import type { Reach } from "./presence";

export type Happening =
  /** Where the active sign-in stands. */
  | {
      what: "signIn";
      standing: "opening" | "signedIn" | "unreachable" | "otherServer" | "signedOut";
      code?: string | undefined;
    }
  /** Where a computer stands, and what the last channel to it ended with. */
  | { what: "reach"; host: HostId; reach: Reach["kind"]; code?: string | undefined }
  /** The phone's network changed: its kind, and whether it is connected. */
  | { what: "network"; network: string };

export interface Entry {
  at: number;
  happening: Happening;
}

/** How many happenings are kept, the oldest dropped first. */
export const KEPT = 200;

/** A close code, a reason or a server's error code, and nothing longer or freer. */
const CODE = /^[A-Za-z0-9_.:-]{1,48}$/;

export function asCode(text: string | number | null | undefined): string | undefined {
  if (text === null || text === undefined) return undefined;
  const code = String(text);
  return CODE.test(code) ? code : "other";
}

export class Diagnostics {
  #entries: readonly Entry[] = [];
  #listeners = new Set<() => void>();

  constructor(readonly clock: () => number = Date.now) {}

  record(happening: Happening): void {
    const kept: Happening = { ...happening };
    if ("code" in kept) {
      const code = asCode(kept.code);
      if (code === undefined) delete kept.code;
      else kept.code = code;
    }
    if (kept.what === "network") kept.network = asCode(kept.network) ?? "other";
    const entries = [...this.#entries, { at: this.clock(), happening: kept }];
    this.#entries = entries.length > KEPT ? entries.slice(entries.length - KEPT) : entries;
    for (const listener of this.#listeners) listener();
  }

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** The same array until something happens, as `useSyncExternalStore` needs. */
  getSnapshot = (): readonly Entry[] => this.#entries;
}

/** The phone's own, recorded since it started. */
export const diagnostics = new Diagnostics();

/** What is true now, beside what happened. */
export interface Facts {
  /** `0.1.5`. */
  app: string;
  /** `ios 18.1`. */
  system: string;
  /** The protocol this phone speaks, `1.7`. */
  protocol: string;
  server: ServerId | null;
  device: DeviceId | null;
}

/**
 * The whole text, as it is shown and as it is shared: what is read is what goes. `asOf` is when it
 * was read: the screen's opening, or the last happening after that.
 */
export function report(facts: Facts, entries: readonly Entry[], asOf: number): string {
  const lines = [
    "Yardsort phone diagnostics",
    `As of ${stamp(asOf)}`,
    "",
    `App ${facts.app}`,
    `System ${facts.system}`,
    `Protocol ${facts.protocol}`,
    `Server ${facts.server ?? "none"}`,
    `Device ${facts.device ?? "none"}`,
    "",
    entries.length === 0 ? "Nothing has happened since the app started." : "Since the app started:",
    ...entries.map((entry) => `${stamp(entry.at)}  ${said(entry.happening)}`),
  ];
  return lines.join("\n");
}

function said(happening: Happening): string {
  const code = "code" in happening && happening.code ? ` (${happening.code})` : "";
  switch (happening.what) {
    case "signIn":
      return `sign-in ${happening.standing}${code}`;
    case "reach":
      return `computer ${happening.host} ${happening.reach}${code}`;
    case "network":
      return `network ${happening.network}`;
  }
}

/** UTC to the millisecond, so two phones' and a server's logs can be laid side by side. */
function stamp(at: number): string {
  return new Date(at).toISOString();
}
