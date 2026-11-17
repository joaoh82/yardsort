// The operator of a computer with no screen, opening its dashboard, for a smoke test with no
// person at it: asks `yardsort-host dashboard` for a sign-in link at a terminal of their own,
// signs a browser's worth of requests in with it, and has the dashboard run a command.
//
//   bun .yardsort/tests/support/dashboard.ts <run dir> <port>
//
// `yardsort-host` runs through headless.sh: the build headless-build.sh made, on this run's profile.

const [, port] = process.argv.slice(2);
const HOST = "sh .yardsort/tests/support/headless.sh yardsort-host";
const address = `http://127.0.0.1:${port}`;

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
const said = asked.stdout.toString();
const link = said.match(/http:\/\/127\.0\.0\.1:\d+\/#code=([0-9A-Z-]+)/);
if (!link) fail(`No sign-in link in what the host said:\n${said}${asked.stderr.toString()}`);
if (!link[0].startsWith(`${address}/#code=`)) fail(`The link is not for ${address}: ${link[0]}`);
console.log("a sign-in link, at a terminal of the operator's own");

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
if (signedIn.status !== 200) fail(`Signing in answered ${signedIn.status}.`);
const cookie = (signedIn.headers.get("set-cookie") ?? "").split(";")[0];
if (!cookie.startsWith("yardsort_dashboard_")) fail("Signing in set no session cookie.");
const again = await page("POST", "/api/session", { code: link[1] });
if (again.status !== 401) fail(`The same code a second time answered ${again.status}.`);
console.log("signed in once; the code is spent");

const status = await (await page("GET", "/api/status", undefined, cookie)).json();
if (status.host?.program !== "yardsort-host") fail(`Not the host: ${JSON.stringify(status.host)}`);
console.log(`the host: ${status.host.program} ${status.host.version}, ${status.dashboard.address}`);

const job = await (
  await page("POST", "/api/commands", { command: "echo from-the-dashboard-$((6*7))" }, cookie)
).json();
for (let tries = 0; ; tries++) {
  const reading = await (await page("GET", `/api/jobs/${job.id}?from=0`, undefined, cookie)).json();
  if (reading.state !== "running") {
    if (reading.state !== "succeeded" || !reading.output.includes("from-the-dashboard-42"))
      fail(`The command ${reading.state}: ${reading.output}`);
    console.log("ran: from-the-dashboard-42");
    break;
  }
  if (tries > 200) fail("The command never ended.");
  await Bun.sleep(50);
}

const out = await page("DELETE", "/api/session", undefined, cookie);
const after = await page("GET", "/api/status", undefined, cookie);
if (out.status !== 200 || after.status !== 401) fail("Signing out left the session in force.");
console.log("signed out");
