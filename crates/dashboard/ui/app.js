// The host dashboard's page. No framework and no build: it asks the host over /api and draws
// what it is told. Everything a job prints, and every name, is set as text, never as markup.
"use strict";

const $ = (id) => document.getElementById(id);

/** An element, with its properties and children. A string child is text. */
function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (key in node) node[key] = value;
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child === undefined || child === null || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function replace(node, ...children) {
  node.replaceChildren(
    ...children.flat().filter((c) => c !== null && c !== undefined && c !== false),
  );
}

// ── Asking the host ──────────────────────────────────────────────────────────────────────────

class SignedOut extends Error {}
class Unreachable extends Error {}

async function ask(method, path, body) {
  let response;
  try {
    response = await fetch(path, {
      method,
      // What tells the host this is its own page asking: no form or link can add a header.
      headers: {
        "X-Yardsort-Dashboard": "1",
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "same-origin",
      cache: "no-store",
    });
  } catch {
    lost(true);
    throw new Unreachable("This computer is not answering.");
  }
  lost(false);
  let data = null;
  try {
    data = await response.json();
  } catch {
    // Not JSON: a refusal from something between here and the host.
  }
  if (response.status === 401 && data?.error === "signed_out") {
    showGate();
    throw new SignedOut("Signed out.");
  }
  if (!response.ok) {
    throw new Error(data?.message ?? `The host answered ${response.status}.`);
  }
  return data;
}

function lost(is) {
  $("lost").hidden = !is;
}

/** Whether an error is one the page already shows in its own way. */
const quiet = (error) => error instanceof SignedOut || error instanceof Unreachable;

// ── Words for numbers ────────────────────────────────────────────────────────────────────────

function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let at = 0;
  let value = n;
  while (value >= 1024 && at < units.length - 1) {
    value /= 1024;
    at += 1;
  }
  const number = value >= 100 || at === 0 ? String(Math.round(value)) : value.toFixed(1);
  return `${number.replace(/\.0$/, "")} ${units[at]}`;
}

function lasted(seconds) {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
}

const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

// ── Signing in ───────────────────────────────────────────────────────────────────────────────

let signedIn = false;

function showGate(message) {
  signedIn = false;
  stopEverything();
  $("shell").hidden = true;
  $("gate").hidden = false;
  const error = $("gate-error");
  error.hidden = !message;
  error.textContent = message ?? "";
  $("gate-code").focus();
}

/**
 * The code a sign-in link carries after its #, which a browser sends to nobody. Taken out of
 * the address at once, so that it is not in the history, nor in a copy of the address.
 */
function codeInAddress() {
  const code = /^#code=([0-9A-Za-z-]+)$/.exec(location.hash)?.[1];
  if (code) history.replaceState(null, "", location.pathname);
  return code;
}

async function redeem(code) {
  try {
    await ask("POST", "/api/session", { code });
  } catch (error) {
    showGate(error.message);
    return;
  }
  $("gate-code").value = "";
  showShell();
}

$("gate-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void redeem($("gate-code").value);
});

$("sign-out").addEventListener("click", async () => {
  try {
    await ask("DELETE", "/api/session");
  } catch (error) {
    // Signed out, but the host could not write it down: it says what a restart would do.
    showGate(quiet(error) ? undefined : error.message);
    return;
  }
  showGate();
});

// ── The frame ────────────────────────────────────────────────────────────────────────────────

const TABS = ["overview", "agents", "git", "commands", "access"];
let tab = null;
let timers = [];
const watchers = new Map();

function every(ms, work) {
  const run = () => {
    if (!signedIn || document.hidden) return;
    void work().catch((error) => {
      if (!quiet(error)) console.error(error);
    });
  };
  run();
  timers.push(setInterval(run, ms));
}

function stopEverything() {
  for (const timer of timers) clearInterval(timer);
  timers = [];
  for (const watcher of watchers.values()) watcher.stop();
  watchers.clear();
  tab = null;
}

