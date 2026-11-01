// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Links the system hands the app, before the router reads them. A pairing link carries its offer's
 * secret, so it is taken here and held in memory: the router, its history and its params only ever
 * see `/pair`. Whoever sent the link is unknown, so the person is asked before it is used.
 */

import { parsePairingLink } from "@yardsort/remote-client";

import { incoming } from "../lib/pairing";

export function redirectSystemPath({ path }: { path: string; initial: boolean }): string {
  const link = parsePairingLink(path);
  if (!link) return path;
  incoming.hold(link, false);
  return "/pair";
}
