// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A workflow decision the phone can send while the computer is online.
 * Nothing is kept to send later. A decision id is reused only when the request
 * itself failed before a reply, so a retry applies once.
 */

import { ClientError } from "@yardsort/remote-client";

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

/** One id per action and note. A lost reply reuses it. A different action or note is a new id. */
export class DecisionIds {
  #pending: { id: string; action: string; note: string } | null = null;

  next(action: string, note: string, fresh: () => string): string {
    if (this.#pending?.action === action && this.#pending.note === note) return this.#pending.id;
    const id = fresh();
    this.#pending = { id, action, note };
    return id;
  }

  settled(): void {
    this.#pending = null;
  }
}

/**
 * The computer refused a challenge because the step moved on. The code is what
 * decides it. The message is only what to show.
 */
export function staleChallenge(error: unknown): boolean {
  return error instanceof ClientError && error.code === "stale";
}

/** What a conflicting decision id turned out to be, after the run is read again. */
export function conflictSummary(detail: {
  status: string;
  waitingKind?: string | undefined;
  resumeEligible: boolean;
}): string {
  if (detail.waitingKind === "approval") return "That step is still waiting for approval.";
  if (detail.waitingKind === "question") return "That step is still waiting for an answer.";
  if (detail.resumeEligible)
    return "That decision was already sent. The computer has not applied it yet.";
  return `That decision was already sent. The run is ${detail.status}.`;
}
