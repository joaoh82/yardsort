// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { ClientError } from "./client.ts";
import type { GrantId, HostId } from "./protocol.ts";
import type { Session } from "./session.ts";

/**
 * What a channel closed as `revoked` (4003) took away, as the server tells it (31 §5). The relay
 * sends the same code when one computer took back this device's grant, when the computer itself
 * was removed or signed out, and when this device was revoked.
 *
 * - `unpaired`: that computer no longer lets this device in. Forget it and what was kept of it;
 *   this device stays signed in, and every other computer goes on working.
 * - `signedOut`: the server refuses this device's own credential. Forget the profile: its
 *   credentials and everything kept under it.
 * - `undetermined`: the server could not be asked, or did not say. Delete nothing, show the
 *   computer as unreachable, and ask again later.
 */
export type Revoked = "unpaired" | "signedOut" | "undetermined";

export async function explainRevoked(
  session: Session,
  grant: GrantId,
  host: HostId,
): Promise<Revoked> {
  const refused = (error: unknown): Revoked | null =>
    error instanceof ClientError && error.code === "unauthenticated" ? "signedOut" : null;
  try {
    const access = await session.access();
    try {
      const state = (await session.client.grant(access, grant)).state;
      if (state === "revoked" || state === "denied" || state === "expired") return "unpaired";
    } catch (error) {
      if (error instanceof ClientError && error.code === "not_found") return "unpaired";
      throw error;
    }
    const hosts = await session.client.hosts(access);
    return hosts.hosts.some((h) => h.id === host) ? "undetermined" : "unpaired";
  } catch (error) {
    return refused(error) ?? "undetermined";
  }
}
