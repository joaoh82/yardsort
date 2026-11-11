// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import { noticeRoute } from "../../src/lib/notice-link";
import { askedScopes } from "../../src/lib/accounts";
import {
  DecisionIds,
  conflictSummary,
  decisionAvailability,
  OFFLINE,
  workflowTarget,
} from "../../src/lib/workflows";

const ID = "11111111-1111-4111-8111-111111111111";

describe("a workflow notice", () => {
  test("keeps the notice link and opens the decision from the notice itself", () => {
    expect(noticeRoute(`yardsort://notice/workflow/${ID}`)).toBe(`/notice/${ID}`);
    expect(noticeRoute(`yardsort://notice/workflow/${ID}?run=abc`)).toBeNull();
    expect(
      workflowTarget({
        category: "workflow",
        runId: "run-1",
        stepId: "sign-off",
        hostId: "host-1",
      }),
    ).toBe("/decision?host=host-1&run=run-1&step=sign-off");
    expect(
      workflowTarget({ category: "task", runId: "run-1", stepId: "sign-off", hostId: "h" }),
    ).toBeNull();
    expect(workflowTarget({ category: "workflow" })).toBeNull();
  });

  test("does not send a decision while the computer is offline", () => {
    expect(decisionAvailability(false)).toEqual({ enabled: false, reason: OFFLINE });
    expect(decisionAvailability(true).enabled).toBe(true);
  });

  test("reuses a decision id only until a reply arrives", () => {
    const ids = new DecisionIds();
    let n = 0;
    const fresh = () => `id-${++n}`;
    expect(ids.next("approve", "", fresh)).toBe("id-1");
    expect(ids.next("approve", "", fresh)).toBe("id-1");
    expect(ids.next("reject", "", fresh)).toBe("id-2");
    expect(ids.next("approve", "shorter", fresh)).toBe("id-3");
    ids.settled();
    expect(ids.next("approve", "", fresh)).toBe("id-4");
  });

  test("asks for workflow scopes only when the server speaks minor 7", () => {
    expect(askedScopes({ major: 1, minor: 6 })).not.toContain("workflow:read");
    expect(askedScopes({ major: 1, minor: 6 })).not.toContain("workflow:decide");
    expect(askedScopes({ major: 1, minor: 7 })).toContain("workflow:decide");
    expect(askedScopes({ major: 2, minor: 7 })).not.toContain("workflow:decide");
  });

  test("a conflict says what the run is doing now", () => {
    expect(
      conflictSummary({ status: "running", waitingKind: "approval", resumeEligible: false }),
    ).toBe("That step is still waiting for approval.");
    expect(conflictSummary({ status: "running", resumeEligible: true })).toContain("not applied");
    expect(conflictSummary({ status: "succeeded", resumeEligible: false })).toContain("succeeded");
  });
});
