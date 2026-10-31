// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What the server's HTTP API carries: signing in and the server's own credentials
 * (`remote_proto::account`), and pairing and grants (`remote_proto::pairing`). A device never sees a token the identity provider issued: it signs in
 * through the server, and holds the server's credentials.
 */

import {
  array,
  bool,
  chars,
  type Decoder,
  hasControl,
  i64,
  nullable,
  object,
  oneOf,
  tagged,
  text,
  u64,
  utf8Length,
  variant,
} from "./decode.ts";
import {
  CODE_LEN,
  MAX_GRANT_PROJECTS,
  MAX_REDIRECT_LEN,
  MAX_SECRET_LEN,
  MAX_TEXT,
  MAX_VERIFIER_LEN,
  MIN_VERIFIER_LEN,
} from "./limits.ts";
import {
  type DeviceId,
  type GrantId,
  type HostId,
  id,
  type Platform,
  platform,
  type ProjectGrant,
  projectGrant,
  type ProtocolVersion,
  protocolVersion,
  type Scope,
  scopes,
  type ServerId,
} from "./protocol.ts";
import { Secret, secret } from "./secret.ts";

// --- signing in --------------------------------------------------------------------------------

/** The only challenge method accepted: the SHA-256 of the verifier. */
export const CHALLENGE_METHOD = "S256";

/** `GET /v1/server`: who the server is and how to sign in to it. Public. */
export interface ServerDescription {
  server: ServerId;
  name: string;
  protocol: ProtocolVersion;
  /** The identity provider people sign in with, as its issuer URL. */
  issuer: string;
  capabilities: string[];
}

export const serverDescription: Decoder<ServerDescription> = object({
  server: id<ServerId>(),
  name: text(),
  protocol: protocolVersion,
  issuer: text(),
  capabilities: array(text()),
});

export const DEVICE_KINDS = ["phone", "computer", "host", "other"] as const;
export type DeviceKind = (typeof DEVICE_KINDS)[number];

/** The device being enrolled, as it describes itself. */
export interface NewDevice {
  /** What its person will recognise it by: "Ana's phone". */
  name: string;
  kind: DeviceKind;
  platform: Platform;
}

const newDevice: Decoder<NewDevice> = object({
  name: text(),
  kind: oneOf(DEVICE_KINDS, "other"),
  platform,
});

/** `POST /v1/token`. */
export type TokenRequest =
  | { grant: "signIn"; code: Secret; verifier: Secret; device: NewDevice }
  | { grant: "refresh"; refresh: Secret };

export const tokenRequest: Decoder<TokenRequest> = tagged<TokenRequest>("grant", {
  signIn: variant("grant", "signIn", { code: secret, verifier: secret, device: newDevice }),
  refresh: variant("grant", "refresh", { refresh: secret }),
});

const UNRESERVED = /^[A-Za-z0-9._~-]+$/;

/** Why the server would refuse a token request, or `null`: the checks it makes. */
export function validateTokenRequest(request: TokenRequest): string | null {
  if (request.grant === "refresh") {
    const refresh = request.refresh.expose();
    return refresh.length === 0 || refresh.length > MAX_SECRET_LEN
      ? "The refresh credential is missing or too long."
      : null;
  }
  const code = request.code.expose();
  if (code.length === 0 || code.length > MAX_SECRET_LEN) {
    return "The sign-in code is missing or too long.";
  }
  const verifier = request.verifier.expose();
  if (
    verifier.length < MIN_VERIFIER_LEN ||
    verifier.length > MAX_VERIFIER_LEN ||
    !UNRESERVED.test(verifier)
  ) {
    return "The verifier is not a PKCE verifier.";
  }
  return checkName(request.device.name, "A device name");
}

/** A name a person gives: one line, of at most {@link MAX_TEXT} characters once trimmed. */
function checkName(name: string, what: string): string | null {
  const trimmed = name.trim();
  return trimmed.length === 0 || chars(trimmed) > MAX_TEXT || hasControl(trimmed)
    ? `${what} is one line of at most 256 characters.`
    : null;
}

