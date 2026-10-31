// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Reading untrusted JSON into typed values, with the protocol's bounds.
 *
 * A decoder either returns a value in the wire's own shape, the way the Rust side writes it, or
 * throws a {@link DecodeError} naming where it went wrong. Its message never quotes the value: what
 * arrives may be a credential or a person's own text. Fields this build does not know are dropped,
 * as the Rust side drops them, so a newer peer's additions are not a failure.
 *
 * Writing a decoded value back with `JSON.stringify` gives what the Rust side would write: fields
 * it always writes are present (`null` when empty), and fields it leaves out when empty are left
 * out. That is what the fixtures pin.
 */

export class DecodeError extends Error {
  constructor(
    readonly path: string,
    reason: string,
  ) {
    super(`${path || "the message"} ${reason}`);
    this.name = "DecodeError";
  }
}

export type Decoder<T> = (value: unknown, path: string) => T;

/** Decode `value`, the whole of a message. */
export function decode<T>(decoder: Decoder<T>, value: unknown): T {
  return decoder(value, "");
}

/** Parse `text` as JSON and decode it. */
export function parse<T>(decoder: Decoder<T>, text: string): T {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new DecodeError("", "is not JSON");
  }
  return decoder(value, "");
}

const at = (path: string, key: string | number) =>
  typeof key === "number" ? `${path}[${key}]` : path ? `${path}.${key}` : key;

/** How many characters `text` holds, as Rust counts them: code points, not UTF-16 units. */
export function chars(text: string): number {
  let count = 0;
  for (const _ of text) count++;
  return count;
}

/** How many bytes `text` takes as UTF-8. */
export function utf8Length(text: string): number {
  let bytes = 0;
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i);
    if (unit < 0x80) bytes += 1;
    else if (unit < 0x800) bytes += 2;
    else if (unit >= 0xd800 && unit <= 0xdbff && i + 1 < text.length) {
      const next = text.charCodeAt(i + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        i++;
      } else bytes += 3;
    } else bytes += 3;
  }
  return bytes;
}

/** Whether `text` holds a control character, as Rust's `char::is_control` has them. */
export function hasControl(text: string): boolean {
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i);
    if (unit < 0x20 || (unit >= 0x7f && unit <= 0x9f)) return true;
  }
  return false;
}

/** A string of at most `max` characters, when given. */
export function text(max?: number): Decoder<string> {
  return (value, path) => {
    if (typeof value !== "string") throw new DecodeError(path, "is not a string");
    if (max !== undefined && value.length > max && chars(value) > max) {
      throw new DecodeError(path, `is longer than ${max} characters`);
    }
    return value;
  };
}

export const bool: Decoder<boolean> = (value, path) => {
  if (typeof value !== "boolean") throw new DecodeError(path, "is not true or false");
  return value;
};

/**
 * A whole number from `min` to `max`. Rust's 64-bit numbers are held to what a JavaScript number
 * keeps exactly, 2^53 − 1, and refused past it rather than rounded.
 */
export function integer(min: number, max: number): Decoder<number> {
  return (value, path) => {
    if (typeof value !== "number" || !Number.isInteger(value)) {
      throw new DecodeError(path, "is not a whole number");
    }
    if (value < min || value > max) throw new DecodeError(path, "is out of range");
    return value;
  };
}

export const u16 = integer(0, 0xffff);
export const u32 = integer(0, 0xffff_ffff);
export const u64 = integer(0, Number.MAX_SAFE_INTEGER);
export const i64 = integer(Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER);

/** One of `values`. A value it does not know is `fallback`, when there is one, and refused otherwise. */
export function oneOf<const T extends string>(values: readonly T[], fallback?: T): Decoder<T> {
  return (value, path) => {
    if (typeof value !== "string") throw new DecodeError(path, "is not a string");
    if ((values as readonly string[]).includes(value)) return value as T;
    if (fallback !== undefined) return fallback;
    throw new DecodeError(path, "is not one this build knows");
  };
}

