// A pretend phone against the stand-in server, for a smoke test with no person at it, and the other
// copy of the app that gets a computer signed out.
//
//   bun .yardsort/tests/support/phone.ts <run dir> ask | reaches <name> | must-pair-again <name> | spend-sign-in
//     | starts <name>
//   bun .yardsort/tests/support/phone.ts <run dir> type | take-over <name> <line> <expected>
//   bun .yardsort/tests/support/phone.ts <run dir> sees <name> <expected>
//   bun .yardsort/tests/support/phone.ts <run dir> refused <name> | ends <name>
//   bun .yardsort/tests/support/phone.ts <run dir> resumes <name> <message> <expected>
//   bun .yardsort/tests/support/phone.ts <run dir> registers | opens-notice <name> <pushes file>
//
// `type` takes control of the one session running on the computer, types the line and a return,
// and waits for the expected text in what the terminal prints. `take-over` does the same while
// another device has control, and keeps it. `sees` only watches, for the expected text.
// `refused` expects a phone let only watch to be refused starting and resuming. `ends` types an
// end of input into the one running session, which ends a stand-in agent. `resumes` resumes the
// one ended conversation the computer can resume, with a message, and waits for the expected text.
// `registers` gives the server a push token, as a phone allowed to notify does. `opens-notice`
// takes the one push the stand-in recorded, reads its notice, and checks it names the terminal of
// the one session running on the computer, and that the push itself names nothing.

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

const [dir, doing, named, line, expected] = process.argv.slice(2);
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

