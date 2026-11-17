// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Exporting an account (`remote_proto::export`): everything the server keeps of it, a page at a
 * time, put together as the file its person keeps. A device reads only what it needs to page and
 * keeps the rest as the server sent it, so a field a newer server adds is in the file too.
 */

import { array, type Decoder, DecodeError, isObject, object, optional, text } from "./decode.ts";

/** The capability a server lists when it answers `GET /v1/account/export`. */
export const EXPORT_CAPABILITY = "account.export";
/** The file's format. */
export const EXPORT_FORMAT = 1;

/** One page of an export: the account on the first only, records on each, `next` until the last. */
export interface ExportPage {
  account?: Record<string, unknown>;
  records: ExportedRecord[];
  next?: string;
}

/** A synced record as the server sent it. Only its id is read. */
export type ExportedRecord = Record<string, unknown> & { id: string };

const asIs: Decoder<Record<string, unknown>> = (value, path) => {
  if (!isObject(value)) throw new DecodeError(path, "is not an object");
  return value;
};

const exportedRecord: Decoder<ExportedRecord> = (value, path) => {
  const record = asIs(value, path);
  text()(record.id, `${path}.id`);
  return record as ExportedRecord;
};

export const exportPage: Decoder<ExportPage> = object({
  account: optional(asIs),
  records: array(exportedRecord),
  next: optional(text()),
});

/** The file: every page put together, as `remote_proto::export::AccountExport` writes it. */
export interface AccountExport {
  format: number;
  server: string;
  serverName: string;
  exportedAt: number;
  account: Record<string, unknown>;
  records: ExportedRecord[];
}
