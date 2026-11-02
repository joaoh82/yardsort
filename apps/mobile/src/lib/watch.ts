// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Watching a terminal through whichever channel is open (31 §5). Reconnecting never types: a dropped
 * channel attaches again from the last position drawn, and the computer says whether that goes on or
 * a new snapshot replaces the screen. Nothing of what a terminal prints is kept outside the page.
 */

import {
  type Attachment,
  type AttachmentEnd,
  type Channel,
  ChannelClosed,
  ClientError,
  type Controller,
  type StreamPosition,
  type TermSize,
  type TerminalId,
  type Watcher,
} from "@yardsort/remote-client";

import { messageOf } from "./accounts";
import { drawable, toBase64, type ToApp, type ToPage } from "./bridge";

/** What may be with the page, undrawn, before more waits here. */
const MAX_WITH_PAGE = 256 * 1024;
/** What may wait here. Past this a page is too far behind to catch up on: it starts again. */
const MAX_WAITING = 4 * 1024 * 1024;
/** Starting again this many times without the page catching up means it will not settle. */
const MAX_FRESH_STARTS = 5;

/** What a watch asks to be drawn. */
export interface Drawing {
  reset(size: TermSize): void;
  write(bytes: Uint8Array): void;
  resize(size: TermSize): void;
}

/**
 * What goes to the page, in order, holding back what it has not caught up with. A busy program can
 * print faster than a phone draws, and a WebView's queue of messages has no bottom.
 */
export class PageFeed implements Drawing {
  #post: ((message: string) => void) | null = null;
  #withPage = new Map<number, number>();
  #bytesWithPage = 0;
  #waiting: { message: ToPage; bytes: number }[] = [];
  #bytesWaiting = 0;
  #next = 0;

  /**
   * @param behind Called when the page is too far behind: all that is left is a new snapshot.
   * @param caughtUp Called when the page has drawn everything it was sent, and nothing is waiting.
   */
  constructor(
    readonly behind: () => void,
    readonly caughtUp: () => void = () => {},
  ) {}

  /** Where messages go. The page is starting anew: nothing it was sent counts any more. */
  connect(post: ((message: string) => void) | null): void {
    this.#post = post;
    this.#withPage.clear();
    this.#bytesWithPage = 0;
    this.#waiting = [];
    this.#bytesWaiting = 0;
  }

  reset(size: TermSize): void {
    // Whatever was waiting is about to be replaced.
    this.#waiting = [];
    this.#bytesWaiting = 0;
    this.#send({ t: "reset", cols: size.cols, rows: size.rows });
  }

  write(bytes: Uint8Array): void {
    const id = this.#next;
    this.#next += 1;
    const message: ToPage = { t: "output", id, data: toBase64(bytes) };
    if (this.#waiting.length > 0 || !this.#roomFor(bytes.length)) {
      if (this.#bytesWaiting + bytes.length > MAX_WAITING) {
        this.#waiting = [];
        this.#bytesWaiting = 0;
        this.behind();
        return;
      }
      this.#waiting.push({ message, bytes: bytes.length });
      this.#bytesWaiting += bytes.length;
      return;
    }
    this.#sendOutput(message, id, bytes.length);
  }

