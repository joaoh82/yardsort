// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Having control of a terminal, for as long as the person asked for it (31 §5). Control is only ever
 * taken by a person pressing something: nothing here takes it again by itself. Nothing is ever sent
 * twice either. What may not have arrived is shown as such, and what was waiting when control was
 * lost is dropped.
 */

import {
  type Channel,
  ChannelClosed,
  ClientError,
  type Control,
  type ControlLoss,
  type Controller,
  type DeviceId,
  MAX_INPUT_BYTES,
  type TermSize,
  type TerminalId,
  utf8Length,
} from "@yardsort/remote-client";

import { messageOf } from "./accounts";
import type { Sight } from "./watch";

/** Between text and its Enter, as the computer does for a first prompt: a burst reads as a paste. */
export const SUBMIT_DELAY_MS = 150;
/** Keyboards and rotation come and go in steps: only where it settles is worth a redraw. */
const FIT_SETTLE_MS = 150;
/** Smaller than this, nothing could be used, so the size stays as it was. */
const MIN_COLS = 20;
const MIN_ROWS = 5;

/** How many columns and rows fit, or `null` when too few do. */
export function fitted(
  room: { width: number; height: number },
  cell: { width: number; height: number },
): TermSize | null {
  const cols = Math.floor(room.width / cell.width);
  const rows = Math.floor(room.height / cell.height);
  return cols >= MIN_COLS && rows >= MIN_ROWS ? { cols, rows } : null;
}

/** What to call a device with control. A computer may not have read one's name yet. */
export function deviceName(name: string): string {
  return name || "Another device";
}

/** Something that may or may not have reached the computer. */
export interface Uncertain {
  /** What to call it: the text itself, or the name of a key. */
  said: string;
  /** What to put back in the field, for text. */
  text: string | null;
}

export interface Hold {
  holding: boolean;
  /** Control has been asked for, and the computer has not answered. */
  asking: boolean;
  /** What the person should know about the last thing that happened to control. */
  notice: string | null;
  uncertain: Uncertain | null;
}

const NOT_HOLDING: Hold = { holding: false, asking: false, notice: null, uncertain: null };

/** One write. `pause` is how long to wait before it. */
interface Piece {
  data: string;
  paste?: boolean;
  pause?: number;
}

/** What the person sent in one go: everything in it, or as far as it got. */
interface Unit extends Uncertain {
  pieces: Piece[];
}

const NOT_SENT = "What was still waiting was not sent.";

/** Keystrokes, or a paste when there are several lines, which a program should not read as Enter. */
function typing(text: string): Piece {
  return { data: text, paste: text.includes("\n") };
}

export class TerminalControl {
  #hold: Hold = NOT_HOLDING;
  #control: Control | null = null;
  #asking = 0;
  #waiting: Unit[] = [];
  #working = false;
  #wanted: TermSize | null = null;
  #sized: TermSize | null = null;
  #settling: ReturnType<typeof setTimeout> | null = null;
  #controller: Controller | null = null;
  #listeners = new Set<() => void>();

