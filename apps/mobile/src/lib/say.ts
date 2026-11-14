// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Typing into a conversation's terminal from Chat (43): control is taken for one message and given
 * back once it is sent, as the person pressed Send, never by itself. The terminal is not resized:
 * Chat draws no terminal, so it stays the computer's size.
 */

import {
  type Channel,
  ChannelClosed,
  ClientError,
  type Controller,
  MAX_INPUT_BYTES,
  type TerminalId,
  utf8Length,
} from "@yardsort/remote-client";

import { SUBMIT_DELAY_MS } from "./control";
import { messageOf } from "./accounts";

/** What sending came to. `uncertain` when it may or may not have reached the computer. */
export type Said =
  | { kind: "sent" }
  | { kind: "held"; by: Controller | null; why: string }
  | { kind: "refused"; why: string }
  | { kind: "uncertain"; why: string };

/** What a message is typed as: keystrokes and Enter, or one paste and Enter for several lines. */
export interface Typed {
  text: string;
  /** Enter after it. */
  enter: boolean;
  /** As a paste even on one line: a path to a picture, which an agent reads as the picture. */
  paste?: boolean;
}

export async function say(
  channel: Channel,
  terminal: TerminalId,
  typed: Typed,
  options: { takeOver?: boolean; heldBy?: Controller | null } = {},
): Promise<Said> {
  const body = typed.text.replace(/\n+$/, "");
  if (utf8Length(body) > MAX_INPUT_BYTES) {
    return { kind: "refused", why: "That is too long to send at once. 32 KB is the most." };
  }
  let control;
  try {
    control = await channel.acquire(terminal, { takeOver: options.takeOver ?? false });
  } catch (error) {
    if (error instanceof ClientError && error.code === "control_conflict") {
      return { kind: "held", by: options.heldBy ?? null, why: messageOf(error) };
    }
    return { kind: "refused", why: words(error) };
  }
  try {
    if (body.length > 0) {
      await control.input(body, typed.paste || body.includes("\n") ? { paste: true } : {});
    }
    if (typed.enter) {
      if (body.length > 0) await new Promise((resolve) => setTimeout(resolve, SUBMIT_DELAY_MS));
      await control.input("\r");
    }
    return { kind: "sent" };
  } catch (error) {
    if (error instanceof ChannelClosed) {
      return {
        kind: "uncertain",
        why: "This may not have reached the computer. Look at the conversation before sending it again.",
      };
    }
    return { kind: "refused", why: words(error) };
  } finally {
    void control.release().catch(() => {});
  }
}

function words(error: unknown): string {
  return error instanceof ChannelClosed
    ? "The computer could not be reached. Nothing was sent."
    : messageOf(error);
}