/** What a device holds after signing in or refreshing. */
export interface Credentials {
  device: DeviceId;
  /** Sent as `Authorization: Bearer …` with each request. */
  access: Secret;
  /** How long `access` works for, from when it was issued: a duration, so a wrong clock does not matter. */
  accessExpiresInMs: number;
  /** Kept in the device's credential store, and nowhere else. */
  refresh: Secret;
}

export const credentials: Decoder<Credentials> = object({
  device: id<DeviceId>(),
  access: secret,
  accessExpiresInMs: u64,
  refresh: secret,
});

export interface DeviceSummary {
  id: DeviceId;
  name: string;
  kind: DeviceKind;
  platform: Platform;
  createdAt: number;
  /** When it last used its access, as far as the server noticed. History, not presence. */
  lastSeenAt: number | null;
  /** A revoked device is listed until it is forgotten. */
  revokedAt: number | null;
  /** The device asking. */
  current: boolean;
}

/** `GET /v1/devices`: the account's devices, newest first. */
export interface DevicePage {
  devices: DeviceSummary[];
}

export const devicePage: Decoder<DevicePage> = object({
  devices: array(
    object({
      id: id<DeviceId>(),
      name: text(MAX_TEXT),
      kind: oneOf(DEVICE_KINDS, "other"),
      platform,
      createdAt: i64,
      lastSeenAt: nullable(i64),
      revokedAt: nullable(i64),
      current: bool,
    }),
  ),
});

// --- pairing -----------------------------------------------------------------------------------

/** `POST /v1/hosts`: a host signing in registers. The client reads it only to test the wire. */
export interface HostRegistration {
  host: HostId | null;
  name: string;
}

export const hostRegistration: Decoder<HostRegistration> = object({
  host: nullable(id<HostId>()),
  name: text(),
});

export function validateHostRegistration(registration: HostRegistration): string | null {
  return checkName(registration.name, "A host name");
}

/** A computer on the account. */
export interface HostSummary {
  id: HostId;
  name: string;
  createdAt: number;
  /** History, not presence: a computer is online while a channel to it is open. */
  lastSeenAt: number | null;
  /** The device asking is this host. */
  current: boolean;
}

/** `GET /v1/hosts`: the account's computers, newest first. Removed ones are not listed. */
export interface HostPage {
  hosts: HostSummary[];
}

export const hostPage: Decoder<HostPage> = object({
  hosts: array(
    object({
      id: id<HostId>(),
      name: text(MAX_TEXT),
      createdAt: i64,
      lastSeenAt: nullable(i64),
      current: bool,
    }),
  ),
});

/** What a computer shows to let one device in: as a QR code, and as a code to type. */
export interface Offer {
  secret: Secret;
  code: Secret;
  expiresInMs: number;
}

export const offer: Decoder<Offer> = object({ secret, code: secret, expiresInMs: u64 });

/** Which half of an offer a device presents: the QR code's secret, or the typed code. */
export type OfferKey = { kind: "secret"; value: Secret } | { kind: "code"; value: Secret };

/** `POST /v1/pairings`: redeem an offer and ask for access. */
export interface PairingRequest {
  offer: OfferKey;
  /** What the device asks for. The computer's person may give less, never more. */
  scopes: Scope[];
}

export const pairingRequest: Decoder<PairingRequest> = object({
  offer: tagged<OfferKey>("kind", {
    secret: variant("kind", "secret", { value: secret }),
    code: variant("kind", "code", { value: secret }),
  }),
  scopes,
});

export function validatePairingRequest(request: PairingRequest): string | null {
  if (request.offer.kind === "code" && normalizeCode(request.offer.value.expose()) === null) {
    return "A pairing code is eight letters and digits, like ABCD-EFGH.";
  }
  return request.scopes.length === 0 ? "A pairing asks for at least one scope." : null;
}

/** `POST /v1/grants/{grant}/approve`, as the computer's person chose. */
export interface GrantDecision {
  scopes: Scope[];
  projects: ProjectGrant;
}

export const grantDecision: Decoder<GrantDecision> = object({ scopes, projects: projectGrant });