  constructor(
    readonly terminal: TerminalId,
    /** This phone, to tell itself from another device. */
    readonly me: DeviceId | null,
  ) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): Hold => this.#hold;

  /**
   * Ask for control. From another device that is a take-over, which is why the person sees whose it
   * is before pressing. From the person at the computer it cannot be: only they can let go of it.
   */
  async take(channel: Channel): Promise<void> {
    if (this.#hold.holding || this.#hold.asking) return;
    this.#asking += 1;
    const asked = this.#asking;
    this.#update({ asking: true, notice: null });
    const taken = this.#heldByAnother();
    let control: Control | null = null;
    try {
      control = await channel.acquire(this.terminal, {
        takeOver: taken,
        lost: (why) => this.#lost(control, why),
      });
    } catch (error) {
      if (asked === this.#asking) {
        this.#update({ asking: false, notice: this.#refusal(error) });
      }
      return;
    }
    if (asked !== this.#asking) {
      void control.release().catch(() => {});
      return;
    }
    this.#control = control;
    this.#sized = null;
    this.#update({ asking: false, holding: true });
    this.#resize();
  }

  /** Hand control back to the computer, whose own size comes back with it. */
  release(): void {
    this.#asking += 1;
    const control = this.#control;
    this.#forget(control ? null : this.#hold.notice);
    void control?.release().catch(() => {});
  }

  /** Who the computer says controls the terminal, and whether its program is still there. */
  seen(sight: Sight): void {
    // Only a new report counts: taking over, what was last heard still names the other device.
    const told = sight.controller !== this.#controller;
    this.#controller = sight.controller;
    if (!this.#control) return;
    const ended = sight.exit !== null || sight.gone;
    // Nobody is not a loss by itself: a lapse is found out at the next renewal, and an event saying
    // so can arrive just after the lease that followed it.
    if (ended || (told && (sight.controller?.kind === "local" || this.#heldByAnother()))) {
      const control = this.#control;
      this.#forget(ended ? null : this.#tookIt());
      // It may still be this phone's at the computer. Letting go settles that, either way.
      void control.release().catch(() => {});
    }
  }

  /**
   * Type a line, followed by Enter: nothing typed, and Enter alone. Several lines go as one paste.
   * Returns whether it was taken, and until it is, the person still has their text.
   */
  send(text: string): boolean {
    const body = text.replace(/\n+$/, "");
    if (!this.#control || this.#tooLong(body)) return false;
    const enter: Piece = { data: "\r" };
    if (body.length === 0) {
      this.#queue({ said: "Enter", text: null, pieces: [enter] });
    } else {
      this.#queue({
        said: body,
        text: body,
        pieces: [typing(body), { ...enter, pause: SUBMIT_DELAY_MS }],
      });
    }
    return true;
  }

  /** Type with no Enter after it: a key a menu takes, or the start of a line to finish with Tab. */
  type(text: string): boolean {
    if (!this.#control || text.length === 0 || this.#tooLong(text)) return false;
    this.#queue({ said: text, text, pieces: [typing(text)] });
    return true;
  }

  /** Send what a key or Ctrl and a letter makes, named as the person would name it. */
  press(said: string, data: string): void {
    if (this.#control) this.#queue({ said, text: null, pieces: [{ data }] });
  }

  /** The screen has room for this size. It is asked for only while this phone has control. */
  fit(size: TermSize): void {
    this.#wanted = size;
    if (!this.#control) return;
    if (this.#settling) clearTimeout(this.#settling);
    this.#settling = setTimeout(() => {
      this.#settling = null;
      this.#resize();
    }, FIT_SETTLE_MS);
  }

  /** The person has seen what may not have arrived. Its text, for the field, if any. */
  settleUncertain(): string | null {
    const text = this.#hold.uncertain?.text ?? null;
    this.#update({ uncertain: null });
    return text;
  }

  #tooLong(text: string): boolean {
    if (utf8Length(text) <= MAX_INPUT_BYTES) return false;
    this.#update({ notice: "That is too long to send at once. 32 KB is the most." });
    return true;
  }

  #queue(unit: Unit): void {
    this.#waiting.push(unit);
    void this.#work();
  }

  /** One at a time, each after the last was answered, so nothing is left to send once it is lost. */
  async #work(): Promise<void> {
    if (this.#working) return;
    this.#working = true;
    try {
      for (;;) {
        const unit = this.#waiting.shift();
        const control = this.#control;
        if (!unit || !control) return;
        await this.#deliver(unit, control);
      }
    } finally {
      this.#working = false;
    }
  }

  async #deliver(unit: Unit, control: Control): Promise<void> {
    for (const piece of unit.pieces) {
      if (piece.pause) await new Promise((resolve) => setTimeout(resolve, piece.pause));
      if (this.#control !== control) return this.#notSent();
      try {
        await control.input(piece.data, piece.paste ? { paste: true } : {});
      } catch (error) {
        if (error instanceof ChannelClosed) {
          this.#update({ uncertain: { said: unit.said, text: unit.text } });
        } else if (error instanceof ClientError && error.code === "lease_lost") {
          this.#notSent();
        } else {
          this.#update({ notice: messageOf(error) });
        }
        this.#dropWaiting();
        return;
      }
    }
  }

  #resize(): void {
    const control = this.#control;
    const size = this.#wanted;
    if (!control || !size) return;
    if (this.#sized?.cols === size.cols && this.#sized.rows === size.rows) return;
    this.#sized = size;
    void control.resize(size).catch(() => {
      // Lost control is told apart elsewhere. Otherwise the size stays as it is.
      this.#sized = null;
    });
  }

  #lost(control: Control | null, why: ControlLoss): void {
    if (control !== this.#control) return;
    if (why.kind === "closed") {
      this.#forget("Control ended when the connection dropped. Take it again when you are ready.");
    } else if (why.kind === "lost") {
      this.#forget(this.#tookIt());
    } else {
      this.#forget(null);
    }
  }

  /** No longer holding: nothing waiting goes anywhere. */
  #forget(notice: string | null): void {
    const dropped = this.#waiting.length > 0;
    this.#control = null;
    this.#waiting = [];
    this.#sized = null;
    if (this.#settling) clearTimeout(this.#settling);
    this.#settling = null;
    this.#update({ holding: false, asking: false, notice });
    if (dropped) this.#notSent();
  }

  #dropWaiting(): void {
    if (this.#waiting.length === 0) return;
    this.#waiting = [];
    this.#notSent();
  }

  /** Say that some of what the person sent went nowhere, once. */
  #notSent(): void {
    const notice = this.#hold.notice;
    if (notice?.endsWith(NOT_SENT)) return;
    this.#update({ notice: notice ? `${notice} ${NOT_SENT}` : NOT_SENT });
  }

  #heldByAnother(): boolean {
    const controller = this.#controller;
    return controller?.kind === "device" && controller.device !== this.me;
  }

  #tookIt(): string {
    const controller = this.#controller;
    if (controller?.kind === "local") return "The person at the computer took control.";
    if (controller?.kind === "device" && controller.device !== this.me) {
      return `${deviceName(controller.name)} took control.`;
    }
    return "This phone no longer has control.";
  }

  #refusal(error: unknown): string {
    if (error instanceof ClientError && error.code === "control_conflict") {
      return this.#controller?.kind === "local"
        ? "The person at the computer is typing here. Try again once they stop."
        : "Another device has control of this terminal.";
    }
    if (error instanceof ClientError && error.code === "forbidden") {
      return "This computer lets this phone watch, but not type. To change that, pair again and allow control at the computer.";
    }
    return error instanceof ChannelClosed
      ? "The connection dropped before the computer answered."
      : messageOf(error);
  }

  #update(change: Partial<Hold>): void {
    this.#hold = { ...this.#hold, ...change };
    for (const listener of this.#listeners) listener();
  }
}
