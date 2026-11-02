// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** A phone signed in to the stand-in, and paired with its computer. */

import { expect } from "bun:test";

import { Secret, webCrypto, type Scope } from "@yardsort/remote-client";

import type { StandIn } from "../../../../packages/remote-client/test/stand-in.ts";
import { Accounts, type Browser, type Env } from "../../src/lib/accounts";
import { MemoryVault } from "./memory-vault";

export const browser: Browser = async (url, redirect) => {
  let next = url;
  for (let hop = 0; hop < 5; hop++) {
    const answer = await fetch(next, { redirect: "manual" });
    const location = answer.headers.get("location");
    if (!location) throw new Error(`no redirect (${answer.status})`);
    if (location.startsWith(redirect)) return location;
    next = new URL(location, next).toString();
  }
  throw new Error("the sign-in went round in circles");
};

/** A phone the computer has said yes to, with what it was allowed. */
export async function paired(standIn: StandIn, name: string, scopes?: Scope[]) {
  const shelf = new MemoryVault();
  const env: Env = {
    crypto: webCrypto() as NonNullable<ReturnType<typeof webCrypto>>,
    platform: "ios",
    userAgent: "yardsort-mobile/test (ios)",
    shelf,
  };
  const accounts = new Accounts(env, new MemoryVault());
  await accounts.load();
  expect(await accounts.signIn(await accounts.lookUp(standIn.server), name, browser)).toBe(true);
  const { code } = await standIn.offer();
  const asked = await accounts.pair({ kind: "code", value: new Secret(code) });
  await standIn.pending();
  await standIn.approve(asked.id, scopes);
  const list = await accounts.computers();
  expect(list.computers.map((c) => c.host)).toEqual([standIn.host]);
  return { accounts, shelf, grant: asked };
}
