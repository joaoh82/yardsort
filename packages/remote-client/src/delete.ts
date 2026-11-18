// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Deleting an account (`remote_proto::delete`): everything the server keeps of it, for every
 * device. The person types the confirmation, then signs in again at the identity provider; the
 * code that brings back is what deletes, from the device that started that sign-in.
 */

import { type Decoder, object, u32 } from "./decode.ts";

/** The capability a server lists when it answers `POST /v1/account/delete`. */
export const DELETE_CAPABILITY = "account.delete";
/** What the person types to confirm. */
export const DELETE_CONFIRMATION = "delete";

/** What went with the account, for the client to say. */
export interface AccountDeleted {
  devices: number;
  hosts: number;
  records: number;
}

export const accountDeleted: Decoder<AccountDeleted> = object({
  devices: u32,
  hosts: u32,
  records: u32,
});

/** Whether `typed` confirms: `delete`, whatever its case and the spaces around it. */
export function confirmsDeletion(typed: string): boolean {
  return typed.trim().toLowerCase() === DELETE_CONFIRMATION;
}
