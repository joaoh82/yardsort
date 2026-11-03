// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import { type DeviceId, Secret, type ServerId } from "@yardsort/remote-client";

import { type Profile, Profiles } from "../../src/lib/profiles";
import { MemoryVault } from "./memory-vault";

const profile = (n: number): Profile => ({
  id: String(n).repeat(32).slice(0, 32),
  address: `https://server-${n}.example.com/`,
  server: `server-${n}` as ServerId,
  serverName: `Server ${n}`,
  device: `device-${n}` as DeviceId,
  deviceName: "Ana’s phone",
  signedInAt: 1_700_000_000_000,
});

describe("profiles", () => {
  test("are kept, each with its own credential, and read back", async () => {
    const vault = new MemoryVault();
    const profiles = await Profiles.load(vault);
    await profiles.add(profile(1), new Secret("refresh-1"));
    await profiles.add(profile(2), new Secret("refresh-2"));
    expect(profiles.active?.id).toBe(profile(2).id);

    const again = await Profiles.load(vault);
    expect(again.all.map((p) => p.serverName)).toEqual(["Server 1", "Server 2"]);
    expect(again.active?.id).toBe(profile(2).id);
    expect((await again.refresh(profile(1).id))?.expose()).toBe("refresh-1");
    expect((await again.refresh(profile(2).id))?.expose()).toBe("refresh-2");
    // The list holds no credential.
    expect(vault.kept.get("yardsort.profiles")).not.toContain("refresh-");
  });

  test("a new refresh credential replaces the one spent", async () => {
    const vault = new MemoryVault();
    const profiles = await Profiles.load(vault);
    await profiles.add(profile(1), new Secret("first"));
    await profiles.keeper(profile(1).id)(new Secret("second"));
    expect((await profiles.refresh(profile(1).id))?.expose()).toBe("second");
  });

  test("a credential that comes back after the profile was deleted is not kept", async () => {
    const vault = new MemoryVault();
    const profiles = await Profiles.load(vault);
    await profiles.add(profile(1), new Secret("first"));
    const keep = profiles.keeper(profile(1).id);
    await profiles.forget(profile(1).id);
    await keep(new Secret("second"));
    expect(vault.credentials()).toEqual([]);
  });

  test("a profile is listed only once its credential is kept", async () => {
    const vault = new MemoryVault();
    const profiles = await Profiles.load(vault);
    vault.failing = /^yardsort\.refresh\./;
    await expect(profiles.add(profile(1), new Secret("refresh-1"))).rejects.toThrow();
    expect(profiles.all).toEqual([]);
    expect((await Profiles.load(vault)).all).toEqual([]);
  });

  test("forgetting one deletes its credential, and leaves none in use", async () => {
    const vault = new MemoryVault();
    const profiles = await Profiles.load(vault);
    await profiles.add(profile(1), new Secret("refresh-1"));
    await profiles.add(profile(2), new Secret("refresh-2"));
    await profiles.forget(profile(2).id);
    expect(vault.credentials()).toEqual([`yardsort.refresh.${profile(1).id}`]);
    expect(profiles.all.map((p) => p.id)).toEqual([profile(1).id]);
    // The person picks the next one; it is not switched to by itself.
    expect(profiles.active).toBeNull();
    await profiles.activate(profile(1).id);
    expect((await Profiles.load(vault)).active?.id).toBe(profile(1).id);
  });

  test("a list that does not read, or an entry that does not, is left out", async () => {
    const vault = new MemoryVault();
    vault.kept.set("yardsort.profiles", "not json");
    expect((await Profiles.load(vault)).all).toEqual([]);

    vault.kept.set(
      "yardsort.profiles",
      JSON.stringify({
        version: 1,
        active: "nobody",
        profiles: [profile(1), { ...profile(2), id: "../../elsewhere" }, { id: 3 }],
      }),
    );
    const read = await Profiles.load(vault);
    expect(read.all.map((p) => p.id)).toEqual([profile(1).id]);
    expect(read.active).toBeNull();

    vault.kept.set("yardsort.profiles", JSON.stringify({ version: 2, profiles: [profile(1)] }));
    expect((await Profiles.load(vault)).all).toEqual([]);
  });
});