function showShell() {
  signedIn = true;
  $("gate").hidden = true;
  $("shell").hidden = false;
  showTab();
}

function showTab() {
  if (!signedIn) return;
  const wanted = location.hash.slice(1);
  const next = TABS.includes(wanted) ? wanted : "overview";
  if (next === tab) return;
  for (const timer of timers) clearInterval(timer);
  timers = [];
  tab = next;
  for (const name of TABS) $(`tab-${name}`).hidden = name !== tab;
  for (const link of $("tabs").querySelectorAll("a")) {
    if (link.dataset.tab === tab) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  // The header and the overview are the same reading; every tab keeps the header true.
  every(5000, readStatus);
  if (tab === "agents") void readAgents();
  if (tab === "git") void readGit();
  if (tab === "commands") every(3000, readJobs);
  if (tab === "access") tunnelFormFilled = false;
}

window.addEventListener("hashchange", () => {
  // A sign-in link opened in a tab that already shows the dashboard does not load it again.
  const code = codeInAddress();
  if (code) void redeem(code);
  else showTab();
});
document.addEventListener("visibilitychange", () => {
  if (!document.hidden && signedIn) void readStatus().catch(() => {});
});

// ── Asking before something is done ──────────────────────────────────────────────────────────

function confirmed({ title, text, terms, yes }) {
  const dialog = $("confirm");
  $("confirm-title").textContent = title;
  $("confirm-text").textContent = text;
  $("confirm-terms").hidden = !terms;
  $("confirm-terms").textContent = terms ?? "";
  $("confirm-yes").textContent = yes;
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "yes"), { once: true });
    dialog.returnValue = "no";
    dialog.showModal();
  });
}

for (const button of $("confirm").querySelectorAll("button")) {
  button.type = "button";
  button.addEventListener("click", () => $("confirm").close(button.value));
}

// ── A job, with its output as it comes ───────────────────────────────────────────────────────

const STATE_WORDS = {
  running: "Running…",
  succeeded: "Done",
  failed: "Failed",
  stopped: "Stopped",
};
const STATE_DOT = { running: "warn", succeeded: "good", failed: "bad", stopped: "" };

