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
//   bun .yardsort/tests/support/phone.ts <run dir> lets-a-test-run <name>
//   bun .yardsort/tests/support/phone.ts <run dir> sees-pull-requests <name>
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
// `lets-a-test-run` reads the card of the smoke test run waiting in the project's own checkout,
// is refused letting another file's hash run, lets the one shown run, and leaves the run's id in
// the run's folder as `test-run`. `sees-pull-requests` asks the computer for its pull requests
// until it has read them, and checks the one from the project's checkout: its badge's facts, its
// checks by name with the failed one first, and that one it never read is not found.

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
  case "lets-a-test-run": {
    // What a phone that may type does with a smoke test waiting for its person.
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    if (!channel.host.capabilities.includes("test.waiting")) {
      throw new Error("The computer does not say it answers test.waiting.");
    }
    const listed = await channel.workspaces();
    const project = listed.projects.find((p) => p.name === "repo");
    const workspace = listed.workspaces.find((w) => w.project === project?.id);
    if (!workspace) throw new Error("The project's checkout is not on the phone.");
    let waiting = await channel.testsWaiting(workspace.id);
    for (let tries = 0; tries < 50 && !waiting.first; tries++) {
      await Bun.sleep(200);
      waiting = await channel.testsWaiting(workspace.id);
    }
    const card = waiting.first;
    if (!card) throw new Error("No smoke test waits there.");
    if (!card.whole) throw new Error(`The card is not whole: ${card.whyNot}`);
    if (!card.steps.some((step) => step.words.includes("let-go-from-the-phone"))) {
      throw new Error(`Its steps do not say what it runs: ${JSON.stringify(card.steps)}`);
    }
    console.log(`shown the card of "${card.name}", asked by ${card.askedBy}`);
    const decide = (contentHash: string) =>
      channel.decideTest({ workspace: workspace.id, run: card.run, contentHash, decision: "run" });
    const refused = await decide("0".repeat(64)).then(
      () => null,
      (error: unknown) => error,
    );
    if (!(refused instanceof ClientError) || refused.code !== "stale") {
      throw new Error(`Another file's hash was not refused as stale: ${String(refused)}`);
    }
    console.log("another file's hash is refused");
    await decide(card.contentHash);
    if ((await channel.testsWaiting(workspace.id)).first) throw new Error("It still waits.");
    writeFileSync(join(dir, "test-run"), card.run);
    console.log("let it run");
    channel.close();
    break;
  }
  case "sees-pull-requests": {
    // What a phone that reads workspaces is told of the computer's pull requests.
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    if (!channel.host.capabilities.includes("pull.list")) {
      throw new Error("The computer does not say it answers pull.list.");
    }
    const listed = await channel.workspaces();
    const project = listed.projects.find((p) => p.name === "repo");
    const workspace = listed.workspaces.find((w) => w.project === project?.id);
    if (!project || !workspace) throw new Error("The project's checkout is not on the phone.");

    // The computer never makes a phone wait for gh: the first answer says it is reading.
    let page = await channel.pulls();
    const first = page;
    for (let tries = 0; tries < 100 && (page.reading || page.pulls.length === 0); tries++) {
      await Bun.sleep(200);
      page = await channel.pulls();
    }
    if (first.pulls.length === 0 && !first.reading) {
      throw new Error("With nothing read yet, the computer did not say it was reading.");
    }
    const mine = page.pulls.filter((pull) => pull.workspaces.includes(workspace.id));
    if (mine.length !== 1) {
      throw new Error(`The checkout has ${mine.length} pull requests: ${JSON.stringify(page)}`);
    }
    const pull = mine[0];
    const facts = {
      number: pull.number,
      title: pull.title,
      state: pull.state,
      branch: pull.branch,
      base: pull.base,
      checks: pull.checks,
      review: pull.review,
      conflicts: pull.conflicts,
      url: pull.url,
    };
    const wanted = {
      number: 12,
      title: "Add an option",
      state: "open",
      branch: "ys/add-option",
      base: "trunk",
      checks: { passed: 1, failed: 1, running: 0 },
      review: "changesRequested",
      conflicts: true,
      url: "https://github.com/orchard/demo/pull/12",
    };
    if (JSON.stringify(facts) !== JSON.stringify(wanted)) {
      throw new Error(`Not what gh said: ${JSON.stringify(facts)}`);
    }
    // One nobody's workspace opened is not on a workspace list, and is among the open ones.
    if (page.pulls.some((other) => other.number === 9)) {
      throw new Error("A pull request no workspace opened is on the workspace list.");
    }
    if (page.open !== 2) throw new Error(`${page.open} open, not 2.`);
    console.log(`the checkout's pull request #${pull.number}, and ${page.open} open`);

    let open = await channel.pulls({ of: "open" });
    for (let tries = 0; tries < 50 && open.reading; tries++) {
      await Bun.sleep(200);
      open = await channel.pulls({ of: "open" });
    }
    const numbers = open.pulls.map((other) => other.number);
    if (JSON.stringify(numbers) !== "[12,9]") {
      throw new Error(`The open ones, most recently changed first: ${JSON.stringify(numbers)}`);
    }

    const detail = await channel.pull({ project: project.id, number: 12 });
    const checks = detail.checks.map((check) => `${check.name}: ${check.state}`);
    if (JSON.stringify(checks) !== JSON.stringify(["CI / test: failed", "CI / build: passed"])) {
      throw new Error(`Its checks, the failed one first: ${JSON.stringify(checks)}`);
    }
    if (JSON.stringify(detail.reviews) !== '[{"login":"mira-k","state":"changesRequested"}]') {
      throw new Error(`Its reviewers: ${JSON.stringify(detail.reviews)}`);
    }
    console.log(`its checks by name: ${checks.join(", ")}`);

    const unread = await channel.pull({ project: project.id, number: 999 }).then(
      () => null,
      (error: unknown) => error,
    );
    if (!(unread instanceof ClientError) || unread.code !== "not_found") {
      throw new Error(`One it never read was not refused as not found: ${String(unread)}`);
    }
    console.log("saw the pull requests");
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
  case "cannot-read": {
    // A phone let only watch: the changes and the files of a workspace are not its to read.
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const workspace = (await channel.workspaces()).workspaces.find((w) => w.name === "smoke");
    if (!workspace) throw new Error("The workspace is not on the phone.");
    const refusal = (attempt: Promise<unknown>) =>
      attempt.then(
        () => {
          throw new Error("It was let.");
        },
        (error: { code?: string }) => {
          if (error.code !== "forbidden") throw error;
        },
      );
    await refusal(channel.changes(workspace.id));
    await refusal(channel.files({ workspace: workspace.id, dir: "" }));
    await refusal(
      channel.readFile({ workspace: workspace.id, path: "README.md", of: "working", offset: 0 }),
    );
    console.log("refused: reading its changes and its files");
    channel.close();
    break;
  }
  case "cannot-publish": {
    // A phone let only watch: nothing is committed, pushed or opened for it.
    const phone = await session();
    const channel = await reach(phone, await theComputer(phone));
    const workspace = (await channel.workspaces()).workspaces.find((w) => w.name === "smoke");
    if (!workspace) throw new Error("The workspace is not on the phone.");
    const refusal = (attempt: Promise<unknown>) =>
      attempt.then(
        () => {
          throw new Error("It was let.");
        },
        (error: { code?: string }) => {
          if (error.code !== "forbidden") throw error;
        },
      );
    await refusal(channel.publishState({ workspace: workspace.id }));
    await refusal(channel.commit({ workspace: workspace.id, message: "Not committed" }));
    await refusal(channel.push(workspace.id));
    await refusal(channel.openPullRequest({ workspace: workspace.id, title: "Not opened" }));
    console.log("refused: committing, pushing and its pull request");
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
    // The window has control of an agent it started, so the phone takes it over, as a person
    // with both would; with nobody in control, this is an ordinary take.
    const control = await channel.acquire(terminal, { takeOver: true });
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
