// The Yardsort server's local development environment: PostgreSQL and a throwaway ZITADEL in
// Docker (`crates/server/dev/compose.yaml`), and the server itself on this machine.
//
//   bun scripts/server-dev.mjs up      start the containers, and give ZITADEL the server's app
//   bun scripts/server-dev.mjs run     run the server against them (cargo run)
//   bun scripts/server-dev.mjs test    the server's tests, the PostgreSQL ones included
//   bun scripts/server-dev.mjs image   build the server's container image (crates/server/Dockerfile)
//   bun scripts/server-dev.mjs e2e     build the image, run it, and sign in through ZITADEL (Linux)
//   bun scripts/server-dev.mjs down    stop the containers, keeping what is in them
//   bun scripts/server-dev.mjs reset   stop them and forget everything: accounts, ZITADEL, the lot
//
// `up` writes the server's settings, its client secret included, to
// `crates/server/dev/server.env.local`, which git ignores. That secret belongs to the throwaway
// ZITADEL in the containers and opens nothing else.
//
// Plain JavaScript on Node's APIs, so it runs the same under bun on all three systems.

import { execFileSync, spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const dev = join(root, "crates", "server", "dev");
const compose = ["compose", "-f", join(dev, "compose.yaml")];
const settingsFile = join(dev, "server.env.local");

const ISSUER = "http://127.0.0.1:8080";
const PUBLIC_URL = "http://127.0.0.1:8787";
const CALLBACK = `${PUBLIC_URL}/v1/login/callback`;
const PROJECT = "Yardsort server";
const APP = "Yardsort server (development)";
/** The server's own database, and the one its tests make a schema each in. */
const DATABASE_URL = "postgres://postgres:postgres@127.0.0.1:55432/yardsort?sslmode=disable";
const TEST_DATABASE_URL =
  "postgres://postgres:postgres@127.0.0.1:55432/yardsort_test?sslmode=disable";

const docker = (args, options = {}) =>
  execFileSync("docker", [...compose, ...args], { cwd: root, encoding: "utf8", ...options });

const IMAGE = "yardsort-server:dev";
const sleep = (ms) => new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
const E2E_CONTAINER = "yardsort-server-e2e";

async function zitadel(token, method, path, body) {
  const response = await fetch(`${ISSUER}${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`ZITADEL ${method} ${path}: ${response.status} ${text}`);
  return text ? JSON.parse(text) : {};
}

function readSettings() {
  if (!existsSync(settingsFile)) return null;
  const settings = {};
  for (const line of readFileSync(settingsFile, "utf8").split("\n")) {
    const match = /^([A-Z_]+)=(.*)$/.exec(line);
    if (match) settings[match[1]] = match[2];
  }
  return settings;
}

async function up() {
  docker(["up", "-d", "--wait"], { stdio: "inherit" });
  // The service account ZITADEL made at first start; its token is in the shared volume.
  const token = docker([
    "exec",
    "-T",
    "zitadel-login",
    "cat",
    "/zitadel/bootstrap/setup.pat",
  ]).trim();

  const projects = await zitadel(token, "POST", "/management/v1/projects/_search", {
    queries: [{ nameQuery: { name: PROJECT, method: "TEXT_QUERY_METHOD_EQUALS" } }],
  });
  const projectId =
    projects.result?.[0]?.id ??
    (await zitadel(token, "POST", "/management/v1/projects", { name: PROJECT })).id;

  const apps = await zitadel(token, "POST", `/management/v1/projects/${projectId}/apps/_search`, {
    queries: [{ nameQuery: { name: APP, method: "TEXT_QUERY_METHOD_EQUALS" } }],
  });
  const existing = apps.result?.[0];
  const known = readSettings();
  let clientId;
  let clientSecret;
  if (existing && known?.YARDSORT_OIDC_CLIENT_ID === existing.oidcConfig?.clientId) {
    console.log(`ZITADEL already has ${APP}; ${settingsFile} is current.`);
    return;
  } else if (existing) {
    // The app is there but its secret was not kept here: make a new one.
    clientId = existing.oidcConfig.clientId;
    clientSecret = (
      await zitadel(
        token,
        "POST",
        `/management/v1/projects/${projectId}/apps/${existing.id}/oidc_config/_generate_client_secret`,
        {},
      )
    ).clientSecret;
  } else {
    // The settings the owner's checklist gives for a real instance (docs/design/28), except that
    // development mode allows the plain-HTTP callback.
    const created = await zitadel(token, "POST", `/management/v1/projects/${projectId}/apps/oidc`, {
      name: APP,
      redirectUris: [CALLBACK],
      responseTypes: ["OIDC_RESPONSE_TYPE_CODE"],
      grantTypes: ["OIDC_GRANT_TYPE_AUTHORIZATION_CODE"],
      appType: "OIDC_APP_TYPE_WEB",
      authMethodType: "OIDC_AUTH_METHOD_TYPE_BASIC",
      accessTokenType: "OIDC_TOKEN_TYPE_BEARER",
      devMode: true,
    });
    clientId = created.clientId;
    clientSecret = created.clientSecret;
  }

  writeFileSync(
    settingsFile,
    [
      "# Written by `just server-up`. The local development server's settings; never commit.",
      `YARDSORT_PUBLIC_URL=${PUBLIC_URL}`,
      `YARDSORT_OIDC_ISSUER=${ISSUER}`,
      `YARDSORT_OIDC_CLIENT_ID=${clientId}`,
      `YARDSORT_OIDC_CLIENT_SECRET=${clientSecret}`,
      `DATABASE_URL=${DATABASE_URL}`,
      "",
    ].join("\n"),
    { mode: 0o600 },
  );
  console.log(`ZITADEL has ${APP}, sending sign-ins back to ${CALLBACK}.`);
  console.log(`Settings written to ${settingsFile}.`);
}

function run() {
  const settings = readSettings();
  if (!settings) {
    console.error("No settings yet: run `just server-up` first.");
    process.exit(1);
  }
  const result = spawnSync(
    "cargo",
    ["run", "-q", "-p", "yardsort-server", "--bin", "yardsort-server"],
    { cwd: root, stdio: "inherit", env: { ...process.env, ...settings } },
  );
  process.exit(result.status ?? 1);
}

function test(args) {
  const result = spawnSync("cargo", ["test", "-p", "yardsort-server", ...args], {
    cwd: root,
    stdio: "inherit",
    env: {
      ...process.env,
      YARDSORT_TEST_DATABASE_URL: TEST_DATABASE_URL,
      // Fail, rather than skip, when the containers are not up.
      YARDSORT_REQUIRE_TEST_DATABASE: "1",
    },
  });
  process.exit(result.status ?? 1);
}

function image() {
  execFileSync(
    "docker",
    ["build", "-f", join("crates", "server", "Dockerfile"), "-t", IMAGE, "."],
    {
      cwd: root,
      stdio: "inherit",
    },
  );
}

async function healthy(url) {
  try {
    return (await fetch(`${url}/health`)).ok;
  } catch {
    return false;
  }
}

/** The image, run as the server against the containers, and signed in to through ZITADEL with no
 * browser: what CI runs, and slice 3's local smoke test. The container shares the host's network,
 * which Docker offers on Linux. */
async function e2e() {
  const settings = readSettings();
  if (!settings) {
    console.error("No settings yet: run `just server-up` first.");
    process.exit(1);
  }
  if (await healthy(PUBLIC_URL)) {
    console.error(`Something already answers at ${PUBLIC_URL}: stop \`just server-run\` first.`);
    process.exit(1);
  }
  image();
  execFileSync("docker", ["rm", "-f", E2E_CONTAINER], { stdio: "ignore" });
  execFileSync("docker", [
    ...["run", "-d", "--name", E2E_CONTAINER, "--network", "host"],
    ...["--env-file", settingsFile, "-e", `YARDSORT_LISTEN=127.0.0.1:${new URL(PUBLIC_URL).port}`],
    IMAGE,
  ]);
  const logs = () =>
    spawnSync("docker", ["logs", E2E_CONTAINER], { encoding: "utf8" }).output.join("");
  let passed = false;
  try {
    for (let i = 0; i < 60 && !(await healthy(PUBLIC_URL)); i++) await sleep(500);
    if (!(await healthy(PUBLIC_URL))) throw new Error("the server did not come up");

    const token = docker([
      "exec",
      "-T",
      "zitadel-login",
      "cat",
      "/zitadel/bootstrap/login-client.pat",
    ]).trim();
    passed = (await signInAndRevoke(token)) && (await pairAndRevoke(token));
  } catch (error) {
    console.error(error.message);
  } finally {
    const text = logs();
    // The server's log must hold neither its client secret nor anyone's credentials.
    for (const [what, secret] of [
      ["the client secret", settings.YARDSORT_OIDC_CLIENT_SECRET],
      ["a bearer credential", "Bearer "],
      // Every credential, sign-in code and pairing secret the server issues starts so.
      ["a credential or pairing secret", "ys1."],
    ]) {
      if (secret && text.includes(secret)) {
        console.error(`The server's log holds ${what}.`);
        passed = false;
      }
    }
    if (!passed) console.error(`\nThe server's log:\n${text}`);
    execFileSync("docker", ["rm", "-f", E2E_CONTAINER], { stdio: "ignore" });
  }
  console.log(passed ? "\nEnd to end: passed." : "\nEnd to end: FAILED.");
  process.exit(passed ? 0 : 1);
}

