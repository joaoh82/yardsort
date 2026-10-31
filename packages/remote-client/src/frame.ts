// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Terminal output on the wire: a binary message, never JSON (`remote_proto::OutputFrame`).
 *
 * ```text
 * offset  size  field
 *      0     1  kind       = 0x01 (terminal output)
 *      1     1  version    = 0x01
 *      2     2  reserved   = 0, ignored
 *      4     4  stream     u32, little-endian
 *      8     4  epoch      u32, little-endian
 *     12     8  seq        u64, little-endian
 *     20     …  bytes      at most MAX_OUTPUT_CHUNK
 * ```
 *
 * The bytes are the terminal's own, escape sequences and invalid UTF-8 included: they go to the
 * renderer as they are, never read for meaning.
 */

import { MAX_OUTPUT_CHUNK, OUTPUT_HEADER_LEN } from "./limits.ts";
import type { StreamId, StreamPosition } from "./protocol.ts";

export interface OutputFrame {
  stream: StreamId;
  position: StreamPosition;
  bytes: Uint8Array;
}

/**
 * Why a binary message is not an output frame. `seqTooLarge` is this client's alone: a sequence
 * number past 2^53 − 1, which a JavaScript number cannot hold exactly. A terminal would have to
 * print for longer than anything runs to reach it.
 */
export type FrameErrorKind =
  "truncated" | "wrongKind" | "unknownVersion" | "tooLarge" | "seqTooLarge";

export class FrameError extends Error {
  constructor(readonly kind: FrameErrorKind) {
    super(`not a terminal output frame: ${kind}`);
    this.name = "FrameError";
  }
}

const KIND = 0x01;
const VERSION = 0x01;
/** The high half of the largest sequence number a JavaScript number holds exactly. */
const MAX_SEQ_HIGH = 0x1f_ffff;

/** Read a frame that arrived whole, checking its size before anything else. */
export function decodeFrame(message: ArrayBuffer | Uint8Array): OutputFrame {
  const frame = message instanceof Uint8Array ? message : new Uint8Array(message);
  if (frame.length > OUTPUT_HEADER_LEN + MAX_OUTPUT_CHUNK) throw new FrameError("tooLarge");
  if (frame.length < OUTPUT_HEADER_LEN) throw new FrameError("truncated");
  if (frame[0] !== KIND) throw new FrameError("wrongKind");
  if (frame[1] !== VERSION) throw new FrameError("unknownVersion");
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  const low = view.getUint32(12, true);
  const high = view.getUint32(16, true);
  if (high > MAX_SEQ_HIGH) throw new FrameError("seqTooLarge");
  return {
    stream: view.getUint32(4, true),
    position: { epoch: view.getUint32(8, true), seq: high * 0x1_0000_0000 + low },
    bytes: frame.subarray(OUTPUT_HEADER_LEN),
  };
}

/** Write a frame, as a host does. A chunk over {@link MAX_OUTPUT_CHUNK} is refused: split it. */
export function encodeFrame(frame: OutputFrame): Uint8Array {
  if (frame.bytes.length > MAX_OUTPUT_CHUNK) throw new FrameError("tooLarge");
  const { seq, epoch } = frame.position;
  if (!Number.isSafeInteger(seq) || seq < 0) throw new FrameError("seqTooLarge");
  const out = new Uint8Array(OUTPUT_HEADER_LEN + frame.bytes.length);
  const view = new DataView(out.buffer);
  out[0] = KIND;
  out[1] = VERSION;
  view.setUint32(4, frame.stream, true);
  view.setUint32(8, epoch, true);
  view.setUint32(12, seq % 0x1_0000_0000, true);
  view.setUint32(16, Math.floor(seq / 0x1_0000_0000), true);
  out.set(frame.bytes, OUTPUT_HEADER_LEN);
  return out;
}
