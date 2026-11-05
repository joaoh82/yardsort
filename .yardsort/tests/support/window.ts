// Drive the app's own window through WebKit's inspector, for a smoke test that has no person at it.
// The app is started with WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:<port>.
//
//   bun .yardsort/tests/support/window.ts <port> <what to do>

const [port, doing] = process.argv.slice(2);
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

switch (doing) {
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
  default:
    throw new Error(`Nothing called ${doing}.`);
}
process.exit(0);
