// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { jest } from "@jest/globals";

/** Shared with the decision screen's mocks. The screen test sets what the challenge does. */
export const reads: number[] = [];
export const challenge = jest.fn<(...args: unknown[]) => Promise<unknown>>();
export const detail = {
  runId: "run-1",
  name: "Demo",
  status: "running",
  workspace: "fix",
  error: null,
  waitingOn: "sign-off",
  blocking: "Approve the step.",
  waitingKind: "approval",
  attempts: 0,
  locked: false,
  attempt: 0,
  round: 0,
  resumeEligible: false,
  steps: [],
  artifacts: [],
};
