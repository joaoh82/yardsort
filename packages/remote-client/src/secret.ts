// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { DecodeError, type Decoder } from "./decode.ts";
import { MAX_SECRET_LEN } from "./limits.ts";

const HIDDEN = "Secret(…)";

/**
 * A credential, a ticket, a sign-in code or a pairing secret. Printing it, logging an object that
 * holds it or showing it in a debugger says that there is one and nothing of it. It is written
 * whole only where it has to cross the wire: by `JSON.stringify`, and by {@link Secret.expose}.
 */
export class Secret {
  readonly #value: string;

  constructor(value: string) {
    this.#value = value;
  }

  /** The secret itself, for the one place that needs it: sending it, or keeping it. */
  expose(): string {
    return this.#value;
  }

  toJSON(): string {
    return this.#value;
  }

  toString(): string {
    return HIDDEN;
  }

  [Symbol.for("nodejs.util.inspect.custom")](): string {
    return HIDDEN;
  }
}

/** A secret from the wire: not empty, at most {@link MAX_SECRET_LEN} bytes. */
export const secret: Decoder<Secret> = (value, path) => {
  if (typeof value !== "string") throw new DecodeError(path, "is not a string");
  if (value.length === 0 || value.length > MAX_SECRET_LEN) {
    throw new DecodeError(path, "is empty or too long");
  }
  return new Secret(value);
};