  resize(size: TermSize): void {
    const message: ToPage = { t: "resize", cols: size.cols, rows: size.rows };
    // In its place among the output: what came before it was drawn at the old size.
    if (this.#waiting.length > 0) this.#waiting.push({ message, bytes: 0 });
    else this.#send(message);
  }

  /** The page says it drew something. */
  drawn(id: number): void {
    const bytes = this.#withPage.get(id);
    if (bytes === undefined) return;
    this.#withPage.delete(id);
    this.#bytesWithPage -= bytes;
    while (this.#waiting.length > 0) {
      const next = this.#waiting[0] as { message: ToPage; bytes: number };
      if (next.bytes > 0 && !this.#roomFor(next.bytes)) return;
      this.#waiting.shift();
      this.#bytesWaiting -= next.bytes;
      if (next.message.t === "output") {
        this.#sendOutput(next.message, next.message.id, next.bytes);
      } else {
        this.#send(next.message);
      }
    }
    // Nothing is left waiting here: with nothing at the page, all of it went.
    if (this.#withPage.size === 0) this.caughtUp();
  }

  /** With nothing at the page, anything goes: nothing else would ever send it. */
  #roomFor(bytes: number): boolean {
    return this.#bytesWithPage === 0 || this.#bytesWithPage + bytes <= MAX_WITH_PAGE;
  }

  #sendOutput(message: ToPage, id: number, bytes: number): void {
    this.#withPage.set(id, bytes);
    this.#bytesWithPage += bytes;
    this.#send(message);
  }

  #send(message: ToPage): void {
    this.#post?.(JSON.stringify(message));
  }
}

export interface Sight {
  /** The size the program draws at, once the computer has said. */
  size: TermSize | null;
  controller: Controller | null;
  /** The program ended. What it printed last has been drawn. */
  exit: { code: number; success: boolean } | null;
  /** Output is arriving, or would be. */
  following: boolean;
  /** The computer no longer has this terminal. */
  gone: boolean;
  problem: string | null;
}

const NOTHING_YET: Sight = {
  size: null,
  controller: null,
  exit: null,
  following: false,
  gone: false,
  problem: null,
};

export class TerminalWatch {
  #sight: Sight = NOTHING_YET;
  #drawn: StreamPosition | null = null;
  #attachment: Attachment | null = null;
  #channel: Channel | null = null;
  /** Goes up with each start and stop: what an earlier one was waiting for no longer counts. */
  #run = 0;
  #freshStarts = 0;
  /** Whether output past the snapshot has come since the last fresh start. */
  #movedOn = false;
  #listeners = new Set<() => void>();

  constructor(
    readonly terminal: TerminalId,
    readonly drawing: Drawing,
  ) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** The same object until something changes, as `useSyncExternalStore` needs. */
  getSnapshot = (): Sight => this.#sight;