/** Show job `id` in `container`, following it until it ends. One job per container. */
function watchJob(container, id, onEnd) {
  watchers.get(container)?.stop();
  const title = el("span", { class: "job-title" });
  const state = el("span", { class: "job-state", role: "status" });
  const stop = el("button", { type: "button", text: "Stop", hidden: true });
  const output = el("pre", { class: "output", tabIndex: 0 });
  replace(
    container,
    el("div", { class: "job" }, el("div", { class: "job-head" }, title, state, stop), output),
  );

  let next = 0;
  let timer = null;
  let stopped = false;
  stop.addEventListener("click", () => {
    stop.disabled = true;
    void ask("POST", `/api/jobs/${id}/stop`).catch(() => {});
  });

  const read = async () => {
    let job;
    try {
      job = await ask("GET", `/api/jobs/${id}?from=${next}`);
    } catch (error) {
      if (error instanceof Unreachable && !stopped) timer = setTimeout(read, 2000);
      else if (!quiet(error)) state.textContent = error.message;
      return;
    }
    if (stopped) return;
    title.textContent = job.title;
    replace(state, el("span", { class: `dot ${STATE_DOT[job.state]}` }), STATE_WORDS[job.state]);
    stop.hidden = !job.stoppable;
    if (job.output) {
      const atEnd = output.scrollHeight - output.scrollTop - output.clientHeight < 24;
      // The start of a long output is no longer kept: say so, rather than join the two ends.
      if (job.from > next && next > 0) output.append("\n[…]\n");
      output.append(job.output);
      if (atEnd) output.scrollTop = output.scrollHeight;
    }
    next = job.next;
    if (job.state === "running") {
      timer = setTimeout(read, 700);
    } else {
      onEnd?.(job);
    }
  };

  const watcher = {
    id,
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
  watchers.set(container, watcher);
  void read();
  return watcher;
}

/** Start a job with `start`, and follow it in `container`. Says a refusal in `container`. */
async function startJob(container, start, onEnd) {
  try {
    const job = await start();
    watchJob(container, job.id, onEnd);
    return job;
  } catch (error) {
    if (!quiet(error)) {
      watchers.get(container)?.stop();
      replace(container, el("p", { class: "error", role: "alert", text: error.message }));
    }
    return null;
  }
}

// ── Overview ─────────────────────────────────────────────────────────────────────────────────

let status = null;
let tunnelFormFilled = false;

function facts(list, pairs) {
  replace(
    list,
    pairs
      .filter(([, value]) => value !== null && value !== undefined && value !== "")
      .flatMap(([name, value]) => [el("dt", { text: name }), el("dd", {}, value)]),
  );
}

function meter(name, used, total, words) {
  const share = total > 0 ? Math.min(1, used / total) : 0;
  const fill = el("i", { class: share > 0.9 ? "high" : "" });
  fill.style.width = `${Math.round(share * 100)}%`;
  return el(
    "div",
    { class: "meter" },
    el("div", { class: "meter-head" }, el("span", { text: name }), el("span", { text: words })),
    el("div", { class: "bar", role: "img", ariaLabel: `${name}: ${words}` }, fill),
  );
}

function projectRows(projects, none) {
  if (projects.length === 0) return el("p", { class: "muted", text: none });
  return el(
    "div",
    { class: "rows" },
    projects.map((project) =>
      el(
        "div",
        { class: "row" },
        el(
          "div",
          { class: "row-main" },
          el("strong", { text: project.name }),
          el("span", { class: "faint mono", text: project.path }),
        ),
      ),
    ),
  );
}

function tunnelWords(tunnel) {
  switch (tunnel.state) {
    case "off":
      return [el("span", { class: "dot" }), "Off."];
    case "starting":
      return [el("span", { class: "dot warn" }), "Starting…"];
    case "ready":
      return [
        el("span", { class: "dot good" }),
        "Up at ",
        el("a", {
          href: tunnel.url,
          text: tunnel.url,
          rel: "noreferrer noopener",
          target: "_blank",
        }),
      ];
    case "reconnecting":
      return [el("span", { class: "dot warn" }), `Reconnecting: ${tunnel.reason}`];
    case "failed":
      return [
        el("span", { class: "dot bad" }),
        `Not up: ${tunnel.message}${tunnel.hint ? ` ${tunnel.hint}` : ""} It is tried again in a while.`,
      ];
    case "missing":
      return [
        el("span", { class: "dot bad" }),
        "There is no rustunnel on this user's PATH. Install it, run rustunnel setup there, and save again.",
      ];
    default:
      return [tunnel.state];
  }
}

async function readStatus() {
  status = await ask("GET", "/api/status");
  const { host, machine, dashboard, projects } = status;

  $("host-name").textContent = host.remote?.name ?? machine.hostName ?? "Yardsort host";
  $("host-sub").textContent = `${host.program} ${host.version}`;
  document.title = `${$("host-name").textContent} · Yardsort host`;
  $("command-user").textContent = status.user ?? "its user";
  $("command-shell").textContent = status.shell;

  replace(
    $("notices"),
    [...host.notices, status.environmentProblem, status.settingsProblem]
      .filter(Boolean)
      .map((text) => el("p", { class: "notice", role: "note", text })),
  );

  const remote = host.remote;
  facts($("host-facts"), [
    ["Program", `${host.program} ${host.version}`],
    ["Runs as", status.user ? `${status.user} (process ${host.pid})` : `process ${host.pid}`],
    [
      "Remote",
      remote
        ? [el("span", { class: "dot good" }), `On, as ${remote.name} at ${remote.serverUrl}`]
        : [el("span", { class: "dot" }), `Off. ${host.remoteOffHint}`],
    ],
    ["Link", remote?.link],
    ["Devices let in", remote ? String(remote.devices) : null],
    ["Terminals running", String(host.terminals)],
  ]);

  facts($("machine-facts"), [
    ["Name", machine.hostName],
    ["System", machine.os],
    ["Kernel", machine.kernel],
    ["Processors", `${machine.cpus} × ${machine.arch}`],
    ["Up for", lasted(machine.uptime)],
  ]);
  const load = machine.load ? `load ${machine.load.map((n) => n.toFixed(2)).join(" ")}` : "";
  replace(
    $("meters"),
    machine.cpuPercent === null
      ? null
      : meter(
          "Processors",
          machine.cpuPercent,
          100,
          [`${Math.round(machine.cpuPercent)}%`, load].filter(Boolean).join(" · "),
        ),
    meter(
      "Memory",
      machine.memory.used,
      machine.memory.total,
      `${bytes(machine.memory.used)} of ${bytes(machine.memory.total)}`,
    ),
    machine.swap.total > 0 &&
      meter(
        "Swap",
        machine.swap.used,
        machine.swap.total,
        `${bytes(machine.swap.used)} of ${bytes(machine.swap.total)}`,
      ),
    machine.disks.map((disk) =>
      meter(
        `${disk.holds} (${disk.mount})`,
        disk.used,
        disk.total,
        `${bytes(disk.total - disk.used)} free of ${bytes(disk.total)}`,
      ),
    ),
  );

  replace(
    $("overview-projects"),
    projectRows(
      projects,
      "No projects yet. Clone a repository under Git, or add one with ys project add.",
    ),
  );

  // Access
  facts($("access-facts"), [
    ["On this computer", el("code", { text: dashboard.address })],
    ["Through the tunnel", tunnelWords(dashboard.tunnel)],
  ]);
  $("ssh-line").textContent =
    `ssh -L ${dashboard.port}:127.0.0.1:${dashboard.port} ${machine.hostName ?? "this-computer"}`;
  replace($("tunnel-state"), tunnelWords(dashboard.tunnel));
  $("sessions-count").textContent =
    dashboard.signedIn === 1
      ? "This browser alone is signed in."
      : `${dashboard.signedIn} browsers are signed in, this one among them.`;
  if (!tunnelFormFilled) {
    const wanted = dashboard.tunnelWanted;
    $("tunnel-enabled").checked = wanted.enabled;
    $("tunnel-subdomain").value = wanted.subdomain ?? "";
    $("tunnel-region").value = wanted.region ?? "";
    $("tunnel-server").value = wanted.server ?? "";
    tunnelFormFilled = true;
  }
}

// ── Agents ───────────────────────────────────────────────────────────────────────────────────

let installing = false;

async function readAgents() {
  let agents;
  try {
    agents = await ask("GET", "/api/agents");
  } catch (error) {
    if (!quiet(error)) replace($("agents-list"), el("p", { class: "error", text: error.message }));
    return;
  }
  replace(
    $("agents-list"),
    agents.map((agent) => {
      const button =
        agent.install &&
        el("button", {
          type: "button",
          text: agent.found ? "Update" : "Install",
          class: agent.found ? "" : "primary",
          disabled: installing,
          onclick: () => void install(agent),
        });
      return el(
        "div",
        { class: "row" },
        el(
          "div",
          { class: "row-main" },
          el(
            "strong",
            {},
            agent.name,
            !agent.enabled && el("span", { class: "tag", text: "turned off in Settings" }),
          ),
          agent.found
            ? el(
                "span",
                { class: "faint" },
                el("span", { class: "dot good" }),
                [agent.version, agent.found].filter(Boolean).join(" · "),
              )
            : el(
                "span",
                { class: "faint" },
                el("span", { class: "dot" }),
                `Not found: no ${agent.command} on this user's PATH.`,
              ),
        ),
        button,
      );
    }),
  );
}

async function install(agent) {
  const user = status?.user ?? "this user";
  const from = agent.install;
  const sure = await confirmed({
    title: `${agent.found ? "Update" : "Install"} ${agent.name}?`,
    text: from.vendor
      ? `This downloads ${from.from}, ${from.what}, and runs it on this computer as ${user}.`
      : `This installs ${from.what} from npm, its newest release, into ~/.local as ${user}.`,
    terms: from.vendor
      ? `It is ${from.vendor}'s software, under ${from.vendor}'s terms (${from.terms}): installing it is accepting them.`
      : null,
    yes: agent.found ? "Update" : "Install",
  });
  if (!sure) return;
  installing = true;
  void readAgents();
  const done = () => {
    installing = false;
    void readAgents();
  };
  const job = await startJob(
    $("agents-job"),
    () => ask("POST", `/api/agents/${agent.id}/install`),
    done,
  );
  if (!job) done();
}

$("agents-refresh").addEventListener("click", () => void readAgents());

// ── Git ──────────────────────────────────────────────────────────────────────────────────────

let identityTouched = false;

async function readGit() {
  let git;
  try {
    git = await ask("GET", "/api/git");
  } catch (error) {
    if (!quiet(error)) $("github-state").textContent = error.message;
    return;
  }
  $("git-missing").hidden = git.version !== null;
  if (!identityTouched) {
    $("identity-name").value = git.name ?? "";
    $("identity-email").value = git.email ?? "";
  }
  const github = git.github;
  const found = github.version !== null;
  replace(
    $("github-state"),
    !found
      ? [
          el("span", { class: "dot bad" }),
          "There is no GitHub CLI (gh) on this user's PATH. Install it as ",
          el("a", {
            href: "https://github.com/cli/cli#installation",
            text: "GitHub says",
            rel: "noreferrer noopener",
            target: "_blank",
          }),
          ", then look again.",
        ]
      : github.signedIn
        ? [el("span", { class: "dot good" }), `Signed in. ${github.version}`]
        : [el("span", { class: "dot" }), `Not signed in. ${github.version}`],
  );
  $("github-detail").hidden = !found || !github.detail;
  $("github-detail").textContent = github.detail;
  $("github-sign-in").disabled = !found;
  $("github-sign-in").textContent = github.signedIn
    ? "Sign in again with a browser"
    : "Sign in with a browser";
  $("github-sign-in").className = github.signedIn ? "" : "primary";
  $("github-sign-out").hidden = !github.signedIn;
  $("github-token").hidden = !found;
  $("projects-dir").textContent = git.projectsDir ?? "~/Projects";
  replace($("git-projects"), projectRows(git.projects, "No projects yet."));
}

$("git-refresh").addEventListener("click", () => void readGit());

for (const id of ["identity-name", "identity-email"]) {
  $(id).addEventListener("input", () => {
    identityTouched = true;
    $("identity-said").textContent = "";
  });
}

$("identity-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const said = $("identity-said");
  said.className = "said";
  try {
    await ask("POST", "/api/git/identity", {
      name: $("identity-name").value,
      email: $("identity-email").value,
    });
    identityTouched = false;
    said.textContent = "Saved.";
    void readGit();
  } catch (error) {
    if (quiet(error)) return;
    said.className = "said bad";
    said.textContent = error.message;
  }
});

