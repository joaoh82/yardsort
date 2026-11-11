// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A notification's link, `yardsort://notice/{category}/{id}`, as a route the phone can open.
 * The id is the server's. The link carries no code and no action.
 */

const NOTICE = /^yardsort:\/+notice\/[a-z0-9-]+\/([0-9a-f-]{36})$/;

/** The in-app route for a notification link, or null when `path` is not one. */
export function noticeRoute(path: string): string | null {
  const id = NOTICE.exec(path)?.[1];
  return id ? `/notice/${id}` : null;
}
