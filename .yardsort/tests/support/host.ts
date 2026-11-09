// The operator of a computer with no screen, for a smoke test with no person at it: signs
// `yardsort-host` in by pasting the address a browser came back to, and pairs the pretend phone at
// a terminal of their own.
//
//   bun .yardsort/tests/support/host.ts <run dir> sign-in <name>
//   bun .yardsort/tests/support/host.ts <run dir> pair [watching | window <port> [v|t]]
//
// The pretend phone is let watch and type, or with `watching` only watch. With `window`, the app's
// window is the device that asks, through its inspector on `port`. It asks to watch and type, as
// the phone does, and is let watch (`v`, the default) or type too (`t`).
//
// `yardsort-host` runs through headless.sh: the build headless-build.sh made, on this run's profile.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, doing, named, port, answer] = process.argv.slice(2);
const HOST = ["sh", ".yardsort/tests/support/headless.sh", "yardsort-host"];
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

/** Everything a process prints, and a wait for something in it. */
function reading(stream: ReadableStream<Uint8Array>) {
  let text = "";
  const waiting: { want: RegExp; found: (m: RegExpMatchArray) => void }[] = [];
  (async () => {
    const decoder = new TextDecoder();
    for await (const chunk of stream) {
      text += decoder.decode(chunk, { stream: true });
      process.stdout.write(decoder.decode(chunk));
      for (const w of waiting.splice(0)) {
        const m = text.match(w.want);
        if (m) w.found(m);
        else waiting.push(w);
      }
    }
  })();
  return {
    until(want: RegExp, seconds = 60): Promise<RegExpMatchArray> {
      const m = text.match(want);
      if (m) return Promise.resolve(m);
      return new Promise((found, failed) => {
        waiting.push({ want, found });
        setTimeout(
          () => failed(new Error(`Not printed within ${seconds}s: ${want}`)),
          seconds * 1000,
        );
      });
    },
  };
}

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

switch (doing) {
  case "sign-in": {
    const signing = Bun.spawn([...HOST, "sign-in", server, "--name", named], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
      env: own(),
    });
    const out = reading(signing.stdout);
    const err = reading(signing.stderr);
    const [, address] = await out.until(/^ {2}(http\S+)$/m);
    const back = await signInIn(address);
    await err.until(/Address: /);
    // Not the address first: a wrong paste is said, and asked for again.
    signing.stdin.write("http://127.0.0.1:1/signed-in?code=not-this-one\n");
    await err.until(/not the address this sign-in waits for/);
    signing.stdin.write(`${back}\n`);
    await signing.stdin.flush();
    await out.until(/Signed in as /);
    process.exit(await signing.exited);
    break;
  }
  case "pair": {
    // A terminal of the operator's own: `script` gives the command one.
    const pairing = Bun.spawn(["script", "-qefc", `${HOST.join(" ")} pair`, "/dev/null"], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "inherit",
      env: own(),
    });
    const out = reading(pairing.stdout);
    const [, code] = await out.until(/^ {2}([A-Z0-9-]{6,})\s*$/m);
    writeFileSync(join(dir, "pairing-code"), code);
    const window = named === "window";
    const asker = window
      ? ["bun", ".yardsort/tests/support/window.ts", port, "enter-code", dir]
      : ["bun", ".yardsort/tests/support/phone.ts", dir, "ask"];
    const asking = Bun.spawnSync(asker, { stdout: "inherit", stderr: "inherit" });
    if (asking.exitCode !== 0) throw new Error("The device could not ask.");
    await out.until(/Allow \[v\]iewing, allow viewing and \[t\]yping, or \[d\]eny\?/);
    pairing.stdin.write(window ? `${answer ?? "v"}\n` : named === "watching" ? "v\n" : "t\n");
    await pairing.stdin.flush();
    await out.until(/Let in\./);
    pairing.kill();
    process.exit(0);
    break;
  }
  default:
    throw new Error(`Nothing called ${doing}.`);
}
