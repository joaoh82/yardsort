// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One server, as a device speaks to it over HTTPS: the twin of `crates/remote-client`'s `Client`.
 *
 * Only HTTPS servers are spoken to, apart from one on a loopback address written as one
 * (`http://127.0.0.1:8787`), which is how a development server runs. Each request asks not to be
 * redirected, since following a redirect could carry a credential somewhere else. Bun and browsers
 * honour that; React Native's `fetch` may follow one regardless, which the API never sends.
 *
 * Addresses are read and written here rather than with `URL`, which React Native implements only
 * in part.
 */

import {
  type Credentials,
  credentials,
  type DevicePage,
  devicePage,
  type GrantPage,
  grantPage,
  type GrantSummary,
  grantSummary,
  type HostPage,
  hostPage,
  type NewDevice,
  type PairingRequest,
  type ServerDescription,
  serverDescription,
  type TokenRequest,
  validatePairingRequest,
  validateTokenRequest,
  validRedirect,
  CHALLENGE_METHOD,
} from "./account.ts";
import { challengeFor, type Crypto, newVerifier, webCrypto } from "./crypto.ts";
import { DecodeError, type Decoder, parse } from "./decode.ts";
import type { DeviceId, GrantId, HostId, RemoteError } from "./protocol.ts";
import { remoteError } from "./protocol.ts";
import { relayTicket, type RelayTicket, type TicketRequest } from "./relay.ts";
import { Secret } from "./secret.ts";

/** How long one request may take. */
const TIMEOUT_MS = 30_000;
/** The most a JSON answer may hold. The server's are far smaller. */
const MAX_ANSWER = 1024 * 1024;

export type ClientErrorKind =
  /** The server refused, with its reason: {@link ClientError.remote}. */
  | "remote"
  | "unreachable"
  /** The server answered something this client does not understand. */
  | "unexpected"
  | "invalid"
  | "timedOut"
  /**
   * The server issued a new refresh credential and it could not be kept. The session still holds
   * it, so it works until the app ends.
   */
  | "notKept";

export class ClientError extends Error {
  constructor(
    readonly kind: ClientErrorKind,
    message: string,
    readonly remote?: RemoteError,
  ) {
    super(message);
    this.name = "ClientError";
  }

  /** The server's reason, when it gave one. */
  get code(): RemoteError["code"] | undefined {
    return this.remote?.code;
  }

  static from(error: RemoteError): ClientError {
    return new ClientError("remote", error.message, error);
  }
}

export interface ClientOptions {
  fetch?: typeof fetch;
  crypto?: Crypto;
  /** Sent as `User-Agent` where the platform allows it. */
  userAgent?: string;
  timeoutMs?: number;
}

interface Address {
  scheme: "http" | "https";
  /** Host and port as written, lower case: `example.com`, `127.0.0.1:8787`, `[::1]:8787`. */
  authority: string;
  /** Ends in `/`. */
  path: string;
}