export function validateGrantDecision(decision: GrantDecision, requested: Scope[]): string | null {
  if (decision.scopes.length === 0) return "An approval gives at least one scope.";
  if (!decision.scopes.every((s) => requested.includes(s))) {
    return "An approval cannot give more than the device asked for.";
  }
  if (
    decision.projects.kind === "only" &&
    (decision.projects.projects.length === 0 ||
      decision.projects.projects.length > MAX_GRANT_PROJECTS)
  ) {
    return "An approval names between one and 200 projects, or all of them.";
  }
  return null;
}

export const GRANT_STATES = [
  "pending",
  "approved",
  "denied",
  "expired",
  "revoked",
  "unknown",
] as const;
/** Where a grant stands. One from a newer server reads as `unknown`. */
export type GrantState = (typeof GRANT_STATES)[number];

/** A device's grant on a computer, or its request for one. */
export interface GrantSummary {
  id: GrantId;
  host: HostId;
  hostName: string;
  device: DeviceId;
  deviceName: string;
  state: GrantState;
  requested: Scope[];
  /** What it was given: empty unless approved. */
  scopes: Scope[];
  /** Which projects: `null` unless approved. */
  projects: ProjectGrant | null;
  createdAt: number;
  decidedAt: number | null;
  /** While pending: how long it still waits. */
  expiresInMs: number | null;
}

export const grantSummary: Decoder<GrantSummary> = object({
  id: id<GrantId>(),
  host: id<HostId>(),
  hostName: text(MAX_TEXT),
  device: id<DeviceId>(),
  deviceName: text(MAX_TEXT),
  state: oneOf(GRANT_STATES, "unknown"),
  requested: scopes,
  scopes,
  projects: nullable(projectGrant),
  createdAt: i64,
  decidedAt: nullable(i64),
  expiresInMs: nullable(u64),
});

export interface GrantPage {
  grants: GrantSummary[];
}

export const grantPage: Decoder<GrantPage> = object({ grants: array(grantSummary) });

/** Crockford's base32: no I, L, O or U, so nothing reads as something else. */
const CODE_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/**
 * A pairing code as typed, made canonical: dashes and spaces dropped, case ignored, and the letters
 * people mistake for digits read as those digits. `null` when it is not one.
 */
export function normalizeCode(typed: string): string | null {
  let code = "";
  for (const typedChar of typed) {
    let c = typedChar.toUpperCase();
    if (c === "-" || c === " ") continue;
    if (c === "O") c = "0";
    else if (c === "I" || c === "L") c = "1";
    if (c.length !== 1 || !CODE_ALPHABET.includes(c) || code.length === CODE_LEN) return null;
    code += c;
  }
  return code.length === CODE_LEN ? code : null;
}

/** How a pairing code is shown: `ABCD-EFGH`. */
export function displayCode(code: string): string {
  const canonical = normalizeCode(code);
  return canonical === null ? code : `${canonical.slice(0, 4)}-${canonical.slice(4)}`;
}

/** What a computer's QR code says: `yardsort://pair?server=…&secret=…`. */
export interface PairingLink {
  /** The server the computer is signed in to, as its address. */
  server: string;
  secret: Secret;
}

/** Read a pairing link, or `null` when it is not one. The server's address is not checked here. */
export function parsePairingLink(link: string): PairingLink | null {
  const match = /^yardsort:\/\/pair\?([^#]*)$/.exec(link.trim());
  if (!match) return null;
  const fields = new Map<string, string>();
  for (const pair of (match[1] as string).split("&")) {
    const at = pair.indexOf("=");
    if (at < 0) continue;
    try {
      const name = decodeURIComponent(pair.slice(0, at).replace(/\+/g, " "));
      const value = decodeURIComponent(pair.slice(at + 1).replace(/\+/g, " "));
      if (!fields.has(name)) fields.set(name, value);
    } catch {
      return null;
    }
  }
  const server = fields.get("server");
  const key = fields.get("secret");
  if (!server || !key || key.length > MAX_SECRET_LEN) return null;
  return { server, secret: new Secret(key) };
}

/** A redirect a device names: not empty, at most {@link MAX_REDIRECT_LEN} bytes. */
export function validRedirect(redirect: string): boolean {
  return redirect.length > 0 && utf8Length(redirect) <= MAX_REDIRECT_LEN;
}
