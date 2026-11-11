// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A workflow decision the phone can send while the computer is online.
 * Nothing is kept to send later. A decision id is reused only when the request
 * itself failed before a reply, so a retry applies once.
 */

export const OFFLINE =
  "This computer is offline. The action was not sent, and it will not be sent later.";

export function decisionAvailability(online: boolean): {
  enabled: boolean;
  reason: string | null;
} {
  if (!online) return { enabled: false, reason: OFFLINE };
  return { enabled: true, reason: null };
}

/** The decision screen for a workflow notice, once the phone has read it. */
export function workflowTarget(notice: {
  category: string;
  runId?: string | undefined;
  stepId?: string | undefined;
  hostId?: string | undefined;
}): string | null {
  if (notice.category !== "workflow" || !notice.runId || !notice.stepId || !notice.hostId) {
    return null;
  }
  const host = encodeURIComponent(notice.hostId);
  const run = encodeURIComponent(notice.runId);
  const step = encodeURIComponent(notice.stepId);
  return `/decision?host=${host}&run=${run}&step=${step}`;
}

/** One id per tap. A lost reply reuses it. A reply, including a refusal, starts a new one. */
export class DecisionIds {
  #pending: string | null = null;

  next(fresh: () => string): string {
    this.#pending ??= fresh();
    return this.#pending;
  }

  settled(): void {
    this.#pending = null;
  }
}
