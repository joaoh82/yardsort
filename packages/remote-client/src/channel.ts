// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A device's channel to one computer through the relay: a ticket, the socket, the ticket as its
 * first message, the relay's greeting, then `Hello` and the remote protocol (27 §8–§11).
 *
 * The channel matches each reply to its request, keeps each terminal attachment by its stream
 * and checks that its items come numbered without a gap, routes activity batches to their
 * subscription, and renews a control lease while it is held. When the socket closes it says why,
 * from the relay's close code; `revoked` needs {@link explainRevoked} before anything acts on it.
 *
 * **Liveness.** A browser or React Native WebSocket never shows the relay's pings, so silence
 * cannot be timed from them. Instead a channel that has heard nothing for
 * {@link ChannelOptions.probeAfterMs} asks `host.describe`, and one that hears nothing within
 * {@link ChannelOptions.probeTimeoutMs} after that is taken to be dead and closed (`silent`).
 * Anything that arrives counts, a reply, an event or output; what is sent does not, so a held
 * lease's renewals going unanswered end the channel as surely as a quiet one. Opening a channel
 * again is the caller's: it attaches again from {@link Attachment.drawn}, and never sends again
 * what it typed (see {@link Control.input}).
 */

import { ClientError, type Client } from "./client.ts";
import { DecodeError, isObject, utf8Length } from "./decode.ts";
import { decodeFrame, FrameError, type OutputFrame } from "./frame.ts";
import {
  MAX_CLIENT_NAME,
  MAX_CONTROL_FRAME,
  MAX_OUTPUT_CHUNK,
  OUTPUT_HEADER_LEN,
  PROTOCOL_MAJOR,
} from "./limits.ts";
import {
  type ActivityBatch,
  type ActivityListParams,
  type ActivityPage,
  type ActivitySubscribeParams,
  type Attached,
  type Controller,
  CURRENT,
  type Cursor,
  EVENTS,
  event as eventDecoder,
  helloReply,
  type HostDescription,
  type HostId,
  type Lease,
  type MemoryListParams,
  type MemoryPage,
  type Operation,
  type ProtocolVersion,
  type RemoteError,
  type Reply,
  request as requestDecoder,
  type RequestId,
  response as responseDecoder,
  type SessionListParams,
  type SessionPage,
  type StreamId,
  type StreamPosition,
  type SubscriptionId,
  type TerminalId,
  type TermSize,
  type WorkspaceDetail,
  type WorkspaceId,
  type WorkspaceListParams,
  type WorkspacePage,
} from "./protocol.ts";
import { closeReason, type CloseReason, closeText, greeting as greetingDecoder } from "./relay.ts";
import type { Secret } from "./secret.ts";

/** What a channel needs of a WebSocket: the standard one's, as Bun, browsers and React Native have it. */
export interface Socket {
  binaryType: string;
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((event: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event: { code: number; reason: string }) => void) | null;
  onerror: ((event: unknown) => void) | null;
}

export type SocketFactory = (address: string) => Socket;

export interface ChannelOptions {
  /** Who this is, for the computer's list of devices: `yardsort-mobile/0.1.0 (ios)`. */
  client: string;
  /** Opens the socket. The platform's `WebSocket` unless given. */
  socket?: SocketFactory;
  /** Opening, through the welcome, may take this long. 30 seconds unless given. */
  openTimeoutMs?: number;
  /** Ask after this long without hearing anything. 20 seconds unless given. */
  probeAfterMs?: number;
  /** Give up after this long more without hearing anything. 10 seconds unless given. */
  probeTimeoutMs?: number;
}

/**
 * Why a channel ended: one of the relay's close reasons, `silent` when it stopped answering,
 * `invalid` too when the computer broke the protocol, or `closedHere` when {@link Channel.close}
 * was called. `code` is the WebSocket close code, when one came.
 */
export interface ChannelEnd {
  reason: CloseReason | "silent" | "closedHere";
  code: number | null;
  /** A few words for a person. */
  text: string;
}