  /** Watch through `channel`, carrying on from what was drawn if the computer can. */
  async follow(channel: Channel): Promise<void> {
    this.#run += 1;
    const run = this.#run;
    this.#channel = channel;
    this.#update({ problem: null });

    // Whatever arrives before the reply has been dealt with goes after the screen is cleared.
    let held: (() => void)[] | null = [];
    const inTurn = (step: () => void) => {
      if (run !== this.#run) return;
      if (held) held.push(step);
      else step();
    };
    const watcher: Watcher = {
      output: (bytes, position) =>
        inTurn(() => {
          // A snapshot's frames carry the position's own number, and always come, so they say
          // nothing about the stream moving on. What comes next, or after resuming, is past it.
          const started = this.#attachment;
          if (started && position.seq > started.position.seq) this.#movedOn = true;
          this.drawing.write(bytes);
        }),
      resized: (size) =>
        inTurn(() => {
          if (!drawable(size)) return this.#tooLarge();
          this.#update({ size });
          this.drawing.resize(size);
        }),
      control: (controller) => inTurn(() => this.#update({ controller })),
      exited: (exit) => inTurn(() => this.#update({ exit })),
      ended: (why) => inTurn(() => this.#ended(why, channel)),
    };

    try {
      const attachment = await this.#attach(channel, watcher, run);
      if (!attachment) return;
      this.#attachment = attachment;
      if (!drawable(attachment.size)) return this.#tooLarge();
      this.#update({
        size: attachment.size,
        controller: attachment.controller,
        following: !attachment.ended,
      });
      if (attachment.start === "snapshot") this.drawing.reset(attachment.size);
    } catch (error) {
      if (run !== this.#run || error instanceof ChannelClosed) return;
      if (error instanceof ClientError && error.code === "not_found") {
        // An ended program's terminal goes in time: what it said stays on the screen.
        this.#update({ following: false, gone: this.#sight.exit === null });
        return;
      }
      this.#update({ following: false, problem: messageOf(error) });
      return;
    }
    const steps = held;
    held = null;
    for (const step of steps) step();
  }

  /** Stop watching. Following again carries on from here. */
  stop(): void {
    this.#run += 1;
    this.#channel = null;
    const attachment = this.#attachment;
    this.#attachment = null;
    if (attachment) {
      this.#remember(attachment);
      void attachment.detach().catch(() => {});
    }
    this.#update({ following: false });
  }

  /**
   * The page has drawn all it was sent. Having moved on past the snapshot too, this is what settling
   * looks like: new output alone is not, since a program outrunning the page has plenty of that.
   */
  pageCaughtUp(): void {
    if (this.#movedOn) this.#freshStarts = 0;
  }

  /** The page started anew, with nothing on it: the next attachment starts from a snapshot. */
  pageStarted(): void {
    this.stop();
    this.#drawn = null;
  }

  /** The page fell too far behind to catch up on: a new snapshot replaces what is on it. */
  startOver(): void {
    const channel = this.#channel;
    this.pageStarted();
    if (channel && this.#mayStartAgain()) void this.follow(channel);
  }

  async #attach(channel: Channel, watcher: Watcher, run: number): Promise<Attachment | null> {
    const from = this.#drawn;
    try {
      const attachment = await channel.attach(this.terminal, watcher, from ?? undefined);
      if (run === this.#run) return attachment;
      void attachment.detach().catch(() => {});
      return null;
    } catch (error) {
      if (!from || !(error instanceof ClientError) || run !== this.#run) throw error;
      // Refused from there. Not a reason to give up on a snapshot.
      this.#drawn = null;
      return await this.#attach(channel, watcher, run);
    }
  }

  #ended(why: AttachmentEnd, channel: Channel): void {
    const attachment = this.#attachment;
    this.#attachment = null;
    if (attachment) this.#remember(attachment);
    this.#update({ following: false });
    switch (why.kind) {
      case "closed":
        // The next channel carries on, or after falling behind, starts from a snapshot.
        if (why.end.reason === "overloaded") this.#drawn = null;
        return;
      case "gap":
        return this.#again(channel);
      case "detached":
        if (why.error.code === "resnapshot_required") return this.#again(channel);
        if (why.error.code === "not_found") {
          this.#update({ gone: this.#sight.exit === null });
          return;
        }
        this.#update({ problem: why.error.message });
        return;
      case "detachedHere":
        return;
    }
  }

  /** Attach again for a snapshot, unless that keeps happening without the page settling. */
  #again(channel: Channel): void {
    this.#drawn = null;
    if (this.#mayStartAgain()) void this.follow(channel);
  }

  /** Count a fresh start, and say so if it is one too many. */
  #mayStartAgain(): boolean {
    this.#freshStarts += 1;
    this.#movedOn = false;
    if (this.#freshStarts <= MAX_FRESH_STARTS) return true;
    this.#update({
      following: false,
      problem: "The computer’s output could not be followed. Go back and try again.",
    });
    return false;
  }

  #remember(attachment: Attachment): void {
    this.#drawn = attachment.drawn;
  }

  #tooLarge(): void {
    const attachment = this.#attachment;
    this.#run += 1;
    this.#attachment = null;
    void attachment?.detach().catch(() => {});
    this.#update({
      following: false,
      problem: "This terminal is too large to draw here.",
    });
  }

  #update(change: Partial<Sight>): void {
    this.#sight = { ...this.#sight, ...change };
    for (const listener of this.#listeners) listener();
  }
}

/** What the page said, for the screen to act on, once what is the feed's own is dealt with. */
export function pageSaid(message: ToApp, feed: PageFeed): ToApp | null {
  if (message.t !== "drawn") return message;
  feed.drawn(message.id);
  return null;
}