/** The sign-in tool (`crates/server/examples/sign-in.rs`) signs in through ZITADEL's API and
 * waits; once it does, the image's own `revoke-account` signs the account out, and the tool checks
 * that its credentials stopped working. */
function signInAndRevoke(token) {
  return new Promise((resolvePassed) => {
    // Built first and run directly, so that stopping it stops the tool and not only cargo.
    execFileSync("cargo", ["build", "-q", "-p", "yardsort-server", "--example", "sign-in"], {
      cwd: root,
      stdio: "inherit",
    });
    const target = process.env.CARGO_TARGET_DIR ?? join(root, "target");
    const binary = join(target, "debug", "examples", "sign-in");
    const tool = spawn(
      process.platform === "win32" ? `${binary}.exe` : binary,
      [PUBLIC_URL, "--zitadel-api", "--wait-for-revoke"],
      {
        cwd: root,
        stdio: ["ignore", "pipe", "inherit"],
        env: {
          ...process.env,
          ZITADEL_LOGIN_CLIENT_TOKEN: token,
          YARDSORT_SIGN_IN_AS: "developer@yardsort.test",
          YARDSORT_SIGN_IN_PASSWORD: "Yardsort-dev-1",
        },
      },
    );
    let output = "";
    let revoked = false;
    tool.stdout.setEncoding("utf8");
    tool.stdout.on("data", (chunk) => {
      process.stdout.write(chunk);
      output += chunk;
      const subject = /as user (\S+)\./.exec(output)?.[1];
      if (!revoked && subject && output.includes("Waiting for this device")) {
        revoked = true;
        const result = spawnSync(
          "docker",
          [
            ...["exec", E2E_CONTAINER, "/usr/local/bin/yardsort-server"],
            ...["revoke-account", "--subject", subject],
          ],
          { encoding: "utf8" },
        );
        process.stdout.write(`\n$ yardsort-server revoke-account --subject ${subject}\n`);
        process.stdout.write(result.output.join(""));
        // A revocation takes effect on the device's next request, which it makes every half second.
        setTimeout(() => tool.kill(), 30_000).unref();
      }
    });
    tool.on("exit", (code) => resolvePassed(code === 0 && revoked));
  });
}

