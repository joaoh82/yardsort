// The operator of a computer with no screen, finishing its setup from the dashboard, for a smoke
// test with no person at it. Signs in to the dashboard with a link from a terminal of their own,
// then, as the page does: signs the host in to the stand-in server by pasting the address a
// browser came back to; shows a pairing code, has the pretend phone ask with it, and lets the
// phone watch and type; opens a terminal, types into it, and is refused `yardsort-host pair`
// there; and reads what the dashboard recorded of all that.
//
//   bun .yardsort/tests/support/dashboard-remote.ts <run dir> <port>
//
// `yardsort-host` runs through headless.sh: the build headless-build.sh made, on this run's profile.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, port] = process.argv.slice(2);
const HOST = "sh .yardsort/tests/support/headless.sh yardsort-host";
const address = `http://127.0.0.1:${port}`;
const server = JSON.parse(readFileSync(join(dir, "stand-in.json"), "utf8").split("\n")[0]).ready
  .server;

/** This run's environment without a launched session's markers: the operator's own terminal. */
function own(): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (value !== undefined && !/^YARDSORT_(RUN|WORKSPACE|SESSION_RECORD)_ID$/.test(key))
      env[key] = value;
  }
  return env;
}

function fail(why: string): never {
  console.error(why);
  process.exit(1);
}

