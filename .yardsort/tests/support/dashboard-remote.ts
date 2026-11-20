// The operator of a computer with no screen, doing Remote from its dashboard, for a smoke test with
// no person at it: signs a browser's worth of requests in to the dashboard with a link from
// `yardsort-host dashboard` at a terminal of their own, then, as the page's Remote tab does:
//
//   sign-in  signs the host in to the stand-in server, following the sign-in page as a browser on
//            another computer would and pasting back the address it lands on, a wrong one first;
//   pair     shows a pairing code, has the pretend phone ask with it, and lets it watch and type;
//            first checks that a command the dashboard runs is still refused `pair`;
//   sign-out signs the host out of the server.
//
//   bun .yardsort/tests/support/dashboard-remote.ts <run dir> <port> sign-in | pair | sign-out
//
// `yardsort-host` runs through headless.sh: the build headless-build.sh made, on this run's profile.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, port, doing] = process.argv.slice(2);
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

/** A request as the dashboard's page makes it. */
async function page(method: string, path: string, body?: unknown, cookie?: string) {
  return fetch(`${address}${path}`, {
    method,
    headers: {
      "x-yardsort-dashboard": "1",
      ...(body === undefined ? {} : { "content-type": "application/json" }),
      ...(cookie ? { cookie } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

const signedIn = await page("POST", "/api/session", { code: link[1] });
if (signedIn.status !== 200) fail(`Signing in to the dashboard answered ${signedIn.status}.`);
const cookie = (signedIn.headers.get("set-cookie") ?? "").split(";")[0];

async function read(path: string) {
  const answer = await page("GET", path, undefined, cookie);
  if (answer.status !== 200) fail(`GET ${path} answered ${answer.status}.`);
  return answer.json();
}

async function post(path: string, body: unknown = {}) {
  const answer = await page("POST", path, body, cookie);
  return { status: answer.status, body: await answer.json() };
}

/** Wait, up to `seconds`, for `check` to give something. */
async function until<T>(what: string, check: () => Promise<T | undefined>, seconds = 60) {
  const since = Date.now();
  while (Date.now() - since < seconds * 1000) {
    const found = await check();
    if (found !== undefined) return found;
    await Bun.sleep(200);
  }
  fail(`Not within ${seconds}s: ${what}`);
}

/** Where a browser ends up, followed one hop at a time, as a browser on another computer. */
async function landsOn(start: string): Promise<string> {
  let at = start;
  for (let hops = 0; hops < 10; hops++) {
    const answer = await fetch(at, { redirect: "manual" });
    const next = answer.headers.get("location");
    if (!next) fail(`The sign-in stopped at ${at} (${answer.status}).`);
    const url = new URL(next, at);
    // On the other computer this address goes nowhere: it is what gets pasted.
    if (url.hostname === "127.0.0.1" && url.pathname === "/signed-in") return url.href;
    at = url.href;
  }
  fail("Too many redirects.");
}

/** Have the dashboard run a line, and wait for what it printed. */
async function ran(command: string): Promise<string> {
  const { body: job } = await post("/api/commands", { command });
  return until(`the command ends: ${command}`, async () => {
    const reading = await read(`/api/jobs/${job.id}?from=0`);
    return reading.state === "running" ? undefined : (reading.output as string);
  });
}

switch (doing) {
  case "sign-in": {
    const begun = await post("/api/remote/sign-in", { server, name: "Smoke box" });
    if (begun.status !== 200 || begun.body.state !== "waiting")
      fail(`Signing in did not begin: ${JSON.stringify(begun.body)}`);
    console.log("the sign-in page is given");
    const back = await landsOn(begun.body.url);
    const wrong = await post("/api/remote/sign-in/pasted", {
      address: "http://127.0.0.1:1/signed-in?code=not-this-one",
    });
    if (wrong.status === 200) fail("A wrong address was taken.");
    console.log(`a wrong address: ${wrong.body.message}`);
    const done = await post("/api/remote/sign-in/pasted", { address: back });
    const state = await until("signed in", async () => {
      const remote =
        done.body.state === "signedIn" ? done.body : (await read("/api/remote")).signingIn;
      if (remote.state === "failed") fail(`Signing in failed: ${remote.message}`);
      return remote.state === "signedIn" ? remote : undefined;
    });
    console.log(`signed in as ${state.name}`);
    const link = await until("linked", async () => {
      const status = await read("/api/status");
      return status.host.remote?.link?.startsWith("connected")
        ? status.host.remote.link
        : undefined;
    });
    console.log(`link: ${link}`);
    break;
  }
  case "pair": {
    const bin = (await Bun.file(`${dir}/bin`).text()).trim();
    const through = await ran(`script -qefc '${bin}/yardsort-host pair' /dev/null`);
    if (!through.includes("is refused to a command or a terminal the dashboard ran"))
      fail(`A command the dashboard ran was not refused pairing:\n${through}`);
    console.log("refused: a command the dashboard ran, pairing");

    const offered = await post("/api/remote/pair");
    if (offered.body.state !== "offered") fail(`No offer: ${JSON.stringify(offered.body)}`);
    const rows: string[] = offered.body.qr;
    if (rows.length < 21 || rows.some((row) => row.length !== rows.length))
      fail(`Not a QR code: ${rows.length} rows`);
    writeFileSync(join(dir, "pairing-code"), offered.body.code);
    console.log(`a code and a ${rows.length}×${rows.length} QR code`);
    const asking = Bun.spawnSync(["bun", ".yardsort/tests/support/phone.ts", dir, "ask"], {
      stdout: "inherit",
      stderr: "inherit",
    });
    if (asking.exitCode !== 0) fail("The phone could not ask.");
    const asked = await until("the phone asks", async () => {
      const pairing = (await read("/api/remote")).pairing;
      return pairing.state === "asked" ? pairing : undefined;
    });
    console.log(`${asked.device} asks${asked.wantsControl ? " to type" : ""}`);
    const answered = await post("/api/remote/pair/answer", {
      grant: asked.grant,
      allow: true,
      control: true,
    });
    if (answered.body.state !== "answered" || !answered.body.allowed)
      fail(`Not let in: ${JSON.stringify(answered.body)}`);
    console.log("Let in.");
    break;
  }
  case "sign-out": {
    const out = await post("/api/remote/sign-out");
    if (out.status !== 200) fail(`Signing out answered ${out.status}: ${out.body.message}`);
    const status = await read("/api/status");
    if (status.host.remote) fail("Still signed in.");
    console.log("signed out of the server");
    break;
  }
  default:
    fail(`Nothing called ${doing}.`);
}
await page("DELETE", "/api/session", undefined, cookie);