/** A list of at most `max` items, when given. */
export function array<T>(item: Decoder<T>, max?: number): Decoder<T[]> {
  return (value, path) => {
    if (!Array.isArray(value)) throw new DecodeError(path, "is not a list");
    if (max !== undefined && value.length > max) {
      throw new DecodeError(path, `holds more than ${max} items`);
    }
    return value.map((v, i) => item(v, at(path, i)));
  };
}

/** A list read as a set: each item once, in `order` (as Rust's `BTreeSet` writes it). */
export function set<T>(
  item: Decoder<T>,
  order: (a: T, b: T) => number,
  max?: number,
): Decoder<T[]> {
  const list = array(item, max);
  return (value, path) => {
    const items = list(value, path).sort(order);
    return items.filter((v, i) => i === 0 || order(items[i - 1] as T, v) !== 0);
  };
}

/** Byte order, which is Rust's for the ASCII strings ids are. */
export const byOrder = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

const OPTIONAL = Symbol("optional");

/** A field Rust leaves out when it is empty: absent or `null` reads as absent. */
export type Optional<T> = Decoder<T | undefined> & { [OPTIONAL]: true };

export function optional<T>(decoder: Decoder<T>): Optional<T> {
  const read = ((value: unknown, path: string) =>
    value === undefined || value === null ? undefined : decoder(value, path)) as Optional<T>;
  read[OPTIONAL] = true;
  return read;
}

/** A field Rust always writes, `null` when empty. Absent reads as `null`. */
export function nullable<T>(decoder: Decoder<T>): Decoder<T | null> {
  return (value, path) => (value === undefined || value === null ? null : decoder(value, path));
}

/** A field Rust fills in with `fallback` when it is absent. */
export function orElse<T>(decoder: Decoder<T>, fallback: () => T): Decoder<T> {
  return (value, path) => (value === undefined ? fallback() : decoder(value, path));
}

type Shape = Record<string, Decoder<unknown>>;
type Required<S extends Shape> = {
  [K in keyof S as S[K] extends Optional<unknown> ? never : K]: S[K] extends Decoder<infer T>
    ? T
    : never;
};
type Omitted<S extends Shape> = {
  [K in keyof S as S[K] extends Optional<unknown> ? K : never]?: S[K] extends Optional<infer T>
    ? T
    : never;
};
export type Decoded<S extends Shape> = Required<S> & Omitted<S>;

export function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** An object with these fields, in this order. Others are dropped. */
export function object<S extends Shape>(shape: S): Decoder<Decoded<S>> {
  return (value, path) => {
    if (!isObject(value)) throw new DecodeError(path, "is not an object");
    const out: Record<string, unknown> = {};
    for (const [key, field] of Object.entries(shape)) {
      const read = field(value[key], at(path, key));
      if (read !== undefined || !(OPTIONAL in field)) out[key] = read;
    }
    return out as Decoded<S>;
  };
}

/**
 * Rust's internally tagged enums: `{"<tag>": "<variant>", …the variant's fields}`. A variant this
 * build does not know is `fallback`'s, when there is one.
 */
export function tagged<T>(
  tag: string,
  variants: Record<string, Decoder<T>>,
  fallback?: Decoder<T>,
): Decoder<T> {
  return (value, path) => {
    if (!isObject(value)) throw new DecodeError(path, "is not an object");
    const name = value[tag];
    if (typeof name !== "string") throw new DecodeError(at(path, tag), "is not a string");
    const variant = Object.prototype.hasOwnProperty.call(variants, name)
      ? variants[name]
      : fallback;
    if (!variant) throw new DecodeError(at(path, tag), "is not one this build knows");
    return variant(value, path);
  };
}

/** A variant's fields with its tag in front: what one of {@link tagged}'s variants reads. */
export function variant<const N extends string, const K extends string, S extends Shape>(
  tag: K,
  name: N,
  shape: S,
): Decoder<{ [P in K]: N } & Decoded<S>> {
  const fields = object(shape);
  return (value, path) => ({ [tag]: name, ...fields(value, path) }) as { [P in K]: N } & Decoded<S>;
}
