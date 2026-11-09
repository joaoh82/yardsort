// Drive the app's own window through WebKit's inspector, for a smoke test that has no person at it.
// The app is started with WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:<port>.
//
//   bun .yardsort/tests/support/window.ts <port> <what to do> [run dir | what]

const [port, doing, runDir] = process.argv.slice(2);
const SECTION = '[aria-label="Terminals cannot be shared yet"]';
const WAIT_MS = 30_000;

function connect(): Promise<(expression: string) => Promise<unknown>> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/socket/1/1/WebPage`);
    const waiting = new Map<number, (value: unknown) => void>();
    let target = "";
    let next = 1;
    socket.onerror = () => reject(new Error("The window's inspector did not answer."));
    socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      if (message.method === "Target.targetCreated" && message.params.targetInfo.type === "page") {
        target = message.params.targetInfo.targetId;
        resolve(
          (expression) =>
            new Promise((done) => {
              const id = next++;
              waiting.set(id, done);
              const inner = {
                id,
                method: "Runtime.evaluate",
                params: { expression, returnByValue: true, awaitPromise: true },
              };
              socket.send(
                JSON.stringify({
                  id: next++,
                  method: "Target.sendMessageToTarget",
                  params: { targetId: target, message: JSON.stringify(inner) },
                }),
              );
            }),
        );
      }
      if (message.method === "Target.dispatchMessageFromTarget") {
        const inner = JSON.parse(message.params.message);
        const done = waiting.get(inner.id);
        if (done) done(inner.result?.result?.value);
      }
    };
  });
}

async function until<T>(what: string, look: () => Promise<T>): Promise<T> {
  const deadline = Date.now() + WAIT_MS;
  for (;;) {
    const found = await look();
    if (found) return found;
    if (Date.now() > deadline) throw new Error(`Never saw: ${what}`);
    await Bun.sleep(200);
  }
}

// A socket that opens before the page is listed hears nothing, then or later.
const listed = () =>
  fetch(`http://127.0.0.1:${port}/`)
    .then((answer) => answer.text())
    .then((list) => list.includes("/socket/1/1/WebPage"))
    .catch(() => false);
await until("the window's page", listed);
const page = await until("the inspector", () => connect().catch(() => null));
const clicked = (text: string) => `(() => {
  const button = [...document.querySelectorAll("button,[role=tab]")].find(
    (b) => b.textContent.trim() === ${JSON.stringify(text)},
  );
  button?.click();
  return Boolean(button);
})()`;
const click = (text: string) => until(`a button "${text}"`, () => page(clicked(text)));
const notice = () =>
  page(`document.querySelector(${JSON.stringify(SECTION)})?.textContent ?? ""`) as Promise<string>;
const expectIn = async (wanted: string) => {
  const text = await until(`"${wanted}" in the notice`, async () => {
    const said = await notice();
    return said.includes(wanted) ? said : "";
  });
  console.log(`saw: ${wanted}`);
  return text;
};
const openRemote = async () => {
  await until("the app", () => page(`Boolean(document.querySelector("button"))`));
  await page(clicked("Skip tour"));
  await page(clicked("Not now"));
  await click("Shortcuts");
  await click("Remote");
  await until("Settings → Remote", () =>
    page(`document.body.textContent.includes("Remote is still under development")`),
  );
};

const everything = () => page(`document.body.textContent`) as Promise<string>;
const expectToSee = async (wanted: string) => {
  await until(`"${wanted}"`, async () => (await everything()).includes(wanted));
  console.log(`saw: ${wanted}`);
};
const fill = (placeholder: string, value: string) =>
  page(`(() => {
    const input = document.querySelector(${JSON.stringify(`input[placeholder="${placeholder}"]`)});
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, ${JSON.stringify(value)});
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return Boolean(input);
  })()`);
const connected = () => expectToSee("Connected to the relay.");
/** `fill`, within Settings → Remote → Other computers, whose fields look like the host's. */
const fillOther = (placeholder: string, value: string) =>
  page(`(() => {
    const input = document.querySelector(${JSON.stringify(`section[aria-label="Other computers"] input[placeholder="${placeholder}"]`)});
    if (!input) return false;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, ${JSON.stringify(value)});
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
/** The text of what matches `selector`, or "" when nothing does. */
const textOf = (selector: string) =>
  page(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? ""`) as Promise<string>;