/** The terminal of the one session running on the computer. */
async function theRunningTerminal(channel: Channel) {
  const page = await channel.workspaces();
  const running = [];
  for (const workspace of page.workspaces.filter((w) => w.runningSessions > 0)) {
    const sessions = await channel.sessions({ workspace: workspace.id });
    for (const s of sessions.sessions) if (s.process === "running" && s.terminal) running.push(s);
  }
  if (running.length !== 1) throw new Error(`${running.length} sessions running, not one.`);
  return running[0].terminal!.id;
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
  case "starts": {
    // What a phone that may type does: start a workspace with an agent, watch it, give it a picture.
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const offered = await channel.harnesses();
    if (!offered.some((harness) => harness.id === "sh-agent")) {
      throw new Error(`Offered only: ${offered.map((harness) => harness.id).join(", ")}`);
    }
    console.log("offered the smoke agent");
    const project = (await channel.workspaces()).projects.find((p) => p.name === "repo");
    if (!project) throw new Error("The project is not on the phone.");
    const started = await channel.startWorkspace({
      project: project.id,
      harness: "sh-agent",
      prompt: "echo started-from-the-phone; sleep 60",
    });
    console.log(`started ${started.harness}`);
    let seen = "";
    const decoder = new TextDecoder();
    const watching = await channel.attach(started.terminal, {
      output: (bytes) => (seen += decoder.decode(bytes, { stream: true })),
    });
    for (let tries = 0; tries < 100 && !seen.includes("started-from-the-phone"); tries++) {
      await Bun.sleep(100);
    }
    if (!seen.includes("started-from-the-phone")) throw new Error(`It printed: ${seen}`);
    console.log("the agent printed what it was told to");
    await watching.detach();

    const photo = Uint8Array.from({ length: 100_000 }, (_, n) => n % 251);
    const path = await channel.upload(started.workspace, "smoke photo.jpg", photo);
    if (!Buffer.from(photo).equals(readFileSync(path))) throw new Error("The picture changed.");
    if (path.startsWith(join(dir, "worktrees"))) throw new Error(`It is in a worktree: ${path}`);
    console.log("the picture is on the computer, whole, outside every worktree");
    channel.close();
    break;
  }
  case "sees": {
    // Here `line` is what to see.
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const terminal = await theRunningTerminal(channel);
    let printed = "";
    const decoder = new TextDecoder();
    const attachment = await channel.attach(terminal, {
      output: (bytes) => (printed += decoder.decode(bytes, { stream: true })),
    });
    for (let tries = 0; tries < 300 && !printed.includes(line); tries++) await Bun.sleep(100);
    if (!printed.includes(line)) throw new Error(`Not on the screen: ${line}\n${printed}`);
    console.log(`saw: ${line}`);
    await attachment.detach();
    channel.close();
    break;
  }
  case "type":
  case "take-over": {
    const phone = await session();
    const host = await theComputer(phone);
    const channel = await reach(phone, host);
    const running = [{ terminal: { id: await theRunningTerminal(channel) } }];
    let printed = "";
    let typedAt = -1;
    let seen!: () => void;
    const appeared = new Promise<void>((resolve) => (seen = resolve));
    const decoder = new TextDecoder();
    const attachment = await channel.attach(running[0].terminal!.id, {
      output(bytes) {
        printed += decoder.decode(bytes, { stream: true });
        // Only what the program printed after the line was typed counts.
        if (typedAt >= 0 && printed.slice(typedAt).includes(expected)) seen();
      },
    });
    const control = await channel.acquire(running[0].terminal!.id, {
      takeOver: doing === "take-over",
    });
    typedAt = printed.length;
    await control.input(`${line}\r`);
    const late = setTimeout(() => {
      console.log(printed.slice(typedAt));
      console.error(`Not printed within 30 seconds: ${expected}`);
      process.exit(1);
    }, 30_000);
    await appeared;
    clearTimeout(late);
    console.log(`read back: ${expected}`);
    // Taken over, it is kept: the other device is to see who has it.
    if (doing === "type") await control.release();
    await attachment.detach();
    channel.close();
    break;
  }
  case "refused": {
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const project = (await channel.workspaces()).projects.find((p) => p.name === "repo");
    if (!project) throw new Error("The project is not on the phone.");
    const refusal = (attempt: Promise<unknown>) =>
      attempt.then(
        () => {
          throw new Error("It was let.");
        },
        (error: { code?: string }) => {
          if (error.code !== "forbidden") throw error;
        },
      );
    await refusal(channel.startWorkspace({ project: project.id, prompt: "echo not-started" }));
    console.log("refused: starting");
    // The scope is checked before the conversation is looked for.
    await refusal(channel.resumeSession({ session: "any-at-all" as never }));
    console.log("refused: resuming");
    channel.close();
    break;
  }
  case "ends": {
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const terminal = await theRunningTerminal(channel);
    let ended!: () => void;
    const exited = new Promise<void>((resolve) => (ended = resolve));
    const attachment = await channel.attach(terminal, { output() {}, exited: () => ended() });
    const control = await channel.acquire(terminal, { takeOver: false });
    // An end of input, as Ctrl-D at a terminal: a stand-in agent stops reading, and ends.
    await control.input("\x04");
    const late = setTimeout(() => {
      console.error("It did not end within 30 seconds.");
      process.exit(1);
    }, 30_000);
    await exited;
    clearTimeout(late);
    console.log("the agent ended");
    await attachment.detach().catch(() => {});
    channel.close();
    break;
  }
  case "resumes": {
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const ended = [];
    for (const workspace of (await channel.workspaces()).workspaces) {
      const page = await channel.sessions({ workspace: workspace.id, includeEnded: true });
      ended.push(...page.sessions.filter((s) => s.resumable));
    }
    if (ended.length !== 1) throw new Error(`${ended.length} to resume, not one.`);
    console.log("one conversation to resume");
    const started = await channel.resumeSession({ session: ended[0].id, prompt: line });
    let printed = "";
    const decoder = new TextDecoder();
    const attachment = await channel.attach(started.terminal, {
      output: (bytes) => (printed += decoder.decode(bytes, { stream: true })),
    });
    for (let tries = 0; tries < 300 && !printed.includes(expected); tries++) await Bun.sleep(100);
    if (!printed.includes(expected)) throw new Error(`Not on the screen: ${expected}\n${printed}`);
    console.log(`saw: ${expected}`);
    await attachment.detach();
    channel.close();
    break;
  }
  case "registers": {
    const phone = await session();
    await client.registerPushToken(await phone.access(), {
      platform: "apns",
      token: "smoke-phone-token",
    });
    console.log("registered for notifications");
    break;
  }
  case "opens-notice": {
    // Here `line` is the file of pushes the stand-in recorded.
    const pushes = readFileSync(line, "utf8").trim().split("\n").filter(Boolean);
    if (pushes.length !== 1)
      throw new Error(`${pushes.length} pushes, not one:\n${pushes.join("\n")}`);
    const alert = JSON.parse(pushes[0]);
    if (Object.keys(alert).sort().join() !== "body,category,deepLink,title") {
      throw new Error(`The push says more than it should: ${pushes[0]}`);
    }
    if (alert.title !== "An agent needs you" || alert.category !== "agent") {
      throw new Error(`Not an agent's notice: ${pushes[0]}`);
    }
    const id = /^yardsort:\/\/notice\/agent\/([0-9a-f-]{36})$/.exec(alert.deepLink)?.[1];
    if (!id) throw new Error(`Not a notice's link: ${alert.deepLink}`);
    const phone = await session();
    const host = await theComputer(phone);
    const notice = await client.notice(await phone.access(), id);
    const channel = await reach(phone, host);
    const terminal = await theRunningTerminal(channel);
    channel.close();
    if (notice.hostId !== host || notice.terminalId !== terminal || !notice.workspaceId) {
      throw new Error(`It opens somewhere else: ${JSON.stringify(notice)}`);
    }
    console.log("the notice opens the terminal that asked");
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