$("github-sign-in").addEventListener("click", () => {
  void startJob(
    $("github-job"),
    () => ask("POST", "/api/git/github/sign-in"),
    () => void readGit(),
  );
});

$("github-sign-out").addEventListener("click", async () => {
  const sure = await confirmed({
    title: "Sign the GitHub CLI out here?",
    text: "Agents on this computer can then no longer push or open pull requests. The token stays good at GitHub until you delete it there.",
    yes: "Sign out",
  });
  if (!sure) return;
  void startJob(
    $("github-job"),
    () => ask("POST", "/api/git/github/sign-out"),
    () => void readGit(),
  );
});

$("token-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const token = $("token-input").value;
  $("token-input").value = "";
  void startJob(
    $("github-job"),
    () => ask("POST", "/api/git/github/token", { token }),
    () => void readGit(),
  );
});

$("clone-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const remote = $("clone-remote").value;
  void startJob(
    $("clone-job"),
    () => ask("POST", "/api/projects/clone", { remote }),
    (job) => {
      if (job.state === "succeeded") $("clone-remote").value = "";
      void readGit();
    },
  );
});

// ── Commands ─────────────────────────────────────────────────────────────────────────────────

let shownJob = null;

async function readJobs() {
  const jobs = await ask("GET", "/api/jobs");
  $("jobs-none").hidden = jobs.length > 0;
  if (shownJob === null && jobs.length > 0) showJob(jobs[0].id);
  replace(
    $("jobs-list"),
    jobs.map((job) =>
      el(
        "li",
        {},
        el(
          "button",
          {
            type: "button",
            title: job.title,
            ariaCurrent: job.id === shownJob ? "true" : undefined,
            onclick: () => {
              showJob(job.id);
              void readJobs();
            },
          },
          el("span", { class: `dot ${STATE_DOT[job.state]}` }),
          el("span", { text: job.title }),
        ),
      ),
    ),
  );
}

