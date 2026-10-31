// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The protocol's numbers, as `crates/remote-proto` defines them. Each is the Rust constant of the
 * same name; a change there is a change here.
 */

/** The major this client speaks. */
export const PROTOCOL_MAJOR = 1;
/** The minor this client speaks: 4, `terminalsLater` in `host.describe`. */
export const PROTOCOL_MINOR = 4;

/** The largest JSON message either side reads, in bytes. */
export const MAX_CONTROL_FRAME = 64 * 1024;
/** The largest chunk of terminal output in one frame, in bytes. */
export const MAX_OUTPUT_CHUNK = 64 * 1024;
/** Bytes before an output frame's payload. */
export const OUTPUT_HEADER_LEN = 20;
/** The most input one `terminal.input` carries, in bytes. */
export const MAX_INPUT_BYTES = 32 * 1024;
/** The most a page holds, whatever a request asks for. */
export const MAX_PAGE = 200;
/** A control lease lapses this long after it was last renewed. */
export const LEASE_TTL_MS = 30_000;
/** How often a lease holder renews. */
export const LEASE_RENEW_MS = 10_000;
/** Longest client name in a `Hello`, in bytes. */
export const MAX_CLIENT_NAME = 128;
/** The most activity subscriptions one connection holds at once. */
export const MAX_SUBSCRIPTIONS = 16;

/** Longest id, in bytes: ASCII letters, digits and `-_.:`. */
export const MAX_ID_LEN = 128;
/** Longest cursor, in bytes: printable ASCII. */
export const MAX_CURSOR_LEN = 256;
/** Longest name or title, in characters. */
export const MAX_TEXT = 256;
/** Longest label (a harness, a model, a program, an author, a task's source), in characters. */
export const MAX_LABEL = 64;
/** Longest memory note, in characters. */
export const MAX_NOTE = 1000;
/** The most tasks one workspace lists. */
export const MAX_TASKS = 8;
/** Longest task link, in bytes. */
export const MAX_LINK = 512;
/** The most other terminals one sessions page lists. */
export const MAX_TERMINALS = 32;

/** Longest one-time code, refresh credential or ticket, in bytes. */
export const MAX_SECRET_LEN = 256;
/** Longest redirect a device may name, in bytes. */
export const MAX_REDIRECT_LEN = 512;
/** A PKCE verifier's length bounds (RFC 7636 §4.1). */
export const MIN_VERIFIER_LEN = 43;
export const MAX_VERIFIER_LEN = 128;
/** Characters in a pairing code, not counting the dash it is shown with. */
export const CODE_LEN = 8;
/** How long an offer can be redeemed, and how long a request then waits for an answer. */
export const OFFER_TTL_MS = 5 * 60_000;
/** The most projects a grant names one by one. */
export const MAX_GRANT_PROJECTS = 200;

/** How long a relay ticket can be presented. */
export const TICKET_TTL_MS = 60_000;
/** The longest route: letters, digits, `-` and `_`. */
export const MAX_ROUTE_LEN = 64;