/** The pairing tool (`crates/server/examples/pair.rs`) as a host and as a phone, both signed in
 * through ZITADEL's API: the host connects to the relay and shows a code, the phone redeems it and
 * asks, the relay tells the host so, and the host lets it watch and type. The phone reaches the host through the relay and types
 * into its program, and hears of a second program the host starts and that ends; the host takes
 * the grant back, and the relay closes the phone's socket. The program got the line and was still
 * running. */
async function pairAndRevoke(token) {
  execFileSync("cargo", ["build", "-q", "-p", "yardsort-server", "--example", "pair"], {
    cwd: root,
    stdio: "inherit",
  });
  const target = process.env.CARGO_TARGET_DIR ?? join(root, "target");
  const binary = join(target, "debug", "examples", "pair");
  const env = {
    ...process.env,
    ZITADEL_LOGIN_CLIENT_TOKEN: token,
    YARDSORT_SIGN_IN_AS: "developer@yardsort.test",
    YARDSORT_SIGN_IN_PASSWORD: "Yardsort-dev-1",
  };
  const run = (args, label, onLine) =>
    new Promise((resolveRun) => {
      const tool = spawn(process.platform === "win32" ? `${binary}.exe` : binary, args, {
        cwd: root,
        stdio: ["ignore", "pipe", "inherit"],
        env,
      });
      let output = "";
      tool.stdout.setEncoding("utf8");
      tool.stdout.on("data", (chunk) => {
        for (const line of chunk.split("\n").filter(Boolean)) {
          process.stdout.write(`${label} ${line}\n`);
          onLine?.(line);
        }
        output += chunk;
      });
      const timer = setTimeout(() => tool.kill(), 120_000);
      tool.on("exit", (code) => {
        clearTimeout(timer);
        resolveRun({ code, output });
      });
    });
  let phone;
  const host = run(
    ["host", PUBLIC_URL, "--zitadel-api", "--control", "--serve", "--revoke-after", "10"],
    "[host] ",
    (line) => {
      const code = /^Code: (\S+)$/.exec(line)?.[1];
      if (code && !phone) {
        phone = run(
          [
            ...["phone", PUBLIC_URL, code, "--zitadel-api", "--wait-for-revoke"],
            ...["--relay", "--type", "e2e-typed"],
          ],
          "[phone]",
        );
      }
    },
  );
  const hosted = await host;
  const phoned = phone ? await phone : { code: 1, output: "" };
  return (
    hosted.code === 0 &&
    phoned.code === 0 &&
    phoned.output.includes("watch and type on every project") &&
    phoned.output.includes('Typed "e2e-typed"') &&
    phoned.output.includes("resumed from there") &&
    phoned.output.includes("numbered without a gap") &&
    phoned.output.includes("Activity: process.exited") &&
    phoned.output.includes("access was revoked") &&
    hosted.output.includes("The relay says a device asks to pair.") &&
    hosted.output.includes('"got:e2e-typed"') &&
    hosted.output.includes("still running when the link closed")
  );
}

const command = process.argv[2];
if (command === "up") await up();
else if (command === "run") run();
else if (command === "test") test(process.argv.slice(3));
else if (command === "image") image();
else if (command === "e2e") await e2e();
else if (command === "down") docker(["down"], { stdio: "inherit" });
else if (command === "reset") {
  docker(["down", "--volumes"], { stdio: "inherit" });
  rmSync(settingsFile, { force: true });
} else {
  console.error("usage: bun scripts/server-dev.mjs up | run | test | image | e2e | down | reset");
  process.exit(2);
}