function showJob(id) {
  shownJob = id;
  watchJob($("jobs-view"), id, () => void readJobs().catch(() => {}));
}

$("command-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const error = $("command-error");
  error.hidden = true;
  try {
    const job = await ask("POST", "/api/commands", { command: $("command-line").value });
    $("command-line").value = "";
    showJob(job.id);
    void readJobs();
  } catch (refused) {
    if (quiet(refused)) return;
    error.hidden = false;
    error.textContent = refused.message;
  }
});

// ── Access ───────────────────────────────────────────────────────────────────────────────────

const throughTheTunnel = !["127.0.0.1", "localhost", "[::1]"].includes(location.hostname);

$("tunnel-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const said = $("tunnel-said");
  said.className = "said";
  said.textContent = "";
  const wanted = {
    enabled: $("tunnel-enabled").checked,
    subdomain: $("tunnel-subdomain").value.trim() || null,
    region: $("tunnel-region").value || null,
    server: $("tunnel-server").value.trim() || null,
  };
  if (throughTheTunnel) {
    const sure = await confirmed({
      title: "Change the tunnel you are using?",
      text: "This page reached the host through the tunnel. Saving restarts it, and turning it off or changing its subdomain takes this address away: you would need the new address, or an SSH forward, to come back.",
      yes: "Save",
    });
    if (!sure) return;
  } else if (wanted.enabled && !status?.dashboard.tunnelWanted.enabled) {
    const sure = await confirmed({
      title: "Give the dashboard a public address?",
      text: "Anyone who has the address can reach its sign-in page. Only a sign-in code from this computer lets them further, and the tunnel's server can read what passes through it.",
      yes: "Turn the tunnel on",
    });
    if (!sure) return;
  }
  try {
    await ask("POST", "/api/tunnel", wanted);
    said.textContent = "Saved.";
    tunnelFormFilled = false;
    void readStatus().catch(() => {});
  } catch (error) {
    if (quiet(error)) return;
    said.className = "said bad";
    said.textContent = error.message;
  }
});

$("sign-everyone-out").addEventListener("click", async () => {
  const sure = await confirmed({
    title: "Sign every browser out?",
    text: "This one too. To come back you need a new sign-in code from this computer.",
    yes: "Sign every browser out",
  });
  if (!sure) return;
  try {
    await ask("POST", "/api/sessions/end");
  } catch (error) {
    showGate(quiet(error) ? undefined : error.message);
    return;
  }
  showGate();
});

// ── Starting ─────────────────────────────────────────────────────────────────────────────────

async function start() {
  const code = codeInAddress();
  if (code) {
    await redeem(code);
    return;
  }
  try {
    const session = await ask("GET", "/api/session");
    if (session.signedIn) showShell();
    else showGate();
  } catch (error) {
    showGate(quiet(error) ? undefined : error.message);
  }
}

void start();