/** A request that could not be answered because the channel ended first. */
export class ChannelClosed extends Error {
  constructor(readonly end: ChannelEnd) {
    super(`The connection closed: ${end.text}.`);
    this.name = "ChannelClosed";
  }
}

/** Open a channel to `host`, under the grant the server has in force for this device. */
export async function openChannel(
  client: Client,
  access: Secret,
  host: HostId,
  options: ChannelOptions,
): Promise<Channel> {
  if (utf8Length(options.client) > MAX_CLIENT_NAME) {
    throw new ClientError("invalid", "The client's name is longer than a greeting carries.");
  }
  const ticket = await client.ticket(access, host);
  const open =
    options.socket ??
    ((address: string) =>
      new (globalThis.WebSocket as unknown as new (a: string) => Socket)(address));
  const socket = open(client.relayAddress(ticket.route));
  const channel = new Channel(socket, host, ticket.ticket, options);
  await channel.ready;
  return channel;
}

interface Pending {
  resolve(reply: Reply): void;
  reject(error: Error): void;
  /** Run as the reply is read, before anything after it on the socket. */
  onReply?(reply: Reply): void;
}

type Phase = "opening" | "greeting" | "welcome" | "open" | "ended";

export class Channel {
  /** The computer, as it described itself when it welcomed this device. */
  host!: HostDescription;
  /** The protocol it speaks. */
  protocol!: ProtocolVersion;
  /** Settles once the channel is open: welcomed, or refused. */
  readonly ready: Promise<void>;
  /** Settles when the channel ends, with why. */
  readonly ended: Promise<ChannelEnd>;

  #socket: Socket;
  #phase: Phase = "opening";
  #end: ChannelEnd | null = null;
  #asked = 0;
  #pending = new Map<RequestId, Pending>();
  #attachments = new Map<StreamId, Attachment>();
  #subscriptions = new Map<SubscriptionId, Subscription>();
  #controls = new Set<Control>();
  #probeTimer: ReturnType<typeof setTimeout> | undefined;
  #silentTimer: ReturnType<typeof setTimeout> | undefined;
  #openTimer: ReturnType<typeof setTimeout> | undefined;
  #settleReady!: { resolve(): void; reject(error: Error): void };
  #settleEnded!: (end: ChannelEnd) => void;
  readonly #probeAfterMs: number;
  readonly #probeTimeoutMs: number;