// A terminal of the operator's own: `script` gives the command one.
const asked = Bun.spawnSync(["script", "-qefc", `${HOST} dashboard`, "/dev/null"], {
  env: own(),
  stdin: "ignore",
});
const link = asked.stdout.toString().match(/http:\/\/127\.0\.0\.1:\d+\/#code=([0-9A-Z-]+)/);
if (!link) fail(`No sign-in link in what the host said:\n${asked.stdout}${asked.stderr}`);

let cookie = "";
/** A request as the dashboard's page makes it: the status, and the body it carried. */
async function page(method: string, path: string, body?: unknown) {
  const answer = await fetch(`${address}${path}`, {
    method,
    headers: {
      "x-yardsort-dashboard": "1",
      ...(body === undefined ? {} : { "content-type": "application/json" }),
      ...(cookie ? { cookie } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const set = (answer.headers.get("set-cookie") ?? "").split(";")[0];
  if (set.startsWith("yardsort_dashboard_")) cookie = set;
  return { status: answer.status, body: await answer.json().catch(() => null) };
}

/** The same, for one that must be taken. */
async function did(method: string, path: string, body?: unknown) {
  const answer = await page(method, path, body);
  if (answer.status !== 200)
    fail(`${method} ${path} answered ${answer.status}: ${JSON.stringify(answer.body)}`);
  return answer.body;
}

async function until<T>(what: string, look: () => Promise<T | undefined | false>): Promise<T> {
  for (let tries = 0; tries < 300; tries++) {
    const found = await look();
    if (found) return found;
    await Bun.sleep(200);
  }
  fail(`Never: ${what}.`);
}

await did("POST", "/api/session", { code: link[1] });
console.log("signed in to the dashboard, with a link from a terminal of the operator's own");

// ── The host is signed in to the server, from the page ──────────────────────────────────────

const before = await did("GET", "/api/remote");
if (!before.managed || before.signedIn !== null)
  fail(`Not a host with Remote off and managed here: ${JSON.stringify(before)}`);
const early = await page("POST", "/api/remote/pair", {});
if (early.status !== 400) fail(`A pairing code with no server answered ${early.status}.`);

const begun = await did("POST", "/api/remote/sign-in", { server, name: "Smoke box" });

/** Where the browser ends up, followed one hop at a time, as a browser on another computer. */
async function signInIn(browser: string): Promise<string> {
  let at = browser;
  for (let hops = 0; hops < 10; hops++) {
    const answer = await fetch(at, { redirect: "manual" });
    const next = answer.headers.get("location");
    if (!next) throw new Error(`The sign-in stopped at ${at} (${answer.status}).`);
    const url = new URL(next, at);
    // On the other computer this address goes nowhere: it is what gets pasted.
    if (url.hostname === "127.0.0.1" && url.pathname === "/signed-in") return url.href;
    at = url.href;
  }
  throw new Error("Too many redirects.");
}

const back = await signInIn(begun.url);
if (new URL(back).port !== String(begun.port))
  fail(`The browser came back to ${back}, not to port ${begun.port}.`);
// Not the address first: a wrong paste is said, and the sign-in still waits.
const wrong = await page("POST", "/api/remote/sign-in/finish", {
  address: "http://127.0.0.1:1/signed-in?code=not-this-one",
});
if (wrong.status !== 400 || !/not the address this sign-in waits for/.test(wrong.body?.message))
  fail(`A wrong paste answered ${wrong.status}: ${JSON.stringify(wrong.body)}`);
if ((await did("GET", "/api/remote")).signingIn.state !== "waiting")
  fail("A wrong paste ended the sign-in.");
await did("POST", "/api/remote/sign-in/finish", { address: back });
const signed = await did("GET", "/api/remote");
if (signed.signedIn?.name !== "Smoke box" || signed.signingIn.state !== "none")
  fail(`Not signed in as Smoke box: ${JSON.stringify(signed)}`);
console.log(`signed in from the page: ${signed.signedIn.name} at ${signed.signedIn.serverUrl}`);
// Signed in while serving: the link to the relay comes up by itself.
await until("the link to the server came up", async () =>
  (await did("GET", "/api/remote")).signedIn?.link.startsWith("connected"),
);
console.log("the link to the server is up");
const twice = await page("POST", "/api/remote/sign-in", { server, name: "Another" });
if (twice.status !== 400 || !/signed in already/.test(twice.body?.message))
  fail(`Signing in a second time answered ${twice.status}: ${JSON.stringify(twice.body)}`);

// ── A phone is let in, from the page ────────────────────────────────────────────────────────

const offer = await did("POST", "/api/remote/pair", {});
if (!/^[A-Z0-9-]{6,}$/.test(offer.code) || offer.qr.length < 21 || offer.expiresInMs <= 0)
  fail(`Not a pairing offer: ${JSON.stringify(offer).slice(0, 300)}`);
writeFileSync(join(dir, "pairing-code"), offer.code);
const asking = Bun.spawnSync(["bun", ".yardsort/tests/support/phone.ts", dir, "ask"], {
  stdout: "inherit",
  stderr: "inherit",
});
if (asking.exitCode !== 0) fail("The pretend phone could not ask.");
const waiting = await until("the phone showed as asking", async () => {
  const remote = await did("GET", "/api/remote");
  return remote.asking.length === 1 && remote.asking[0];
});
if (!waiting.wantsControl) fail(`The phone asked to type too: ${JSON.stringify(waiting)}`);
if ((await did("GET", "/api/remote")).devices.length !== 0) fail("Let in before any answer.");
await did("POST", "/api/remote/answer", { grant: waiting.grant, allow: true, control: true });
const after = await did("GET", "/api/remote");
if (after.asking.length !== 0 || after.devices.length !== 1 || !after.devices[0].control)
  fail(`Not one device that watches and types: ${JSON.stringify(after)}`);
console.log(`let in from the page: the pretend phone, as ${after.devices[0].name}`);

// ── A terminal in the page ──────────────────────────────────────────────────────────────────

const terminal = await did("POST", "/api/terminals", { cols: 200, rows: 50 });
const bin = readFileSync(join(dir, "bin"), "utf8").trim();
/** What the terminal has shown, without the line breaks its width puts into a long sentence. */
async function shown(wanted: string): Promise<string> {
  return until(`the terminal showed ${wanted}`, async () => {
    const reading = await did("GET", `/api/terminals/${terminal.id}/output?from=0&wait=200`);
    const text = Buffer.from(reading.data, "base64")
      .toString()
      .replace(/[\r\n]/g, "");
    return text.includes(wanted) && text;
  });
}
const type = (data: string) => did("POST", `/api/terminals/${terminal.id}/input`, { data });
await type('read answer; echo "typed <$answer>"\n');
await type("into-the-page\n");
await shown("typed <into-the-page>");
console.log("a terminal in the page answered what was typed into it");
await type(`${bin}/yardsort-host pair; echo asked-$((6*7))\n`);
const refused = await shown("asked-42");
if (!refused.includes("Pairing is refused to a command or a terminal the dashboard ran"))
  fail(`A terminal in the page was not refused pairing:\n${refused}`);
console.log("refused: yardsort-host pair, typed into the page's terminal");
await did("POST", `/api/terminals/${terminal.id}/close`, {});

// ── What was done is recorded, without its codes ────────────────────────────────────────────

const activity = JSON.stringify(await did("GET", "/api/activity"));
for (const said of [
  "A browser signed in, on this computer.",
  `This computer was signed in to ${server.replace(/\/$/, "")}`,
  "A pairing code was shown on the page.",
  "was let in to watch and type, from the page.",
  "A terminal was opened",
])
  if (!activity.includes(said)) fail(`Not in the activity: ${said}\n${activity}`);
for (const secret of [offer.code, link[1], new URL(back).searchParams.get("code") ?? "no-code"])
  if (activity.includes(secret)) fail(`A code is in the activity: ${secret}`);
console.log("recorded: the sign-ins, the pairing and the terminal, with no code in them");
