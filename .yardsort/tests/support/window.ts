// Drive the app's own window through WebKit's inspector, for a smoke test that has no person at it.
// The app is started with WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:<port>.
//
//   bun .yardsort/tests/support/window.ts <port> <what to do> [run dir | what] [more]

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

async function until<T>(what: string, look: () => Promise<T>, waitMs = WAIT_MS): Promise<T> {
  const deadline = Date.now() + waitMs;
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
/** Click the button whose accessible name begins with `label`. */
const clickedNamed = (label: string) => `(() => {
  const button = [...document.querySelectorAll("button")].find(
    (b) => (b.getAttribute("aria-label") ?? "").startsWith(${JSON.stringify(label)}),
  );
  button?.click();
  return Boolean(button);
})()`;
/** Set a form field as typing does, so React hears it. */
const setField = (selector: string, value: string) =>
  page(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    if (!field) return false;
    const kind =
      field instanceof HTMLSelectElement
        ? HTMLSelectElement
        : field instanceof HTMLInputElement
          ? HTMLInputElement
          : HTMLTextAreaElement;
    Object.getOwnPropertyDescriptor(kind.prototype, "value").set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event(field instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
    return true;
  })()`);
/** The other computers in the sidebar: a tree, each computer with its projects under it. */
const TREE = `aside[aria-label="Projects"] [role="tree"][aria-label="Other computers"]`;
/**
 * Wait for the other computer called `name` to be online in the sidebar, and leave Settings. Its
 * row says `online`, or `watch only` when this computer is let watch there and not type.
 */
const onlineThere = async (name: string) => {
  await until(`${name} online in the sidebar`, async () =>
    /online|watch only/.test(await textOf(`${TREE} [role="treeitem"][aria-label="${name}"] > div`)),
  );
  await page(clicked("Done"));
};
/** Open the workspace called `workspace` under the other computer called `name`, as a click does. */
const openThere = async (name: string, workspace: string) => {
  await onlineThere(name);
  const row = `${TREE} [role="treeitem"][aria-label="${name}"] [role="treeitem"][aria-label="${workspace}"] button`;
  try {
    await until(`${workspace} under ${name}`, () =>
      page(`(() => {
        const button = document.querySelector(${JSON.stringify(row)});
        button?.click();
        return Boolean(button);
      })()`),
    );
    await until(
      "the workspace there",
      async () => (await textOf(viewThere(name, workspace))) !== "",
    );
  } catch (error) {
    console.log(`the sidebar said: ${await textOf(TREE)}`);
    throw error;
  }
};
/** The composer for a new workspace on this computer. */
const HERE = `form[aria-label="New workspace"]`;
/** Open the composer for this computer's own project called `project`, as its + does. */
const composeHere = async (project: string) => {
  const plus = `aside[aria-label="Projects"] button[aria-label="New workspace in ${project}"]`;
  await until(`the + on ${project}`, () =>
    page(`(() => {
      const button = document.querySelector(${JSON.stringify(plus)});
      button?.click();
      return Boolean(button);
    })()`),
  );
  await until("this computer's composer", async () =>
    (await textOf(HERE)).includes(`New workspace in ${project} on`),
  );
};
/** In the composer's title, choose the other computer called `name`, which must be offered. */
const chooseThere = async (name: string) => {
  await until("the menu", () => page(clickedNamed("Run on: This computer")));
  const row = `[role="menu"][aria-label="Run on"] [role="menuitemradio"][aria-label="${name}, online"]`;
  await until(`${name}, offered in the menu`, () =>
    page(`(() => {
      const row = document.querySelector(${JSON.stringify(row)});
      if (!row || row.disabled) return false;
      row.click();
      return true;
    })()`),
  );
};
/** A workspace on another computer, in the center panel. */
const viewThere = (name: string, workspace: string) =>
  `section[aria-label="${workspace} on ${name}"]`;
const textOf = (selector: string) =>
  page(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? ""`) as Promise<string>;

