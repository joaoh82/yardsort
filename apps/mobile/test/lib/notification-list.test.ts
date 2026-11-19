// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The Notifications list read from a computer (49), against the real server, relay and host
// through the stand-in (`crates/server/examples/stand-in.rs`), whose host keeps a list as the
// app does. The words and the three parts are tested without a server in `notifications.test.ts`.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { StandIn, START_TIMEOUT_MS } from "../../../../packages/remote-client/test/stand-in.ts";
import {
  keepsList,
  Lists,
  needing,
  NOTIFICATION_FOLLOW,
  opens,
  sections,
  unseen,
  what,
} from "../../src/lib/notifications";
import { paired } from "./phone";

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start();
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

async function until(what: string, done: () => boolean): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(20);
  }
}

describe("a computer's Notifications list, on the phone", () => {
  test("it is read, a finished row is marked seen there, and a change is told, not asked for", async () => {
    const { accounts } = await paired(standIn, "List phone");
    await standIn.conversation("Add an imperial option");
    expect(await standIn.agentFinished()).toBe(true);

    const channel = await accounts.openChannel(standIn.host);
    try {
      // The host says it keeps a list, and that it tells of changes.
      expect(keepsList(channel)).toBe(true);
      expect(channel.host.capabilities).toContain(NOTIFICATION_FOLLOW);

      const lists = new Lists();
      const host = standIn.host;
      lists.sync([{ host, name: standIn.hostName, channel, away: false }]);
      await until("the list is read", () => unseen(lists.getSnapshot()) === 1);
      expect(needing(lists.getSnapshot())).toBe(0);

      const [row] = sections(lists.getSnapshot()).finished;
      if (!row) throw new Error("no finished row");
      expect(row.kind).toBe("agent.finished");
      expect(row.projectName).toBe("demo");
      expect(row.workspaceName).toBe("work");
      expect(row.hostName).toBe(standIn.hostName);
      expect(what(row)).toBe("claude finished after 12 min");
      // It opens the conversation it is about, by the host's own ids.
      const to = opens(row);
      expect(to?.pathname).toBe("/terminal");
      expect(to?.params.session).toBe(row.session as string);
      expect(to?.params.workspace).toBe(row.workspace as string);

      // Seen here is seen there: read again from the host, the row is under Earlier.
      await lists.seen(host, [row.id]);
      await lists.read(host);
      const after = sections(lists.getSnapshot());
      expect(after.finished).toEqual([]);
      expect(after.earlier.map((r) => r.id)).toEqual([row.id]);
      expect(unseen(lists.getSnapshot())).toBe(0);

      // The agent finishes again. Nothing asks: the host says its list changed, and it is read.
      expect(await standIn.agentFinished()).toBe(true);
      await until("the phone is told of the new row", () => unseen(lists.getSnapshot()) === 1);
      const newer = sections(lists.getSnapshot()).finished[0];
      expect(newer?.id).not.toBe(row.id);

      // It asks for a permission, as its hook reports: a row that needs its person, told too.
      expect(await standIn.agentWaits()).toBe(true);
      await until("the phone is told that it asks", () => needing(lists.getSnapshot()) === 1);
      const [asks] = sections(lists.getSnapshot()).needs;
      if (!asks) throw new Error("no row that needs its person");
      expect(what(asks)).toBe("claude asks for a permission");
      // A row that waits is not marked seen: it leaves when it is answered.
      await lists.seen(host, [asks.id]);
      await lists.read(host);
      expect(needing(lists.getSnapshot())).toBe(1);
    } finally {
      channel.close();
    }
  }, 60_000);

  test("a phone that may not watch terminals gets no rows, and is told why", async () => {
    const { accounts } = await paired(standIn, "Narrow phone", ["workspace:read"]);
    const channel = await accounts.openChannel(standIn.host);
    try {
      const lists = new Lists();
      lists.sync([{ host: standIn.host, name: standIn.hostName, channel, away: false }]);
      await until("it is refused", () => !!lists.getSnapshot().get(standIn.host)?.problem);
      const held = lists.getSnapshot().get(standIn.host);
      expect(held?.page).toBeNull();
      expect(sections(lists.getSnapshot())).toEqual({ needs: [], finished: [], earlier: [] });
    } finally {
      channel.close();
    }
  }, 60_000);
});
