// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Links the system hands the app, before the router reads them. A pairing link carries its offer's
 * secret, so it is taken here and held in memory: the router, its history and its params only ever
 * see `/pair`. Whoever sent the link is unknown, so the person is asked before it is used.
 *
 * A sign-in's return carries its code, and stays out of the router too. iOS gives it only to the
 * browser session waiting for it. Android hands it to the app like any link as well, and the router
 * would show it as a page that does not exist. The session listens for it itself, so one still
 * waiting carries on. One that has stopped, or an app started by the return, has nothing to use the
 * code with: it goes no further, and the person signs in again.
 */

import { parsePairingLink } from "@yardsort/remote-client";

import { noticeRoute } from "../lib/notice-link";
import { incoming } from "../lib/pairing";

const SIGN_IN_RETURN = /^yardsort:\/*signed-in(?:[/?#]|$)/;

export function redirectSystemPath({
  path,
  initial,
}: {
  path: string;
  initial: boolean;
}): string | null {
  if (SIGN_IN_RETURN.test(path)) return initial ? "/" : null;
  const notice = noticeRoute(path);
  if (notice) return notice;
  const link = parsePairingLink(path);
  if (!link) return path;
  incoming.hold(link, false);
  return "/pair";
}