switch (doing) {
  case "add-project": {
    // As adding a folder in the window does, without the system's folder picker. The inspector
    // does not wait for a promise, so its outcome is left on the window and looked for.
    await page(
      `void (window.__added = "", window.__TAURI_INTERNALS__.invoke("project_open", { path: ${JSON.stringify(`${runDir}/${process.argv[5] ?? "repo"}`)}, initGit: false }).then(() => { window.__added = "added"; }, (e) => { window.__added = JSON.stringify(e); }))`,
    );
    const added = await until("the project added", () => page(`window.__added ?? ""`));
    if (added !== "added") throw new Error(`The project was not added: ${String(added)}`);
    console.log("added the project");
    break;
  }
  case "load-again":
    // The window reads its projects when it loads, so one added past it, through the core, shows
    // once it is loaded again. Whatever looks next connects anew, to the page as it then is.
    void page(`location.reload()`);
    await Bun.sleep(1_000);
    console.log("asked the window to load again");
    break;
  case "shows-projects": {
    // Here the last arguments are the names of projects that must be in the sidebar.
    const names = process.argv.slice(4);
    await until(`${names.join(" and ")} in the sidebar`, async () => {
      const said = await textOf(`aside[aria-label="Projects"]`);
      return names.every((name) => said.includes(name));
    });
    console.log(`saw: ${names.join(" and ")} among this computer's projects`);
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
    // Here the last argument is the host's name. Its project and its workspace are under it in the
    // sidebar, and the workspace opens as one of this computer's does, with nothing that acts.
    const name = runDir as string;
    await onlineThere(name);
    // The host let it watch and no more, and its row says so once the host was read.
    await until(`${name}'s row saying watch only`, async () =>
      (await textOf(`${TREE} [role="treeitem"][aria-label="${name}"] > div`)).includes(
        "watch only",
      ),
    );
    console.log(`saw: ${name}, online and watch only, under Other computers`);
    await until("its project and its workspace under it", async () => {
      const said = await textOf(`${TREE} [role="treeitem"][aria-label="${name}"]`);
      return said.includes("repo") && said.includes("smoke");
    });
    console.log("saw: its project repo and its workspace smoke under it");
    await openThere(name, "smoke");
    const view = viewThere(name, "smoke");
    await until("the computer's bar", async () =>
      (await textOf(`${view} [role="group"][aria-label="${name}, another computer"]`)).includes(
        "online",
      ),
    );
    console.log(`saw: ${name} named across the top`);
    await until("its shell, as a tab", async () =>
      (await textOf(`${view} [role="tablist"]`)).includes("sh"),
    );
    console.log("saw: the workspace smoke and its shell");
    if (!(await textOf(`${view} footer`)).includes(name))
      throw new Error(`The bottom line does not name ${name}.`);
    console.log(`saw: ${name} named in the bottom line`);
    // Let in to watch only: nothing takes control, and nothing starts.
    const offered = await page(
      `[...document.querySelectorAll(${JSON.stringify(`${view} button`)})].filter((b) => !b.matches('[role="tab"]')).map((b) => b.getAttribute("aria-label") ?? b.textContent.trim()).join(",")`,
    );
    if (offered !== "") throw new Error(`It offers something that acts: ${String(offered)}`);
    console.log("saw: nothing to press but its tabs");
    const changes = await textOf(`[data-navigation="Changes"]`);
    if (!changes.includes(`Changes and files stay on ${name}`))
      throw new Error(`The Changes panel shows: ${changes}`);
    console.log("saw: no Changes panel for it");
    break;
  }
  case "watches": {
    // Here the last argument is the host's name. Its workspace opens from the sidebar with its one
    // shell in front, watched, at its size there.
    const name = runDir as string;
    await openThere(name, "smoke");
    try {
      await until("the terminal", () => page(`Boolean(document.querySelector(".xterm"))`));
    } catch (error) {
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    console.log(`saw: the shell on ${name}, watched`);
    await until("who has control", async () =>
      /has control\./.test(await textOf(`[aria-label="Control"]`)),
    );
    console.log(`saw: ${await textOf(`[aria-label="Control"]`)}`);
    // It was running there already: its control is never taken without a press.
    if (await page(clicked("Release"))) throw new Error("It had control before it was asked for.");
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
  case "starts-there": {
    // Here the last arguments are the host's name and what its agent is to do. A new workspace in
    // its one project, composed from the + on that project in the sidebar, with its stand-in
    // agent. Once started it is in view, and this computer has control of it without a press.
    const name = runDir as string;
    const message = process.argv[5] as string;
    await onlineThere(name);
    await until("New workspace, offered on its project", () =>
      page(clickedNamed("New workspace in repo on")),
    );
    const form = `form[aria-label="New workspace on ${name}"]`;
    await until("its agents", () =>
      page(`Boolean(document.querySelector('${form} option[value="stand-in"]'))`),
    );
    await setField(`${form} select[aria-label="Harness"]`, "stand-in");
    await setField(`${form} textarea`, message);
    await click("Start");
    try {
      await until("the started agent's terminal", () =>
        page(`Boolean(document.querySelector(".xterm"))`),
      );
      await until("control of what it started", async () =>
        (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
      );
    } catch (error) {
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    console.log("saw: This computer has control, without a press");
    console.log(`saw: a workspace started on ${name}, watched`);
    break;
  }
  case "resumes-there": {
    // Here the last argument is the host's name. The workspace it started is still in view; its
    // one ended conversation is resumed from there, and this computer has control of it at once.
    const name = runDir as string;
    try {
      await until("Resume, offered", () => page(clickedNamed("Resume ")));
      await until("control of what it resumed", async () =>
        (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
      );
    } catch (error) {
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    await until("the resumed agent's terminal", () =>
      page(`Boolean(document.querySelector(".xterm"))`),
    );
    console.log(`saw: a conversation resumed on ${name}, watched`);
    break;
  }
  case "runs-on": {
    // Here the last arguments are the host's name and what its agent is to do. From the composer
    // of this computer's own project repo: the host is chosen in its title, its project of the
    // same name is the one offered, what was typed comes along, and Start makes the workspace
    // there, under the host in the sidebar and this computer's to type into.
    const name = runDir as string;
    const message = process.argv[5] as string;
    await onlineThere(name);
    await composeHere("repo");
    await setField(`${HERE} textarea`, message);
    await chooseThere(name);
    const form = `form[aria-label="New workspace on ${name}"]`;
    await until("the composer for it", async () => (await textOf(form)) !== "");
    const typed = await page(`document.querySelector(${JSON.stringify(`${form} textarea`)}).value`);
    if (typed !== message) throw new Error(`What was typed did not come along: ${String(typed)}`);
    console.log("saw: what was typed, in the composer for it");
    const offered = await page(
      `(() => { const field = document.querySelector(${JSON.stringify(`${form} select:not([aria-label])`)}); return field?.selectedOptions[0]?.textContent ?? ""; })()`,
    );
    if (offered !== "repo") throw new Error(`The project offered there is: ${String(offered)}`);
    if (!(await textOf(form)).includes("same name as here"))
      throw new Error(`It does not say why repo is offered: ${await textOf(form)}`);
    console.log(`saw: Project on ${name} is repo, same name as here`);
    await until("its agents", () =>
      page(`Boolean(document.querySelector('${form} option[value="stand-in"]'))`),
    );
    await setField(`${form} select[aria-label="Harness"]`, "stand-in");
    await click("Start");
    try {
      await until("the started agent's terminal", () =>
        page(`Boolean(document.querySelector(".xterm"))`),
      );
      await until("control of what it started", async () =>
        (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
      );
      await until("the new workspace under it in the sidebar", () =>
        page(
          `Boolean(document.querySelector(${JSON.stringify(`${TREE} [role="treeitem"][aria-label="${name}"] [role="treeitem"][aria-label="repo"] [role="treeitem"][aria-selected="true"]`)}))`,
        ),
      );
    } catch (error) {
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    console.log(`saw: a workspace started on ${name} from this computer's composer`);
    break;
  }
  case "missing-there": {
    // Here the last argument is the host's name. This computer's project notes has no namesake
    // there: the composer says so, and Start waits.
    const name = runDir as string;
    await composeHere("notes");
    await chooseThere(name);
    const form = `form[aria-label="New workspace on ${name}"]`;
    await until("its project said to be missing", async () =>
      (await textOf(`${form} [role="alert"]`)).includes(`${name} has no project called notes`),
    );
    await setField(`${form} textarea`, "nothing to start");
    const waits = await page(
      `[...document.querySelectorAll(${JSON.stringify(`${form} button[type="submit"]`)})].every((b) => b.disabled)`,
    );
    if (!waits) throw new Error("Start is offered with no project there.");
    console.log(`saw: ${name} has no project called notes, and Start waits`);
    break;
  }
  case "not-offered-away": {
    // Here the last argument is the host's name, which has just stopped. It is listed in the
    // composer's menu as offline, and cannot be chosen.
    const name = runDir as string;
    await until(
      `${name} offline in the sidebar`,
      async () =>
        (await textOf(`${TREE} [role="treeitem"][aria-label="${name}"] > div`)).includes("offline"),
      90_000,
    );
    await composeHere("repo");
    await until("the menu", () => page(clickedNamed("Run on: This computer")));
    const row = `[role="menu"][aria-label="Run on"] [role="menuitemradio"][aria-label^="${name}, offline"]`;
    const away = await until("its row in the menu", () =>
      page(
        `(() => { const row = document.querySelector(${JSON.stringify(row)}); return row ? (row.disabled ? "disabled" : "offered") : ""; })()`,
      ),
    );
    if (away !== "disabled") throw new Error(`${name} is away and still offered.`);
    console.log(`saw: ${name} listed as offline, and not offered`);
    break;
  }
  case "runs-on-saying": {
    // Here the last arguments are the host's name and what its agent is to do. As runs-on, with
    // what a host from 0.35.0 takes: a name, a model, an effort and the branch to start from,
    // each chosen from the host's own lists.
    const name = runDir as string;
    const message = process.argv[5] as string;
    await onlineThere(name);
    await composeHere("repo");
    await setField(`${HERE} textarea`, message);
    await chooseThere(name);
    const form = `form[aria-label="New workspace on ${name}"]`;
    await until("its branches", () =>
      page(
        `Boolean(document.querySelector(${JSON.stringify(`${form} select[aria-label="Branch"] option[value="new:develop"]`)}))`,
      ),
    );
    await until("its agents", () =>
      page(`Boolean(document.querySelector('${form} option[value="stand-in"]'))`),
    );
    await setField(`${form} select[aria-label="Harness"]`, "stand-in");
    await until("that agent's efforts", () =>
      page(
        `Boolean(document.querySelector(${JSON.stringify(`${form} select[aria-label="Effort"] option[value="high"]`)}))`,
      ),
    );
    await setField(`${form} input[placeholder^="named by"]`, "tidy");
    await setField(`${form} input[aria-label="Model"]`, "large");
    await setField(`${form} select[aria-label="Effort"]`, "high");
    await setField(`${form} select[aria-label="Branch"]`, "new:develop");
    console.log("saw: a name, a model, an effort and a branch to choose, from its own lists");
    await click("Start");
    try {
      await until("control of what it started", async () =>
        (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
      );
    } catch (error) {
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    console.log(`saw: a workspace started on ${name}, saying all four`);
    break;
  }
  case "clones-there":
  case "clone-fails-there": {
    // Here the last arguments are the host's name and this computer's project, which the host has
    // none for. The composer offers Clone it there, never by itself; chosen, Start has the host
    // clone the repository and makes the workspace in it. One the host cannot clone says so.
    const name = runDir as string;
    const project = process.argv[5] as string;
    await composeHere(project);
    await setField(`${HERE} textarea`, `in-the-clone-of-${project}`);
    await chooseThere(name);
    const form = `form[aria-label="New workspace on ${name}"]`;
    await until("Clone it there, offered", () => page(clicked("Clone it there")));
    await until("the clone chosen", async () =>
      (await textOf(form)).includes("projects folder when you start"),
    );
    await until("its agents", () =>
      page(`Boolean(document.querySelector('${form} option[value="stand-in"]'))`),
    );
    await setField(`${form} select[aria-label="Harness"]`, "stand-in");
    await click("Start");
    if (doing === "clone-fails-there") {
      await until("why it could not be cloned", async () =>
        (await textOf(`${form} [role="alert"]`)).includes("could not clone"),
      );
      const kept = await page(
        `document.querySelector(${JSON.stringify(`${form} textarea`)}).value`,
      );
      if (kept !== `in-the-clone-of-${project}`) throw new Error("The message was not kept.");
      console.log(`saw: ${name} could not clone ${project}, and the message is still here`);
      break;
    }
    try {
      await until("control of what it started", async () =>
        (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
      );
      await until("the clone under it in the sidebar", () =>
        page(
          `Boolean(document.querySelector(${JSON.stringify(`${TREE} [role="treeitem"][aria-label="${name}"] [role="treeitem"][aria-label="${project}"] [role="treeitem"][aria-selected="true"]`)}))`,
        ),
      );
    } catch (error) {
      console.log(`the window said: ${(await everything()).slice(0, 2000)}`);
      throw error;
    }
    console.log(`saw: ${project} cloned on ${name}, and a workspace started in it`);
    break;
  }
  case "shell-there": {
    // Here the last arguments are the host's name and one of its workspaces. Its + opens the
    // host's shell there as another tab, this computer's to type into at once.
    const name = runDir as string;
    const workspace = process.argv[5] as string;
    await openThere(name, workspace);
    const view = viewThere(name, workspace);
    const tabs = () =>
      page(`document.querySelectorAll(${JSON.stringify(`${view} [role="tab"]`)}).length`);
    const before = Number(await tabs());
    await until("New shell, offered", () => page(clickedNamed("New shell")));
    await until("another tab", async () => Number(await tabs()) > before, 60_000);
    await until("control of the shell", async () =>
      (await textOf(`[aria-label="Control"]`)).includes("This computer has control."),
    );
    console.log(`saw: a shell on ${name} in ${workspace}, with control`);
    break;
  }
  case "remove-there": {
    // Here the last arguments are the host's name, one of its workspaces, and `force` or nothing.
    // The window's questions are the system's own dialogs, which nothing here can press, so the
    // core is asked as the window asks it once the person has answered: without force first.
    const name = runDir as string;
    const workspace = process.argv[5] as string;
    const force = process.argv[6] === "force";
    const asked = await page(`(async () => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      const status = await invoke("other_computers");
      const host = status.hosts.find((h) => h.name === ${JSON.stringify(name)});
      const view = await invoke("other_host", { hostId: host.hostId });
      const found = view.projects.flatMap((p) => p.workspaces).find((w) => w.name === ${JSON.stringify(workspace)});
      if (!found) return "no such workspace";
      try {
        await invoke("other_host_remove_workspace", { hostId: host.hostId, workspaceId: found.id, remove: true, force: ${force} });
        return "deleted";
      } catch (error) {
        return "refused: " + error.code;
      }
    })()`);
    console.log(`saw: ${String(asked)}`);
    break;
  }
  default:
    throw new Error(`Nothing called ${doing}.`);
}
process.exit(0);
