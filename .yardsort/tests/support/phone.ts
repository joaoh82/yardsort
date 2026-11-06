// A pretend phone against the stand-in server, for a smoke test with no person at it, and the other
// copy of the app that gets a computer signed out.
//
//   bun .yardsort/tests/support/phone.ts <run dir> ask | reaches <name> | must-pair-again <name> | spend-sign-in

import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  Client,
  ClientError,
  codeFromRedirect,
  normalizeCode,
  openChannel,
  Secret,
  Session,
  type Channel,
  type HostId,
  type Scope,
} from "../../../packages/remote-client/src/index.ts";

const [dir, doing, named] = process.argv.slice(2);
const KEPT = join(dir, "phone-sign-in");
const HOST = join(dir, "host-id");
const SCOPES: Scope[] = ["workspace:read", "terminal:read", "terminal:control"];

const ready = JSON.parse(readFileSync(join(dir, "stand-in.json"), "utf8").split("\n")[0]);
const client = new Client(ready.ready.server);
const keep = (refresh: Secret) => writeFileSync(KEPT, refresh.expose());

async function signIn(): Promise<Session> {
  let returned!: (url: string) => void;
  const back = new Promise<string>((resolve) => (returned = resolve));
  const listener = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
      returned(request.url);
      return new Response("Signed in.");
    },
  });
  try {
    const started = await client.beginSignIn(`http://127.0.0.1:${listener.port}/signed-in`);
    await fetch(started.url);
    const issued = await client.signIn(codeFromRedirect(await back), started.verifier, {
      name: "Smoke test phone",
      kind: "phone",
      platform: "ios",
    });
    keep(issued.refresh);
    return new Session(client, issued, keep);
  } finally {
    listener.stop(true);
  }
}

const session = () => Session.resume(client, new Secret(readFileSync(KEPT, "utf8")), keep);

/** The one computer by this name. Two is the bug this test is here for. */
async function theComputer(phone: Session): Promise<HostId> {
  const page = await client.hosts(await phone.access());
  const hosts = page.hosts.filter((host) => host.name === named);
  if (hosts.length !== 1) throw new Error(`${hosts.length} computers called ${named}.`);
  const id = hosts[0].id;
  if (existsSync(HOST)) {
    if (readFileSync(HOST, "utf8") !== id) throw new Error(`${named} is a new host.`);
    console.log("the same computer as before, and only one");
  } else {
    writeFileSync(HOST, id);
    console.log("one computer");
  }
  return id;
}

const letsIn = async (phone: Session, host: HostId) =>
  (await client.grants(await phone.access())).grants.filter(
    (grant) => grant.host === host && grant.state === "approved",
  );

async function reach(phone: Session, host: HostId): Promise<Channel> {
  for (let tries = 0; ; tries++) {
    try {
      return await openChannel(client, await phone.access(), host, { client: "Smoke test phone" });
    } catch (error) {
      if (!(error instanceof ClientError) || error.code !== "host_offline" || tries >= 30)
        throw error;
      await Bun.sleep(1_000);
    }
  }
}

switch (doing) {
  case "ask": {
    const code = normalizeCode(readFileSync(join(dir, "pairing-code"), "utf8"));
    if (!code) throw new Error("No pairing code was shown.");
    const phone = await signIn();
    const asked = await client.pair(await phone.access(), {
      offer: { kind: "code", value: new Secret(code) },
      scopes: SCOPES,
    });
    console.log(`asked ${asked.hostName} to be let in`);
    break;
  }
  case "reaches": {
    const phone = await session();
    const host = await theComputer(phone);
    if ((await letsIn(phone, host)).length !== 1) throw new Error("It does not let this phone in.");
    console.log("still paired");
    const channel = await reach(phone, host);
    console.log(`welcomed by ${channel.host.name}`);
    break;
  }
  case "must-pair-again": {
    const phone = await session();
    const host = await theComputer(phone);
    if ((await letsIn(phone, host)).length !== 0) throw new Error("It still lets this phone in.");
    console.log("no longer paired");
    break;
  }
  case "spend-sign-in": {
    // What another copy of the app does: it trades in the sign-in they share, twice, so what this
    // copy kept is spent and past the one retry a server allows.
    const folder = join(dir, "credentials");
    const kept = readdirSync(folder);
    if (kept.length !== 1) throw new Error(`Expected one kept credential, found ${kept.length}.`);
    const first = await client.refresh(
      new Secret(readFileSync(join(folder, kept[0]), "utf8").trim()),
    );
    await client.refresh(first.refresh);
    console.log("spent the sign-in this computer kept");
    break;
  }
  default:
    throw new Error(`Nothing called ${doing}.`);
}
process.exit(0);
