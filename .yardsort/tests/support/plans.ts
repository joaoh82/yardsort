// A stand-in entitlements service for plan-limits.yaml, as the server asks one
// (crates/server/src/entitlements.rs): it answers each action as `plans.json` in the run's folder
// says at that moment, `{"device": {"allowed": true}, "stream": {"allowed": false, "message": …}}`,
// and allows anything it does not name. Each question is appended to `plans-asked.jsonl`, with
// whether the token came with it. Its address goes to `plans-url` once it listens.
//
//   bun .yardsort/tests/support/plans.ts <run dir>
//
// It stops when its input closes, so a test that dies takes it with it.

import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir] = process.argv.slice(2);
const TOKEN = "smoke-entitlements-token";

const listener = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  async fetch(request) {
    if (request.method !== "POST" || new URL(request.url).pathname !== "/ask") {
      return new Response("Not here.", { status: 404 });
    }
    const question = (await request.json()) as { action: string };
    const tokened = request.headers.get("authorization") === `Bearer ${TOKEN}`;
    appendFileSync(join(dir, "plans-asked.jsonl"), `${JSON.stringify({ tokened, question })}\n`);
    if (!tokened) return new Response("Who is asking?", { status: 401 });
    const file = join(dir, "plans.json");
    const plans = existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : {};
    return Response.json(plans[question.action] ?? { allowed: true });
  },
});
writeFileSync(join(dir, "plans-url"), `http://127.0.0.1:${listener.port}/ask`);
console.log(`listening on ${listener.port}`);

for await (const _ of Bun.stdin.stream()) {
  // Nothing is read; this only waits for the input to close.
}
listener.stop(true);
process.exit(0);
