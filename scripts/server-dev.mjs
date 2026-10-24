// The Yardsort server's local development environment: PostgreSQL and a throwaway ZITADEL in
// Docker (`crates/server/dev/compose.yaml`), and the server itself on this machine.
//
//   bun scripts/server-dev.mjs up      start the containers, and give ZITADEL the server's app
//   bun scripts/server-dev.mjs run     run the server against them (cargo run)
//   bun scripts/server-dev.mjs test    the server's tests, the PostgreSQL ones included
//   bun scripts/server-dev.mjs down    stop the containers, keeping what is in them
//   bun scripts/server-dev.mjs reset   stop them and forget everything: accounts, ZITADEL, the lot
//
// `up` writes the server's settings, its client secret included, to
// `crates/server/dev/server.env.local`, which git ignores. That secret belongs to the throwaway
// ZITADEL in the containers and opens nothing else.
//
// Plain JavaScript on Node's APIs, so it runs the same under bun on all three systems.

import { execFileSync, spawnSync } from "node:child_process";
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

const command = process.argv[2];
if (command === "up") await up();
else if (command === "run") run();
else if (command === "test") test(process.argv.slice(3));
else if (command === "down") docker(["down"], { stdio: "inherit" });
else if (command === "reset") {
  docker(["down", "--volumes"], { stdio: "inherit" });
  rmSync(settingsFile, { force: true });
} else {
  console.error("usage: bun scripts/server-dev.mjs up | run | test | down | reset");
  process.exit(2);
}