const ADDRESS =
  /^(https?):\/\/(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9.-]+)(?::(\d{1,5}))?(\/[A-Za-z0-9._~!$&'()*+,;=:@%/-]*)?$/;

/** A server's address, if it is one this client speaks to. */
export function readAddress(address: string): Address {
  const match = ADDRESS.exec(address.trim());
  if (!match) {
    throw new ClientError(
      "invalid",
      "A server's address is https:// and a host name, with no query, fragment or user name.",
    );
  }
  const scheme = (match[1] as string).toLowerCase() as Address["scheme"];
  const host = (match[2] as string).toLowerCase();
  const port = match[3];
  if (port !== undefined && Number(port) > 65535) {
    throw new ClientError("invalid", "That port is not one.");
  }
  if (scheme === "http" && !loopback(host)) {
    throw new ClientError(
      "invalid",
      "A server's address starts with https:// (plain http only on 127.0.0.1 or [::1]).",
    );
  }
  let path = match[4] ?? "/";
  if (!path.endsWith("/")) path += "/";
  return { scheme, authority: port === undefined ? host : `${host}:${port}`, path };
}

/** A loopback address written as one: 127.0.0.0/8, or `[::1]`. Never a name such as `localhost`. */
function loopback(host: string): boolean {
  const v4 = /^127\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (v4) return v4.slice(1).every((part) => Number(part) <= 255);
  return host === "[::1]" || host === "[0:0:0:0:0:0:0:1]";
}

/** `name=value&…`, each part encoded. */
export function query(pairs: Record<string, string>): string {
  return Object.entries(pairs)
    .map(([name, value]) => `${encodeURIComponent(name)}=${encodeURIComponent(value)}`)
    .join("&");
}

/** A sign-in under way: open {@link url} in the system browser, keep {@link verifier}. */
export interface SignIn {
  /** Where to send the browser. It holds no secret: the challenge is a hash. */
  url: string;
  /** Kept until the browser comes back, then given to {@link Client.signIn}. */
  verifier: Secret;
}

export class Client {
  readonly #fetch: typeof fetch;
  readonly #crypto: Crypto | null;
  readonly #address: Address;
  readonly #userAgent: string | undefined;
  readonly #timeoutMs: number;

  /** A client of the server at `address`: `https://remote.example.com`, or a loopback address. */
  constructor(address: string, options: ClientOptions = {}) {
    this.#address = readAddress(address);
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#crypto = options.crypto ?? webCrypto();
    this.#userAgent = options.userAgent;
    this.#timeoutMs = options.timeoutMs ?? TIMEOUT_MS;
  }

  /** The server's address, ending in `/`. */
  get address(): string {
    const { scheme, authority, path } = this.#address;
    return `${scheme}://${authority}${path}`;
  }

  /** The relay socket's address, `ws://` or `wss://`, for a ticket's route when it has one. */
  relayAddress(route?: string): string {
    const { scheme, authority, path } = this.#address;
    const base = `${scheme === "https" ? "wss" : "ws"}://${authority}${path}v1/relay`;
    return route === undefined ? base : `${base}?${query({ route })}`;
  }

  /** `GET /v1/server`. */
  describe(): Promise<ServerDescription> {
    return this.#send("GET", "v1/server", serverDescription);
  }

  /**
   * Start signing in: a verifier to keep, and the address that sends the browser to the server's
   * identity provider and back to `redirect` (`yardsort://signed-in` for the app, a loopback
   * address for a computer).
   */
  async beginSignIn(redirect: string): Promise<SignIn> {
    if (!validRedirect(redirect))
      throw new ClientError("invalid", "The redirect is missing or too long.");
    const crypto = this.#needCrypto();
    const verifier = newVerifier(crypto);
    const challenge = await challengeFor(crypto, verifier);
    const url = `${this.address}v1/login?${query({
      challenge,
      challengeMethod: CHALLENGE_METHOD,
      redirect,
    })}`;
    return { url, verifier: new Secret(verifier) };
  }

  /** `POST /v1/token`: redeem the code the browser brought back, enrolling this device. */
  signIn(code: Secret, verifier: Secret, device: NewDevice): Promise<Credentials> {
    return this.#token({ grant: "signIn", code, verifier, device });
  }

  /** `POST /v1/token` with a refresh credential. The answer's refresh credential replaces it. */
  refresh(refresh: Secret): Promise<Credentials> {
    return this.#token({ grant: "refresh", refresh });
  }

  /** `GET /v1/devices`. */
  devices(access: Secret): Promise<DevicePage> {
    return this.#send("GET", "v1/devices", devicePage, { access });
  }

  /** `POST /v1/devices/{device}/revoke`: this device signing out, or another of the account's. */
  revokeDevice(access: Secret, device: DeviceId): Promise<void> {
    return this.#send("POST", `v1/devices/${device}/revoke`, null, { access });
  }

  /** `GET /v1/hosts`: the account's computers. */
  hosts(access: Secret): Promise<HostPage> {
    return this.#send("GET", "v1/hosts", hostPage, { access });
  }

  /** `POST /v1/pairings`: redeem a computer's offer and ask to be let in. */
  pair(access: Secret, request: PairingRequest): Promise<GrantSummary> {
    const invalid = validatePairingRequest(request);
    if (invalid) return Promise.reject(new ClientError("invalid", invalid));
    return this.#send("POST", "v1/pairings", grantSummary, { access, body: request });
  }

  /** `GET /v1/grants/{grant}`. */
  grant(access: Secret, grant: GrantId): Promise<GrantSummary> {
    return this.#send("GET", `v1/grants/${grant}`, grantSummary, { access });
  }

  /** `GET /v1/grants`: this device's own. */
  grants(access: Secret): Promise<GrantPage> {
    return this.#send("GET", "v1/grants", grantPage, { access });
  }

  /** `POST /v1/grants/{grant}/revoke`: unpair. */
  revokeGrant(access: Secret, grant: GrantId): Promise<void> {
    return this.#send("POST", `v1/grants/${grant}/revoke`, null, { access });
  }

  /** `POST /v1/relay/tickets`: a ticket to reach `host`, under the grant the server has in force. */
  ticket(access: Secret, host: HostId): Promise<RelayTicket> {
    const body: TicketRequest = { role: "device", host };
    return this.#send("POST", "v1/relay/tickets", relayTicket, { access, body });
  }

  #token(request: TokenRequest): Promise<Credentials> {
    const invalid = validateTokenRequest(request);
    if (invalid) return Promise.reject(new ClientError("invalid", invalid));
    return this.#send("POST", "v1/token", credentials, { body: request });
  }

  #needCrypto(): Crypto {
    if (!this.#crypto) {
      throw new ClientError("invalid", "This platform has no random source; pass one as `crypto`.");
    }
    return this.#crypto;
  }

  async #send<T>(
    method: "GET" | "POST",
    path: string,
    decoder: Decoder<T> | null,
    { access, body }: { access?: Secret; body?: unknown } = {},
  ): Promise<T> {
    const headers: Record<string, string> = { accept: "application/json" };
    if (access) headers.authorization = `Bearer ${access.expose()}`;
    if (body !== undefined) headers["content-type"] = "application/json";
    if (this.#userAgent) headers["user-agent"] = this.#userAgent;
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), this.#timeoutMs);
    let answer: globalThis.Response;
    let text: string;
    try {
      answer = await this.#fetch(`${this.address}${path}`, {
        method,
        headers,
        redirect: "manual",
        signal: abort.signal,
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
      text = await answer.text();
    } catch (error) {
      if (abort.signal.aborted)
        throw new ClientError("timedOut", "The server did not answer in time.");
      throw new ClientError("unreachable", `Could not reach the server: ${describe(error)}`);
    } finally {
      clearTimeout(timer);
    }
    if (text.length > MAX_ANSWER)
      throw new ClientError("unexpected", "The server's answer is too large.");
    if (answer.status < 200 || answer.status > 299) {
      let refused: RemoteError;
      try {
        refused = parse(remoteError, text);
      } catch {
        throw new ClientError("unexpected", `The server answered HTTP ${answer.status}.`);
      }
      throw ClientError.from(refused);
    }
    if (decoder === null) return undefined as T;
    try {
      return parse(decoder, text);
    } catch (error) {
      if (error instanceof DecodeError) {
        throw new ClientError(
          "unexpected",
          `The server's answer was not understood: ${error.message}`,
        );
      }
      throw error;
    }
  }
}

/** Read the code a sign-in brought back to `redirect`, or why there is none. */
export function codeFromRedirect(returned: string): Secret {
  const at = returned.indexOf("?");
  const fields = new Map<string, string>();
  for (const pair of (at < 0 ? "" : (returned.slice(at + 1).split("#")[0] ?? "")).split("&")) {
    const eq = pair.indexOf("=");
    if (eq < 0) continue;
    try {
      fields.set(
        decodeURIComponent(pair.slice(0, eq).replace(/\+/g, " ")),
        decodeURIComponent(pair.slice(eq + 1).replace(/\+/g, " ")),
      );
    } catch {
      // Not a field that can be read; the others still can.
    }
  }
  const code = fields.get("code");
  if (code) return new Secret(code);
  // An error code from the browser is bounded: it came through an address anyone could open.
  const error = (fields.get("error") ?? "").replace(/[^A-Za-z0-9_]/g, "").slice(0, 32);
  throw ClientError.from({
    code: "unauthenticated",
    message: error
      ? `The sign-in was not completed (${error}).`
      : "The sign-in brought back no code.",
    retryable: false,
  });
}

/** An error's message, for "could not reach": never a request's content. */
function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