switch (doing) {
  case "add-project": {
    // As adding a folder in the window does, without the system's folder picker. The inspector
    // does not wait for a promise, so its outcome is left on the window and looked for.
    await page(
      `void window.__TAURI_INTERNALS__.invoke("project_open", { path: ${JSON.stringify(`${runDir}/repo`)}, initGit: false }).then(() => { window.__added = "added"; }, (e) => { window.__added = JSON.stringify(e); })`,
    );
    const added = await until("the project added", () => page(`window.__added ?? ""`));
    if (added !== "added") throw new Error(`The project was not added: ${String(added)}`);
    console.log("added the project");
    break;
  }
  case "sees":
    // Here the last argument is what to look for.
    await expectToSee(runDir as string);
    break;
  case "says-why":
    await openRemote();
    await expectIn("A phone cannot open this computer's terminals yet.");
    await expectIn("new ones too");
    await expectIn("from Yardsort 0.20.2");
    break;
  case "a-no-stops-nothing":
    await click("Stop everything and restart…");
    await expectIn("This stops 1 agent and 1 shell");
    await click("Keep them running");
    await until("the question gone", async () => !(await notice()).includes("This stops"));
    console.log("saw: the question gone, nothing stopped");
    break;
  case "replace":
    await click("Stop everything and restart…");
    await click("Stop them and restart");
    console.log("asked to stop them and restart");
    break;
  case "says-nothing-now":
    await openRemote();
    if (await notice()) throw new Error("The notice is still there.");
    console.log("saw: Settings → Remote, with no notice");
    break;
  case "turn-on": {
    const stood = await Bun.file(`${runDir}/stand-in.json`).text();
    await openRemote();
    await fill("https://remote.example.com", JSON.parse(stood.split("\n")[0]).ready.server);
    await fill("Workstation", "Smoke computer");
    await click("Turn on and sign in");
    await connected();
    break;
  }
  case "pairing-code": {
    await click("Pair a device");
    const code = await until("a pairing code", () =>
      page(`document.querySelector('[aria-label="Pairing code"]')?.textContent`),
    );
    await Bun.write(`${runDir}/pairing-code`, String(code));
    console.log("saw: a pairing code");
    break;
  }
  case "let-it-in":
    await click("Let it watch and type");
    await expectToSee("Smoke test phone");
    await until("the question gone", async () => !(await everything()).includes("Waiting for you"));
    console.log("saw: the phone let in");
    break;
  case "says-signed-out": {
    await openRemote();
    await expectToSee("This computer was signed out at the server.");
    await expectToSee("Sign in again");
    if ((await everything()).includes("off, then on")) throw new Error("It still says off and on.");
    console.log("saw: no advice to turn it off and on");
    break;
  }
  case "sign-in-again":
    await click("Sign in again");
    await connected();
    if ((await everything()).includes("None can reach this computer"))
      throw new Error("It says no device can reach it.");
    await expectToSee("Smoke test phone");
    break;
  case "turn-off":
    await click("Turn off");
    await expectToSee("Turn on and sign in");
    break;
  case "turn-on-again":
    await click("Turn on and sign in");
    await connected();
    await expectToSee("None yet.");
    break;
  case "client-sign-in": {
    const stood = await Bun.file(`${runDir}/stand-in.json`).text();
    await openRemote();
    const server = JSON.parse(stood.split("\n")[0]).ready.server;
    await until("the client's server field", () => fillOther("https://remote.example.com", server));
    await until("the client's name field", () => fillOther("Laptop", "Smoke laptop"));
    await click("Sign in to reach other computers");
    await expectToSee("Signed in as Smoke laptop");
    break;
  }
  case "enter-code": {
    const code = (await Bun.file(`${runDir}/pairing-code`).text()).trim();
    await openRemote();
    await until("the pairing code field", () => fillOther("ABCD-EFGH", code));
    await click("Pair");
    await expectToSee("waiting for its answer");
    break;
  }
  case "sees-the-host": {
    // Here the last argument is the host's name.
    const name = runDir as string;
    const row = `aside[aria-label="Projects"] section[aria-label="Other computers"]`;
    await until(`${name} online in the sidebar`, async () => {
      const said = await textOf(row);
      return said.includes(name) && said.includes("online");
    });
    console.log(`saw: ${name}, online, under Other computers`);
    await page(clicked("Done"));
    await page(`document.querySelector(${JSON.stringify(`${row} button`)})?.click()`);
    const view = `section[aria-label="${name}, another computer"]`;
    await until("its page", async () =>
      (await textOf(view)).includes(`${name} · another computer`),
    );
    console.log(`saw: ${name} named across the top`);
    await until("its workspace and its shell", async () => {
      const said = await textOf(view);
      return said.includes("smoke") && said.includes("sh");
    });
    console.log("saw: the workspace smoke and its shell");
    const buttons = await page(
      `[...document.querySelectorAll(${JSON.stringify(`${view} button`)})].map((b) => b.textContent.trim()).filter((t) => t !== "Watch").join(",")`,
    );
    if (buttons !== "Close")
      throw new Error(`It offers more than Close and Watch: ${String(buttons)}`);
    console.log("saw: nothing to press but Close and Watch");
    const changes = await textOf(`[data-navigation="Changes"]`);
    if (!changes.includes("Changes and files are not shown for another computer"))
      throw new Error(`The Changes panel shows: ${changes}`);
    console.log("saw: no Changes panel for it");
    break;
  }
  case "watches": {
    // Here the last argument is the host's name. Its one shell opens, watched, at its size there.
    const name = runDir as string;
    const row = `aside[aria-label="Projects"] section[aria-label="Other computers"]`;
    await until(`${name} online in the sidebar`, async () =>
      (await textOf(row)).includes("online"),
    );
    await page(clicked("Done"));
    await page(`document.querySelector(${JSON.stringify(`${row} button`)})?.click()`);
    const view = `section[aria-label="${name}, another computer"]`;
    try {
      await until("its shell, to watch", () => page(clicked("Watch")));
    } catch (error) {
      console.log(`its page said: ${await textOf(view)}`);
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    await until("the terminal", () => page(`Boolean(document.querySelector(".xterm"))`));
    console.log(`saw: the shell on ${name}, watched`);
    await until("who has control", async () =>
      /has control\./.test(await textOf(`[aria-label="Control"]`)),
    );
    console.log(`saw: ${await textOf(`[aria-label="Control"]`)}`);
    if (await page(clicked("Release control")))
      throw new Error("It had control before it was asked for.");
    break;
  }
  case "takes-control-and-types": {
    // Here the last argument is the line to type.
    const line = runDir as string;
    await click("Take control");
    await until("control", async () =>
      (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
    );
    console.log("saw: This computer has control.");
    // As the keyboard does: xterm reads its hidden field.
    const typed = await page(`(() => {
      const field = document.querySelector(".xterm-helper-textarea");
      if (!field) return false;
      field.focus();
      field.dispatchEvent(new InputEvent("input", { data: ${JSON.stringify(`${line}
`)}, inputType: "insertText", bubbles: true }));
      return true;
    })()`);
    if (!typed) throw new Error("No terminal to type into.");
    console.log(`typed: ${line}`);
    break;
  }
  case "lost-control": {
    // Here the last argument is who took it.
    const who = runDir as string;
    await expectToSee(`${who} took control.`);
    // The phone's step has ended by now, and its control with it, so either may be said.
    const status = await until("who has it", async () => {
      const said = await textOf(`[aria-label="Control"]`);
      return said.includes(`${who} has control.`) || said.includes("Nobody has control.")
        ? said
        : "";
    });
    console.log(`saw: ${status}`);
    await until("Take control offered again", () =>
      page(
        `[...document.querySelectorAll("button")].some((b) => b.textContent.trim() === "Take control")`,
      ),
    );
    console.log("saw: Take control, offered again");
    break;
  }
  default:
    throw new Error(`Nothing called ${doing}.`);
}
process.exit(0);
