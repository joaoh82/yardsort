import type { RemoteController, RemoteTerminalSeen } from "@/lib/ipc";

/** How the terminal stands, by what its channel last said. */
export interface RemoteState {
  controller: RemoteController | null;
  /** This computer has control: the view fits it, and typing goes there. */
  held: boolean;
  /** Control was asked for, and that computer has not answered. */
  asking: boolean;
  /** What the person should know about the last thing that happened to control. */
  notice: string | null;
  /** Not reached just now, and why. */
  away: string | null;
  /** It cannot be watched any more, and why. */
  gone: string | null;
  exit: { code: number; success: boolean } | null;
}

export const WATCHING: RemoteState = {
  controller: null,
  held: false,
  asking: false,
  notice: null,
  away: null,
  gone: null,
  exit: null,
};

/** What one thing the channel said changes. */
export function seenBy(state: RemoteState, seen: RemoteTerminalSeen): RemoteState {
  switch (seen.kind) {
    case "reset":
    case "resized":
      return { ...state, away: null };
    case "controller":
      return { ...state, controller: seen.controller, away: null };
    case "exited":
      return { ...state, exit: { code: seen.code, success: seen.success } };
    case "held":
      return { ...state, held: true, asking: false, notice: null };
    case "notHeld":
      return { ...state, held: false, asking: false, notice: seen.why };
    case "away":
      return { ...state, away: seen.why };
    case "gone":
      return { ...state, held: false, asking: false, gone: seen.why };
  }
}

/** Who has control, in words, as this computer sees it. */
export function controllerText(state: RemoteState): string {
  if (state.held) return "This computer has control.";
  const controller = state.controller;
  switch (controller?.kind) {
    case undefined:
      return "";
    case "nobody":
      return "Nobody has control.";
    case "local":
      return "The person at that computer has control.";
    case "device":
      return `${controller.name || "Another device"} has control.`;
  }
}
