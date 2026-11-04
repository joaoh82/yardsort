// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The same, in a column of times: no words around it. */
export function stamp(ms: number, now = Date.now()): string {
  return moment(ms, now).replace(/^(at|on) /, "");
}

/** The issuer's host, which is what a person recognises: `auth.example.com`. */
export function issuerHost(issuer: string): string {
  return /^https?:\/\/([^/?#]+)/i.exec(issuer)?.[1] ?? issuer;
}

/** A time a person reads: today's as a time, an earlier one as a date. */
export function moment(ms: number, now = Date.now()): string {
  const at = new Date(ms);
  const sameDay = new Date(now).toDateString() === at.toDateString();
  return sameDay
    ? `at ${at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
    : `on ${at.toLocaleDateString()}`;
}
