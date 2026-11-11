// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A TCP proxy that holds every byte back for a while each way, to try a round trip longer than
 * this computer's own, as a phone's on cellular is (35 §3, "Measuring the budgets").
 *
 * ```text
 * bun tools/delay-proxy.ts TARGET DELAY_MS [PORT]
 * ```
 *
 * `TARGET` is `host:port`, such as a local server's `127.0.0.1:8787`. It listens on `127.0.0.1`,
 * on `PORT` or one the system picks, and prints `listening on 127.0.0.1:<port>`. Each piece goes on
 * `DELAY_MS` after it came, in the order it came, so a round trip through it takes twice that
 * longer. It carries the bytes as they are: point an `http://` client at it; a TLS server's name
 * would not match `127.0.0.1`, and a real distance needs no proxy anyway.
 */

import type { Socket } from "bun";

export interface DelayProxy {
  port: number;
  stop(): void;
}

interface Side {
  /** The socket at the other end, once connected. */
  peer: Socket<Side> | null;
  /** What came before the other end was connected. */
  early: Uint8Array[];
  /** When the last piece is due out, so none overtakes the one before. */
  lastDue: number;
}

/** Listen on `127.0.0.1:port` (0 for any), and carry each connection to `target` late by `delayMs`. */
export function delayProxy(target: string, delayMs: number, port = 0): DelayProxy {
  const [host, targetPort] = splitTarget(target);
  // A piece goes on once its time comes, after every piece before it on that side.
  const later = (from: Side, to: () => Socket<Side> | null, bytes: Uint8Array) => {
    const due = Math.max(Date.now() + delayMs, from.lastDue);
    from.lastDue = due;
    const copy = bytes.slice();
    setTimeout(() => to()?.write(copy), due - Date.now());
  };
  const server = Bun.listen<Side>({
    hostname: "127.0.0.1",
    port,
    socket: {
      open(client) {
        client.data = { peer: null, early: [], lastDue: 0 };
        void Bun.connect<Side>({
          hostname: host,
          port: targetPort,
          socket: {
            open(upstream) {
              upstream.data = { peer: client, early: [], lastDue: 0 };
              client.data.peer = upstream;
              for (const bytes of client.data.early) later(client.data, () => upstream, bytes);
              client.data.early = [];
            },
            data(upstream, bytes) {
              later(upstream.data, () => client, bytes);
            },
            close(upstream) {
              setTimeout(() => client.end(), Math.max(0, upstream.data.lastDue - Date.now()));
            },
            error() {
              client.end();
            },
          },
        }).catch(() => client.end());
      },
      data(client, bytes) {
        const upstream = client.data.peer;
        if (!upstream) client.data.early.push(bytes.slice());
        else later(client.data, () => upstream, bytes);
      },
      close(client) {
        const upstream = client.data.peer;
        if (upstream)
          setTimeout(() => upstream.end(), Math.max(0, client.data.lastDue - Date.now()));
      },
    },
  });
  return { port: server.port, stop: () => server.stop(true) };
}

function splitTarget(target: string): [string, number] {
  const at = target.lastIndexOf(":");
  const port = Number(target.slice(at + 1));
  if (at <= 0 || !Number.isInteger(port) || port <= 0 || port > 65_535) {
    throw new Error(`${target} is not host:port.`);
  }
  return [target.slice(0, at), port];
}

if (import.meta.main) {
  const [target, delay, port] = process.argv.slice(2);
  const delayMs = Number(delay);
  if (!target || !Number.isFinite(delayMs) || delayMs < 0) {
    console.error("bun tools/delay-proxy.ts TARGET DELAY_MS [PORT]");
    process.exit(2);
  }
  const proxy = delayProxy(target, delayMs, port ? Number(port) : 0);
  console.log(`listening on 127.0.0.1:${proxy.port}, ${delayMs} ms each way to ${target}`);
}
