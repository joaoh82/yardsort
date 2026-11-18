// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import { noticeView, notificationPreferences } from "../src/notices.ts";

describe("what reaches the phone", () => {
  const known = {
    muted: false,
    task: true,
    review: true,
    failure: false,
    desktopUpdate: true,
    workflow: true,
  };

  test("an agent's two switches are read when the server sends them", () => {
    expect(
      notificationPreferences({ ...known, agent: true, agentFinished: false }, "preferences"),
    ).toEqual({ ...known, agent: true, agentFinished: false });
  });

  test("a server from before one of them leaves it out, and it stays out", () => {
    // From before agents that finish.
    const earlier = notificationPreferences({ ...known, agent: false }, "preferences");
    expect(earlier).toEqual({ ...known, agent: false });
    expect("agentFinished" in earlier && earlier.agentFinished !== undefined).toBe(false);
    // From before either.
    const first = notificationPreferences(known, "preferences");
    expect(first.agent).toBeUndefined();
    expect(first.agentFinished).toBeUndefined();
  });

  test("an agent that finished is an agent's notice, with its terminal to open", () => {
    const view = noticeView(
      {
        id: "236247a3-f387-4e7b-9428-28d17abb6f84",
        category: "agent",
        kind: "finished",
        title: "An agent finished",
        body: "Open it on this phone to see where it stands.",
        deepLink: "yardsort://notice/agent/236247a3-f387-4e7b-9428-28d17abb6f84",
        workspaceId: "workspace-7",
        terminalId: "pty-7",
        hostId: "host-1",
      },
      "notice",
    );
    expect(view.category).toBe("agent");
    expect(view.kind).toBe("finished");
    expect(view.terminalId).toBe("pty-7");
  });

  test("a category this build does not know is refused, not guessed at", () => {
    expect(() =>
      noticeView(
        { id: "n", category: "weather", kind: "rain", title: "t", body: "b", deepLink: "d" },
        "notice",
      ),
    ).toThrow();
  });
});
