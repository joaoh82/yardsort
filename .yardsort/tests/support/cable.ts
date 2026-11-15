// A computer's network cable, for a smoke test that takes one host offline and brings it back:
// a TCP proxy from a port on this computer to the stand-in. A host signed in through it reaches
// the server only while it runs. Killing it cuts every connection through it at once, the relay
// link included; starting it again on the same port plugs the cable back in.
//
//   bun .yardsort/tests/support/cable.ts <run dir> <name> [port]
//
// Without a port it takes a free one. Either way it writes `http://127.0.0.1:<port>/`, the
// address to sign in to, to `<run dir>/<name>.cable`, and runs until it is killed.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Socket } from "bun";

const [dir, name, wanted] = process.argv.slice(2);
const server = new URL(
  JSON.parse(readFileSync(join(dir, "stand-in.json"), "utf8").split("\n")[0]).ready.server,
);

type Near = { far?: Socket<undefined>; waiting: Uint8Array[]; closed: boolean };

const listener = Bun.listen<Near>({
  hostname: "127.0.0.1",
  port: Number(wanted ?? 0),
  socket: {
    open(near) {
      near.data = { waiting: [], closed: false };
      void Bun.connect({
        hostname: server.hostname,
        port: Number(server.port),
        socket: {
          data(_far, chunk) {
            near.write(chunk);
          },
          close() {
            near.end();
          },
          error() {
            near.end();
          },
        },
      }).then(
        (far) => {
          if (near.data.closed) return far.end();
          near.data.far = far;
          for (const chunk of near.data.waiting) far.write(chunk);
          near.data.waiting = [];
        },
        () => near.end(),
      );
    },
    data(near, chunk) {
      if (near.data.far) near.data.far.write(chunk);
      else near.data.waiting.push(new Uint8Array(chunk));
    },
    close(near) {
      near.data.closed = true;
      near.data.far?.end();
    },
  },
});

writeFileSync(join(dir, `${name}.cable`), `http://127.0.0.1:${listener.port}/\n`);