  constructor(socket: Socket, expected: HostId, ticket: Secret, options: ChannelOptions) {
    this.#socket = socket;
    this.#probeAfterMs = options.probeAfterMs ?? 20_000;
    this.#probeTimeoutMs = options.probeTimeoutMs ?? 10_000;
    this.ready = new Promise((resolve, reject) => {
      this.#settleReady = { resolve, reject };
    });
    this.ended = new Promise((resolve) => {
      this.#settleEnded = resolve;
    });
    this.#openTimer = setTimeout(
      () => this.#fail(new ClientError("unreachable", "The relay did not answer in time."), "gone"),
      options.openTimeoutMs ?? 30_000,
    );
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      this.#phase = "greeting";
      socket.send(JSON.stringify({ ticket }));
    };
    socket.onmessage = ({ data }) => this.#message(data, expected, options.client);
    socket.onclose = ({ code }) => this.#closed(code);
    socket.onerror = () => {
      // The close that follows says why, as far as anything does.
    };
  }

  /** Whether the channel still carries requests. */
  get open(): boolean {
    return this.#phase === "open";
  }

  /** Close the channel. Leases held through it lapse at the computer, or are released first by {@link Control.release}. */
  close(): void {
    this.#end ??= { reason: "closedHere", code: null, text: "closed here" };
    this.#shut(1000);
  }

  // --- asking ----------------------------------------------------------------------------------

  /** Send one request and wait for its reply. A refusal rejects with a {@link ClientError} of kind `remote`. */
  ask(operation: Operation, onReply?: (reply: Reply) => void): Promise<Reply> {
    try {
      return this.send(operation, onReply);
    } catch (error) {
      return Promise.reject(error as Error);
    }
  }

  /**
   * @internal {@link ask}, except that a request that cannot be sent throws here, before
   * anything is sent: so a caller can tell a request refused here from one that went.
   */
  send(operation: Operation, onReply?: (reply: Reply) => void): Promise<Reply> {
    if (this.#phase !== "open") {
      throw this.#end
        ? new ChannelClosed(this.#end)
        : new ClientError("invalid", "The channel is not open yet.");
    }
    const requestId = `r${this.#asked + 1}` as RequestId;
    let wire: string;
    try {
      // Read back as a host would, so nothing malformed is ever sent.
      const checked = requestDecoder({ requestId, protocol: CURRENT, ...operation }, "");
      wire = JSON.stringify(checked);
    } catch (error) {
      const why = error instanceof DecodeError ? error.message : String(error);
      throw new ClientError("invalid", `Not a request a host would take: ${why}`);
    }
    if (utf8Length(wire) > MAX_CONTROL_FRAME) {
      throw new ClientError("invalid", "That request is larger than any request may be.");
    }
    this.#asked += 1;
    const reply = new Promise<Reply>((resolve, reject) => {
      this.#pending.set(requestId, { resolve, reject, ...(onReply ? { onReply } : {}) });
    });
    this.#socket.send(wire);
    return reply;
  }

  async describe(): Promise<HostDescription> {
    const { kind: _, ...host } = expect(await this.ask({ op: "host.describe" }), "host");
    return host;
  }

  async workspaces(params: Partial<WorkspaceListParams> = {}): Promise<WorkspacePage> {
    const full = { project: null, includeArchived: false, cursor: null, limit: null, ...params };
    const { kind: _, ...page } = expect(
      await this.ask({ op: "workspace.list", params: full }),
      "workspaces",
    );
    return page;
  }

  async workspace(workspace: WorkspaceId): Promise<WorkspaceDetail> {
    const reply = await this.ask({ op: "workspace.describe", params: { workspace } });
    const { kind: _, ...detail } = expect(reply, "workspace");
    return detail;
  }

  async sessions(
    params: Pick<SessionListParams, "workspace"> & Partial<SessionListParams>,
  ): Promise<SessionPage> {
    const full = { includeEnded: false, cursor: null, limit: null, ...params };
    const { kind: _, ...page } = expect(
      await this.ask({ op: "session.list", params: full }),
      "sessions",
    );
    return page;
  }

  async activity(
    params: Pick<ActivityListParams, "workspace"> & Partial<ActivityListParams>,
  ): Promise<ActivityPage> {
    const full = { session: null, cursor: null, limit: null, ...params };
    const { kind: _, ...page } = expect(
      await this.ask({ op: "activity.list", params: full }),
      "activity",
    );
    return page;
  }

  async memory(
    params: Pick<MemoryListParams, "project"> & Partial<MemoryListParams>,
  ): Promise<MemoryPage> {
    const full = { cursor: null, limit: null, ...params };
    const { kind: _, ...page } = expect(
      await this.ask({ op: "memory.list", params: full }),
      "memory",
    );
    return page;
  }

  // --- terminals -------------------------------------------------------------------------------

  /**
   * Watch a terminal: from a snapshot, or from `resumeAfter`, the last position drawn, when the
   * computer still keeps everything since. The reply says which ({@link Attachment.start}).
   */
  async attach(
    terminal: TerminalId,
    watcher: Watcher,
    resumeAfter?: StreamPosition,
  ): Promise<Attachment> {
    let attachment: Attachment | undefined;
    await this.ask(
      { op: "terminal.attach", params: { terminal, resumeAfter: resumeAfter ?? null } },
      (reply) => {
        if (reply.kind !== "attached") return;
        // Kept before the next message is read: the snapshot's frames follow the reply at once.
        attachment = new Attachment(this, terminal, reply, watcher);
        this.#attachments.set(reply.stream, attachment);
      },
    );
    if (!attachment) throw new ClientError("unexpected", "The computer answered something else.");
    return attachment;
  }

  /** Ask for control of a terminal. It is renewed while held, and lost when taken or lapsed. */
  async acquire(
    terminal: TerminalId,
    options: { takeOver?: boolean; lost?: (why: ControlLoss) => void } = {},
  ): Promise<Control> {
    const reply = await this.ask({
      op: "terminal.acquire",
      params: { terminal, takeOver: options.takeOver ?? false },
    });
    const control = new Control(this, terminal, expect(reply, "lease"), options.lost);
    this.#controls.add(control);
    return control;
  }

  // --- activity --------------------------------------------------------------------------------

  /** Follow activity as it is recorded: everything the grant reaches, unless `params` narrows it. */
  async subscribe(params: ActivitySubscribeParams, follower: Follower): Promise<Subscription> {
    let subscription: Subscription | undefined;
    await this.ask({ op: "activity.subscribe", params }, (reply) => {
      if (reply.kind !== "subscribed") return;
      subscription = new Subscription(this, reply.subscription, reply.newer, follower);
      this.#subscriptions.set(reply.subscription, subscription);
    });
    if (!subscription) throw new ClientError("unexpected", "The computer answered something else.");
    return subscription;
  }

  // --- inside ----------------------------------------------------------------------------------

  /** @internal */
  forget(what: Attachment | Subscription | Control): void {
    if (what instanceof Attachment) this.#attachments.delete(what.stream);
    else if (what instanceof Subscription) this.#subscriptions.delete(what.id);
    else this.#controls.delete(what);
  }

  /** Something arrived: the link is alive. Probe once nothing has for a while. */
  #heard(): void {
    clearTimeout(this.#probeTimer);
    clearTimeout(this.#silentTimer);
    if (this.#phase !== "open") return;
    this.#probeTimer = setTimeout(() => this.#probe(), this.#probeAfterMs);
  }

  #probe(): void {
    if (this.#phase !== "open") return;
    // Anything heard first clears this, the answer or not.
    this.#silentTimer = setTimeout(() => {
      this.#end ??= { reason: "silent", code: null, text: "the computer stopped answering" };
      this.#shut(4000);
    }, this.#probeTimeoutMs);
    this.ask({ op: "host.describe" }).catch(() => {});
  }

  #message(data: unknown, expected: HostId, client: string): void {
    if (this.#phase === "ended") return;
    this.#heard();
    if (typeof data !== "string") {
      this.#binary(data);
      return;
    }
    if (utf8Length(data) > MAX_CONTROL_FRAME) {
      this.#broken("a message larger than any may be");
      return;
    }
    let value: unknown;
    try {
      value = JSON.parse(data);
    } catch {
      this.#broken("a message that is not JSON");
      return;
    }
    switch (this.#phase) {
      case "greeting":
        return this.#greeted(value, expected, client);
      case "welcome":
        return this.#welcomed(value);
      case "open":
        if (isObject(value) && "event" in value) this.#event(value);
        else this.#response(value);
        return;
      default:
        this.#broken("a message before the socket opened");
    }
  }

  #greeted(value: unknown, expected: HostId, client: string): void {
    let greeting;
    try {
      greeting = greetingDecoder(value, "");
    } catch {
      return this.#fail(
        new ClientError("unexpected", "The relay's greeting was not understood."),
        "invalid",
      );
    }
    if ("refused" in greeting) return this.#fail(ClientError.from(greeting.refused), "refused");
    if (greeting.connected.host !== expected) {
      return this.#fail(
        new ClientError("unexpected", "The relay connected to another computer."),
        "invalid",
      );
    }
    this.#phase = "welcome";
    this.#socket.send(JSON.stringify({ client, protocols: [CURRENT.major] }));
  }

  #welcomed(value: unknown): void {
    let reply;
    try {
      reply = helloReply(value, "");
    } catch {
      return this.#fail(
        new ClientError("unexpected", "The computer's welcome was not understood."),
        "invalid",
      );
    }
    if ("error" in reply) return this.#fail(ClientError.from(reply.error), "refused");
    if (reply.welcome.protocol.major !== PROTOCOL_MAJOR) {
      return this.#fail(
        new ClientError("unexpected", "The computer speaks another version of the protocol."),
        "invalid",
      );
    }
    this.host = reply.welcome.host;
    this.protocol = reply.welcome.protocol;
    this.#phase = "open";
    clearTimeout(this.#openTimer);
    this.#heard();
    this.#settleReady.resolve();
  }

  #response(value: unknown): void {
    let response;
    try {
      response = responseDecoder(value, "");
    } catch (error) {
      // The request it answers, if that much can be read, learns that it was not understood.
      const id =
        isObject(value) && typeof value.requestId === "string"
          ? (value.requestId as RequestId)
          : null;
      const pending = id === null ? undefined : this.#pending.get(id);
      if (pending && id !== null) {
        this.#pending.delete(id);
        const why = error instanceof DecodeError ? error.message : "it did not read";
        pending.reject(
          new ClientError("unexpected", `The computer's answer was not understood: ${why}`),
        );
        return;
      }
      return this.#broken("an answer that does not read");
    }
    const pending = response.requestId === null ? undefined : this.#pending.get(response.requestId);
    if (!pending || response.requestId === null) return;
    this.#pending.delete(response.requestId);
    if ("error" in response) {
      pending.reject(ClientError.from(response.error));
      return;
    }
    pending.onReply?.(response.result);
    pending.resolve(response.result);
  }

  #event(value: Record<string, unknown>): void {
    // An event a newer computer sends is not this build's to read.
    if (!(EVENTS as readonly unknown[]).includes(value.event)) return;
    let event;
    try {
      event = eventDecoder(value, "");
    } catch {
      return this.#broken("an event that does not read");
    }
    switch (event.event) {
      case "terminal.resized":
        this.#attachments.get(event.stream)?.resized(event.position, event.size);
        return;
      case "terminal.control":
        this.#attachments.get(event.stream)?.controlled(event.controller);
        return;
      case "terminal.exited":
        this.#attachments.get(event.stream)?.exited(event.exit);
        return;
      case "terminal.detached":
        this.#attachments.get(event.stream)?.end({ kind: "detached", error: event.error });
        return;
      case "activity.events": {
        const { event: _, ...batch } = event;
        this.#subscriptions.get(event.subscription)?.batch(batch);
        return;
      }
      case "activity.ended":
        this.#subscriptions.get(event.subscription)?.end({ kind: "ended", error: event.error });
        return;
    }
  }

  #binary(data: unknown): void {
    if (this.#phase !== "open") return this.#broken("terminal output before the welcome");
    const bytes =
      data instanceof ArrayBuffer ? new Uint8Array(data) : data instanceof Uint8Array ? data : null;
    if (!bytes) return this.#broken("a message of a kind no computer sends");
    if (bytes.length > OUTPUT_HEADER_LEN + MAX_OUTPUT_CHUNK) {
      return this.#broken("terminal output larger than a frame may be");
    }
    let frame: OutputFrame;
    try {
      frame = decodeFrame(bytes);
    } catch (error) {
      // A frame of a kind or version from a newer computer is not this build's to read.
      if (
        error instanceof FrameError &&
        (error.kind === "wrongKind" || error.kind === "unknownVersion")
      )
        return;
      return this.#broken("terminal output that does not read");
    }
    this.#attachments.get(frame.stream)?.output(frame);
  }

  /** The computer broke the protocol: the channel is closed rather than drawn wrong. */
  #broken(what: string): void {
    this.#end ??= { reason: "invalid", code: null, text: `the computer sent ${what}` };
    this.#shut(4400);
  }

  /** Opening failed. */
  #fail(error: Error, reason: CloseReason): void {
    this.#end ??= { reason, code: null, text: error.message };
    this.#settleReady.reject(error);
    this.#shut(1000);
  }

  #shut(code: number): void {
    try {
      this.#socket.close(code);
    } catch {
      // Closing a socket that never opened can throw; it is closed either way.
    }
    this.#finish();
  }

  #closed(code: number): void {
    const reason = code >= 4000 && code <= 4999 ? closeReason(code) : "gone";
    const text = reason === "unknown" ? `${closeText(reason)} (code ${code})` : closeText(reason);
    this.#end ??= { reason, code, text };
    if (this.#phase !== "open" && this.#phase !== "ended") {
      // Closed while opening: what the relay says it was, or that it went.
      this.#settleReady.reject(
        new ClientError("unreachable", `The relay closed the connection: ${text}.`),
      );
    }
    this.#finish();
  }

  #finish(): void {
    if (this.#phase === "ended") return;
    this.#phase = "ended";
    const end = this.#end ?? { reason: "gone", code: null, text: closeText("gone") };
    clearTimeout(this.#probeTimer);
    clearTimeout(this.#silentTimer);
    clearTimeout(this.#openTimer);
    this.#socket.onmessage = null;
    for (const pending of this.#pending.values()) pending.reject(new ChannelClosed(end));
    this.#pending.clear();
    for (const attachment of [...this.#attachments.values()])
      attachment.end({ kind: "closed", end });
    for (const subscription of [...this.#subscriptions.values()])
      subscription.end({ kind: "closed", end });
    for (const control of [...this.#controls]) control.lose({ kind: "closed", end });
    this.#settleReady.reject(new ChannelClosed(end));
    this.#settleEnded(end);
  }
}

/** A reply of the kind asked for. */
function expect<K extends Reply["kind"]>(reply: Reply, kind: K): Extract<Reply, { kind: K }> {
  if (reply.kind !== kind)
    throw new ClientError("unexpected", "The computer answered something else.");
  return reply as Extract<Reply, { kind: K }>;
}

// --- attachments -----------------------------------------------------------------------------

/** What a watcher of a terminal is told. Only `output` is needed. */
export interface Watcher {
  /** The terminal's own bytes, to the renderer unchanged. A snapshot comes this way first. */
  output(bytes: Uint8Array, position: StreamPosition): void;
  resized?(size: TermSize, position: StreamPosition): void;
  /** Who controls the terminal from now on. */
  control?(controller: Controller): void;
  /** The program ended, and everything it printed has come. */
  exited?(exit: { code: number; success: boolean }): void;
  /** The stream is over. Nothing more comes on it. */
  ended?(why: AttachmentEnd): void;
}

/**
 * Why a stream ended: the computer detached it (`resnapshot_required` when this device fell
 * behind), an item came out of order (`gap`: attach again for a snapshot), it was detached here,
 * or the channel closed (attach again on a new one, from {@link Attachment.drawn}).
 */
export type AttachmentEnd =
  | { kind: "detached"; error: RemoteError }
  | { kind: "gap"; expected: StreamPosition; got: StreamPosition }
  | { kind: "detachedHere" }
  | { kind: "closed"; end: ChannelEnd };

export class Attachment {
  readonly stream: StreamId;
  readonly start: Attached["start"];
  /** Where it began, as the reply said: what the snapshot covers, or what it resumed after. */
  readonly position: StreamPosition;
  /** The size the program draws at now. A viewer draws at this size. */
  size: TermSize;
  controller: Controller;
  /** Set once the program has ended. */
  exit: { code: number; success: boolean } | null = null;
  #ended = false;
  readonly #epoch: number;
  #last: number;
  /** Whether the last item was output, which a frame with the same number continues. */
  #inOutput = false;
  /**
   * Whether that output may have more to come: its last frame was a full one. Every frame of a
   * split piece but its last is full, so a shorter frame, or the next item, says it is whole.
   */
  #mayContinue = false;
  #drewAny: boolean;

  constructor(
    readonly channel: Channel,
    readonly terminal: TerminalId,
    attached: Attached,
    readonly watcher: Watcher,
  ) {
    this.stream = attached.stream;
    this.start = attached.start;
    this.position = attached.position;
    this.size = attached.size;
    this.controller = attached.controller;
    this.#epoch = attached.position.epoch;
    // A snapshot's frames carry the position's own number; a resumed stream goes on after it.
    this.#last = attached.start === "snapshot" ? attached.position.seq - 1 : attached.position.seq;
    this.#drewAny = attached.start === "resumed";
  }

  /**
   * The last position drawn whole, to resume from on another channel. `null` when there is none
   * to resume from, and the next attachment starts from a snapshot: before the snapshot has come,
   * or while the last piece of output may have more frames to come. Resuming after a piece cut
   * short would lose its end; resuming before it would draw its start twice.
   */
  get drawn(): StreamPosition | null {
    if (!this.#drewAny || this.#mayContinue) return null;
    return { epoch: this.#epoch, seq: this.#last };
  }

  get ended(): boolean {
    return this.#ended;
  }

  /** Stop watching. */
  async detach(): Promise<void> {
    if (this.#ended) return;
    this.end({ kind: "detachedHere" });
    await this.channel.ask({
      op: "terminal.detach",
      params: { terminal: this.terminal, stream: this.stream },
    });
  }

  /** @internal */
  output(frame: OutputFrame): void {
    if (this.#ended) return;
    const { epoch, seq } = frame.position;
    const continues = this.#inOutput && seq === this.#last;
    if (epoch !== this.#epoch || (!continues && seq !== this.#last + 1))
      return this.#gap(frame.position);
    this.#last = seq;
    this.#inOutput = true;
    this.#mayContinue = frame.bytes.length === MAX_OUTPUT_CHUNK;
    this.#drewAny = true;
    this.watcher.output(frame.bytes, frame.position);
  }

  /** @internal */
  resized(position: StreamPosition, size: TermSize): void {
    if (this.#ended) return;
    if (position.epoch !== this.#epoch || position.seq !== this.#last + 1)
      return this.#gap(position);
    this.#last = position.seq;
    this.#inOutput = false;
    this.#mayContinue = false;
    this.size = size;
    this.watcher.resized?.(size, position);
  }

  /** @internal */
  controlled(controller: Controller): void {
    if (this.#ended) return;
    this.controller = controller;
    this.watcher.control?.(controller);
  }

  /** @internal */
  exited(exit: { code: number; success: boolean }): void {
    if (this.#ended) return;
    this.exit = exit;
    this.watcher.exited?.(exit);
  }

  /** @internal */
  end(why: AttachmentEnd): void {
    if (this.#ended) return;
    this.#ended = true;
    this.channel.forget(this);
    this.watcher.ended?.(why);
  }

  #gap(got: StreamPosition): void {
    const expected = { epoch: this.#epoch, seq: this.#last + 1 };
    this.end({ kind: "gap", expected, got });
    // The computer is told to stop sending what is no longer drawn. Best effort: the channel may
    // be on its way out.
    this.channel
      .ask({ op: "terminal.detach", params: { terminal: this.terminal, stream: this.stream } })
      .catch(() => {});
  }
}

// --- control ---------------------------------------------------------------------------------

/**
 * Why control was lost: the computer said the lease is no longer the current one (taken over,
 * taken back by the person there, or lapsed), it was released here, or the channel closed.
 */
export type ControlLoss =
  { kind: "lost"; error: RemoteError } | { kind: "released" } | { kind: "closed"; end: ChannelEnd };

/**
 * Typing into a terminal, under a lease. Input is numbered within the lease, and never sent again:
 * when {@link input} rejects with {@link ChannelClosed}, whether it arrived is not known, and the
 * person is the one to say whether to type it again.
 */
export class Control {
  #lease: Lease;
  #seq = 0;
  #held = true;
  #renewer: ReturnType<typeof setInterval> | undefined;

  constructor(
    readonly channel: Channel,
    readonly terminal: TerminalId,
    lease: Lease,
    readonly lost?: (why: ControlLoss) => void,
  ) {
    this.#lease = lease;
    this.#renewer = setInterval(() => this.#renew(), lease.renewEveryMs);
  }

  get lease(): Lease {
    return this.#lease;
  }

  get held(): boolean {
    return this.#held;
  }

  /**
   * Type `data`: keystrokes as the renderer made them, or text to paste. Input refused here, too
   * large or not a request a host would take, is never sent and takes no number, so what is typed
   * next is still the next one the computer expects.
   */
  async input(data: string, options: { paste?: boolean } = {}): Promise<void> {
    if (!this.#held) throw new ClientError("invalid", "Control is no longer held.");
    const seq = this.#seq + 1;
    const reply = this.channel.send({
      op: "terminal.input",
      params: { ...this.#ref(), seq, data, paste: options.paste ?? false },
    });
    this.#seq = seq;
    await this.#leased(reply);
  }

  /** Resize the terminal to fit this device. The size stands while control is held. */
  async resize(size: TermSize): Promise<void> {
    await this.#leased(
      this.channel.ask({ op: "terminal.resize", params: { ...this.#ref(), size } }),
    );
  }

  /** Hand control back. */
  async release(): Promise<void> {
    if (!this.#held) return;
    const ref = this.#ref();
    this.lose({ kind: "released" });
    await this.channel.ask({ op: "terminal.release", params: ref });
  }

  /** @internal */
  lose(why: ControlLoss): void {
    if (!this.#held) return;
    this.#held = false;
    clearInterval(this.#renewer);
    this.channel.forget(this);
    this.lost?.(why);
  }

  #ref() {
    return { terminal: this.terminal, lease: this.#lease.id, generation: this.#lease.generation };
  }

  async #renew(): Promise<void> {
    if (!this.#held) return;
    try {
      const reply = await this.#leased(
        this.channel.ask({ op: "terminal.renew", params: this.#ref() }),
      );
      if (reply.kind === "lease") {
        const { kind: _, ...lease } = reply;
        this.#lease = lease;
      }
    } catch {
      // A lost lease is reported by #leased; a closed channel by the channel.
    }
  }

  /** A lease's reply: one that says the lease is no longer current loses control. */
  async #leased(reply: Promise<Reply>): Promise<Reply> {
    try {
      return await reply;
    } catch (error) {
      if (error instanceof ClientError && error.code === "lease_lost") {
        this.lose({ kind: "lost", error: error.remote as RemoteError });
      }
      throw error;
    }
  }
}

// --- subscriptions ---------------------------------------------------------------------------

export interface Follower {
  /** What was recorded next, oldest first, each event once. */
  events(batch: ActivityBatch): void;
  /** The subscription is over. */
  ended?(why: SubscriptionEndReason): void;
}

export type SubscriptionEndReason =
  | { kind: "ended"; error: RemoteError }
  | { kind: "unsubscribed" }
  | { kind: "closed"; end: ChannelEnd };

export class Subscription {
  #ended = false;

  constructor(
    readonly channel: Channel,
    readonly id: SubscriptionId,
    /** Where it got to: subscribe after this again on a new channel, and miss nothing still kept. */
    public newer: Cursor | null,
    readonly follower: Follower,
  ) {}

  async unsubscribe(): Promise<void> {
    if (this.#ended) return;
    this.end({ kind: "unsubscribed" });
    await this.channel.ask({ op: "activity.unsubscribe", params: { subscription: this.id } });
  }

  /** @internal */
  batch(batch: ActivityBatch): void {
    if (this.#ended) return;
    this.newer = batch.newer;
    this.follower.events(batch);
  }

  /** @internal */
  end(why: SubscriptionEndReason): void {
    if (this.#ended) return;
    this.#ended = true;
    this.channel.forget(this);
    this.follower.ended?.(why);
  }
}
