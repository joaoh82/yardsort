// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The relay's own messages (`remote_proto::relay`): tickets, the socket's first message and its
 * answer, the notices a host is sent, and the codes a socket is closed with. What a device says to
 * a host on the socket after that is the remote protocol itself (`protocol.ts`).
 */

import {
  DecodeError,
  type Decoder,
  isObject,
  object,
  optional,
  tagged,
  text,
  u64,
  variant,
} from "./decode.ts";
import { MAX_ROUTE_LEN } from "./limits.ts";
import { type GrantId, type HostId, id, type RemoteError, remoteError } from "./protocol.ts";
import { type Secret, secret } from "./secret.ts";

/** `POST /v1/relay/tickets`: a host's own link, or a device's way to a host. */
export type TicketRequest = { role: "host"; host: HostId } | { role: "device"; host: HostId };

export const ticketRequest: Decoder<TicketRequest> = tagged<TicketRequest>("role", {
  host: variant("role", "host", { host: id<HostId>() }),
  device: variant("role", "device", { host: id<HostId>() }),
});

/** A ticket, good once within a minute. */
export interface RelayTicket {
  ticket: Secret;
  expiresInMs: number;
  /** Which instance of the server holds it; the socket's address names it. */
  route?: string;
}

/** The relay socket's query parameter naming a ticket's route. */
export const ROUTE_PARAM = "route";

const ROUTE = /^[A-Za-z0-9_-]+$/;

/** Whether `route` could name an instance of the server. */
export function validRoute(route: string): boolean {
  return route.length > 0 && route.length <= MAX_ROUTE_LEN && ROUTE.test(route);
}

export const relayTicket: Decoder<RelayTicket> = object({
  ticket: secret,
  expiresInMs: u64,
  route: optional((value, path) => {
    const route = text()(value, path);
    if (!validRoute(route)) throw new DecodeError(path, "is not a route");
    return route;
  }),
});

/** The first message on a relay socket. */
export interface Presented {
  ticket: Secret;
}

export const presented: Decoder<Presented> = object({ ticket: secret });

/** The relay's answer to a presented ticket. After `refused`, it closes the socket. */
export type Greeting = { connected: { host: HostId } } | { refused: RemoteError };

export const greeting: Decoder<Greeting> = (value, path) => {
  if (isObject(value) && "connected" in value) {
    return { connected: object({ host: id<HostId>() })(value.connected, "connected") };
  }
  if (isObject(value) && "refused" in value) {
    return { refused: remoteError(value.refused, "refused") };
  }
  throw new DecodeError(path, "is neither connected nor refused");
};

/** What the relay tells a host on its link's own channel: a hint to ask the server again. */
export type HostNotice = { kind: "pairingRequested"; grant: GrantId } | { kind: "unknown" };

export const hostNotice: Decoder<HostNotice> = tagged<HostNotice>(
  "kind",
  { pairingRequested: variant("kind", "pairingRequested", { grant: id<GrantId>() }) },
  () => ({ kind: "unknown" }),
);

/**
 * Why the relay closed a socket, from its WebSocket close code. `revoked` (4003) does not say
 * what was revoked: this phone's grant on that computer, the computer, or the phone itself. Ask
 * the server before acting on it ({@link explainRevoked}).
 */
export const CLOSE_REASONS = {
  gone: 4000,
  unauthenticated: 4001,
  revoked: 4003,
  hostOffline: 4004,
  overloaded: 4008,
  replaced: 4009,
  refused: 4010,
  invalid: 4400,
  restarting: 4503,
  unknown: 4999,
} as const;
export type CloseReason = keyof typeof CLOSE_REASONS;

export function closeReason(code: number): CloseReason {
  for (const [reason, known] of Object.entries(CLOSE_REASONS)) {
    if (known === code) return reason as CloseReason;
  }
  return "unknown";
}

/** A few words for a person: no ids, nothing of what was carried. */
export function closeText(reason: CloseReason): string {
  switch (reason) {
    case "gone":
      return "the other end left";
    case "unauthenticated":
      return "no valid ticket";
    case "revoked":
      return "access was revoked";
    case "hostOffline":
      return "the host is offline";
    case "overloaded":
      return "fell too far behind";
    case "replaced":
      return "the host connected again";
    case "refused":
      return "the host did not let this device in";
    case "invalid":
      return "the socket broke its rules";
    case "restarting":
      return "the server is restarting";
    case "unknown":
      return "closed";
  }
}
