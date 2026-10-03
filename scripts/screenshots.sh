#!/usr/bin/env bash
# Retake the screenshots in docs/images/ without leaking anything of your own into them.
#
#   scripts/screenshots.sh setup     # throwaway profile, agent shims, demo projects
#   scripts/screenshots.sh seed      # add the demo projects to the running app's profile
#   scripts/screenshots.sh shoot composer
#   scripts/screenshots.sh size      # measure the window without writing anything
#   scripts/screenshots.sh clean     # remove everything setup made
#
# A throwaway profile (YARDSORT_DATA_DIR, YARDSORT_WORKTREE_ROOT) keeps your projects and
# sessions out of the shots, but two things it does not isolate, and both have leaked before:
#
#   Agent paths. Settings -> Harnesses prints "Found at …", a real path under your home. So
#   `setup` puts shims on PATH and hands the app a stand-in login shell that finds them first —
#   the environment probe runs `$SHELL -i -l -c` (crates/core/src/env.rs), and your own rc files
#   would put the real paths back in front.
#
#   The TypeSafe key. It lives in the OS credential store, which is per user, not per profile,
#   so Settings -> Assist shows the configured state and the key's last characters. Never press
#   Forget to clear it; the launch line below makes the store unreachable for that one process
#   instead. XDG_RUNTIME_DIR is left alone — the Wayland socket is under it.
#
#   The demo repositories point at remotes that do not exist, so `setup` also puts a stand-in
#   `gh` on that PATH. It answers what the Pull requests view, the workflow-run shot and the
#   pull-request badges ask — the list, one pull request in full, the open ones, the comments
#   on lines — from files `setup` writes under the demo folder, and refuses everything else.
#   The pull request the Code tab shows has real commits on a real branch, and the refs the
#   diff is read from are already there, so nothing is fetched.
#
# `seed` writes the three demo projects, and a workspace on the pull request's branch, into the
# profile's database once the app has created it, so nothing has to be typed into the window.
#
# See AGENTS.md. `shoot` finds the window by the profile behind it, so an ordinary copy of
# Yardsort left open cannot be captured by mistake. It needs Hyprland, grim and jq; on anything
# else, size the window to the logical equivalent of 1875x1175 and capture it by hand. `setup`
# needs python3 and `seed` needs sqlite3.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(pwd)

SHOT=/tmp/ys-shot        # the throwaway profile: data dir and the stand-in login shell
AGENTS=/tmp/agents       # shims, so Settings prints "Found at /tmp/agents/claude"
DEMO=/tmp/yardsort-demo  # the projects the screenshots show

# What the images in docs/images/ measure. Keeping every shot the same size is what makes them
# look like one set, and the website lays them out assuming it.
WIDTH=1875
HEIGHT=1175

# Commands the agents need on a PATH that holds nothing else. node and npm are here because the
# demo project's tests are `node --test`, and an agent has nothing but this PATH to run them.
SHIMMED=(claude codex grok opencode omp cursor-agent pi node npm npx git)

# Hyprland 0.56 moved dispatchers to a Lua API and the old argv form stopped parsing, so each
# call is made the new way and falls back to the old one. Omarchy's own scripts do the same.
dispatch() {
  local lua=$1
  shift
  hyprctl dispatch "$lua" >/dev/null 2>&1 && return 0
  hyprctl dispatch "$@" >/dev/null 2>&1 && return 0
  # Without this the script dies on `set -e` with nothing but hyprctl's exit code.
  echo "hyprctl rejected both forms of this dispatch: $lua" >&2
  return 1
}

usage() {
  sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# --------------------------------------------------------------------------------------------

setup() {
  rm -rf "$SHOT" "$AGENTS" "$DEMO"
  mkdir -p "$SHOT/data" "$AGENTS" "$DEMO/repos" "$DEMO/wt"
  # Usage reads agent logs independently of the profile. Never let it read real conversations.
  mkdir -p "$SHOT/claude" "$SHOT/codex" "$SHOT/grok"

  for command in "${SHIMMED[@]}"; do
    local real
    real=$(command -v "$command" 2>/dev/null || true)
    if [ -z "$real" ]; then
      echo "  ! $command is not installed"
      continue
    fi
    real=$(readlink -f "$real")
    printf '#!/bin/sh\nexec "%s" "$@"\n' "$real" >"$AGENTS/$command"
    chmod +x "$AGENTS/$command"
    echo "  shim $AGENTS/$command"
  done

  # The demo repositories' remotes do not exist, so the real `gh` would only ever say so. This
  # one answers from the files `demo_pull_requests` writes for the repository it is run in,
  # which it tells by the remote's name. `pr list --head` is what the workflow-run shot and the
  # badges ask; the rest is the Pull requests view.
  cat >"$AGENTS/gh" <<EOF
#!/bin/sh
DEMO=$DEMO
EOF
  cat >>"$AGENTS/gh" <<'EOF'
repo=$(git remote get-url origin 2>/dev/null | sed 's|.*/||; s|\.git$||')
dir="$DEMO/gh/$repo"
[ -d "$dir" ] || { echo "gh: this is the screenshot stand-in; no demo data for '$repo'" >&2; exit 1; }
# The number after `pr view` or in `pulls/<n>/comments`; empty when there is none.
number=$(printf '%s\n' "$@" | sed -n 's|^repos/.*/pulls/\([0-9]*\)/comments$|\1|p; /^[0-9][0-9]*$/p' | head -1)
case "$1 $2" in
  "pr list")
    branch=
    while [ $# -gt 0 ]; do
      case $1 in --head) branch=$2; shift ;; esac
      shift
    done
    if [ -n "$branch" ]; then
      jq --arg b "$branch" '[.[] | select(.headRefName == $b and .state == "OPEN")]' "$dir/list.json"
    else
      cat "$dir/list.json"
    fi
    ;;
  "pr view") cat "$dir/view-$number.json" ;;
  "api graphql") cat "$dir/open.json" ;;
  "api --paginate")
    if [ -f "$dir/comments-$number.json" ]; then cat "$dir/comments-$number.json"; else echo '[[]]'; fi
    ;;
  *) echo "gh: this is the screenshot stand-in; '$1 $2' is not answered" >&2; exit 1 ;;
esac
EOF
  chmod +x "$AGENTS/gh"
  echo "  stand-in $AGENTS/gh (answers from $DEMO/gh)"

  cat >"$SHOT/loginshell" <<'EOF'
#!/bin/sh
# Stands in for the login shell Yardsort probes, so the shims come first and no path under
# $HOME can reach a screenshot. Four entries, which is what the status bar will read.
PATH=/tmp/agents:/usr/local/bin:/usr/bin:/bin
export PATH
while [ $# -gt 0 ]; do
  case $1 in
    -c) shift; exec /bin/sh -c "$1" ;;
    *) shift ;;
  esac
done
exec /bin/sh
EOF
  chmod +x "$SHOT/loginshell"

  demo_projects
  demo_pull_requests
  echo
  echo "Start the app with the throwaway profile, then run 'scripts/screenshots.sh seed' to add"
  echo "the demo projects to it (or add them by hand, in this order: weather-cli, api-gateway,"
  echo "docs-site, all under $DEMO/repos):"
  echo
  cat <<EOF
  YARDSORT_DATA_DIR=$SHOT/data \\
  YARDSORT_WORKTREE_ROOT=$DEMO/wt \\
  YARDSORT_YS_DIR=$DEMO/bin \\
  CLAUDE_CONFIG_DIR=$SHOT/claude \\
  CODEX_HOME=$SHOT/codex \\
  GROK_HOME=$SHOT/grok \\
  SHELL=$SHOT/loginshell \\
  DBUS_SESSION_BUS_ADDRESS=unix:path=$SHOT/no-bus \\
  just dev
EOF
}

# A git repository with no trace of whoever runs this: a commit list must not carry your name.
demo_repo() {
  local dir="$DEMO/repos/$1"
  mkdir -p "$dir"
  git -C "$dir" init -q -b main
  git -C "$dir" config user.name "Demo"
  git -C "$dir" config user.email "demo@example.com"
  cd "$dir"
}

commit_all() {
  git add -A
  git -c commit.gpgsign=false commit -qm "$1"
}

# The projects the shots show. weather-cli is a real, metric-only CLI with tests, so the task
# the screenshots put an agent to work on — adding imperial units — is one it can actually do.
demo_projects() {
  demo_repo weather-cli
  mkdir -p src tests
  cat >package.json <<'EOF'
{
  "name": "weather-cli",
  "version": "1.2.0",
  "description": "Today's weather for a city, in one line",
  "type": "module",
  "bin": { "weather": "src/cli.js" },
  "scripts": { "test": "node --test" }
}
EOF
  cat >README.md <<'EOF'
# weather-cli

Today's weather for a city, in one line.

```sh
weather Lisbon
# Lisbon: 21°C, wind 12 km/h
```

## Install

```sh
npm install -g weather-cli
```

## How it works

Forecasts come from [Open-Meteo](https://open-meteo.com), which needs no API key. The city is
geocoded first, then the current conditions are read from the forecast endpoint.
EOF
  cat >src/forecast.js <<'EOF'
const GEOCODE = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST = "https://api.open-meteo.com/v1/forecast";

/** The first city Open-Meteo matches for `name`, or null. */
export async function findCity(name) {
  const response = await fetch(`${GEOCODE}?name=${encodeURIComponent(name)}&count=1`);
  if (!response.ok) throw new Error(`Geocoding failed: ${response.status}`);
  const { results } = await response.json();
  return results?.[0] ?? null;
}

/** Current conditions for a city, always in metric: the API is asked for nothing else. */
export async function fetchForecast(city) {
  const query = `latitude=${city.latitude}&longitude=${city.longitude}&current_weather=true`;
  const response = await fetch(`${FORECAST}?${query}`);
  if (!response.ok) throw new Error(`Forecast failed: ${response.status}`);
  const { current_weather: current } = await response.json();
  return { city: city.name, temperature: current.temperature, wind: current.windspeed };
}
EOF
  cat >src/render.js <<'EOF'
/** One line: the city, its temperature and its wind speed. */
export function render(forecast) {
  const temperature = Math.round(forecast.temperature);
  const wind = Math.round(forecast.wind);
  return `${forecast.city}: ${temperature}°C, wind ${wind} km/h`;
}
EOF
  cat >src/cli.js <<'EOF'
#!/usr/bin/env node
import { fetchForecast, findCity } from "./forecast.js";
import { render } from "./render.js";

const name = process.argv.slice(2).join(" ").trim();
if (!name) {
  console.error("usage: weather <city>");
  process.exit(1);
}

const city = await findCity(name);
if (!city) {
  console.error(`No such city: ${name}`);
  process.exit(1);
}
console.log(render(await fetchForecast(city)));
EOF
  cat >tests/render.test.js <<'EOF'
import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "../src/render.js";

const lisbon = { city: "Lisbon", temperature: 21.4, wind: 11.6 };

test("renders a forecast in one line", () => {
  assert.equal(render(lisbon), "Lisbon: 21°C, wind 12 km/h");
});

test("rounds to whole units", () => {
  assert.equal(render({ ...lisbon, temperature: 20.5 }), "Lisbon: 21°C, wind 12 km/h");
});
EOF
  commit_all "Read the wind speed from the current conditions"

  demo_repo api-gateway
  printf '# api-gateway\n\nRoutes public traffic to the internal services.\n' >README.md
  commit_all "Initial commit"

  demo_repo docs-site
  printf '# docs-site\n\nThe documentation site.\n' >README.md
  commit_all "Initial commit"
}

# The pull requests the Pull requests view shows: a handful across the three projects, in every
# state the view can show, and one with real commits on a real branch so that its Code tab reads
# a diff. The remotes are named but do not exist; the stand-in `gh` answers for them, and the
# refs the diff is read from are set here so git has nothing to fetch.
demo_pull_requests() {
  local name
  for name in weather-cli api-gateway docs-site; do
    git -C "$DEMO/repos/$name" remote add origin "https://github.com/yardsort-demo/$name.git"
  done

  # Pull request #12's branch: a converter and its tests, as the agent in the shots would write
  # it. The commit is made in a worktree on the branch, never in the main checkout: which
  # workspace a pull request belongs to is read from git's own history of what was checked out
  # and committed where, and `seed` adds a workspace on this worktree so the row links to it.
  mkdir -p "$DEMO/wt/weather-cli"
  git -C "$DEMO/repos/weather-cli" worktree add -q -b ys/celsius-fahrenheit \
    "$DEMO/wt/weather-cli/celsius-fahrenheit" main
  cd "$DEMO/wt/weather-cli/celsius-fahrenheit"
  git config user.name "Demo"
  git config user.email "demo@example.com"
  cat >src/units.js <<'EOF'
/** The unit systems the CLI can print in. Metric is what the API answers in. */
export const UNITS = ["metric", "imperial"];

/** Degrees Fahrenheit for a Celsius reading, unrounded: rendering rounds last. */
export function toFahrenheit(celsius) {
  return (celsius * 9) / 5 + 32;
}

/** Miles per hour for a wind speed in km/h. */
export function toMilesPerHour(kmh) {
  return kmh / 1.609344;
}

/** A forecast in `units`, converting from the metric one the API gave. */
export function convert(forecast, units) {
  if (units === "metric") return forecast;
  return {
    ...forecast,
    temperature: toFahrenheit(forecast.temperature),
    wind: toMilesPerHour(forecast.wind),
  };
}
EOF
  cat >tests/units.test.js <<'EOF'
import assert from "node:assert/strict";
import { test } from "node:test";
import { convert, toFahrenheit, toMilesPerHour } from "../src/units.js";

test("converts freezing and boiling points", () => {
  assert.equal(toFahrenheit(0), 32);
  assert.equal(toFahrenheit(100), 212);
});

test("converts wind speed to miles per hour", () => {
  assert.equal(Math.round(toMilesPerHour(16.09344)), 10);
});

test("leaves a metric forecast alone", () => {
  const lisbon = { city: "Lisbon", temperature: 21.4, wind: 11.6 };
  assert.equal(convert(lisbon, "metric"), lisbon);
});
EOF
  cat >src/render.js <<'EOF'
/** One line: the city, its temperature and its wind speed, in the units asked for. */
export function render(forecast, units = "metric") {
  const temperature = Math.round(forecast.temperature);
  const wind = Math.round(forecast.wind);
  if (units === "imperial") return `${forecast.city}: ${temperature}°F, wind ${wind} mph`;
  return `${forecast.city}: ${temperature}°C, wind ${wind} km/h`;
}
EOF
  commit_all "Add a Celsius/Fahrenheit converter and imperial output"
  local head base
  head=$(git rev-parse HEAD)
  base=$(git rev-parse main)
  git update-ref refs/yardsort/pull/12/head "$head"
  git update-ref refs/yardsort/pull/12/base "$base"
  cd "$repo"

  # What `gh` answers, per repository, in the shapes gh 2.102.0 prints (see
  # crates/core/fixtures/gh/2.102.0). Times are relative to now so the view's "3 h ago" reads
  # right whenever the shots are taken. Names are historical, not anyone's account.
  mkdir -p "$DEMO/gh"
  HEAD_OID=$head BASE_OID=$base DEMO="$DEMO" python3 - <<'EOF'
import json, os
from datetime import datetime, timedelta, timezone

demo = os.environ["DEMO"]
head, base = os.environ["HEAD_OID"], os.environ["BASE_OID"]
now = datetime.now(timezone.utc)
ago = lambda hours: (now - timedelta(hours=hours)).strftime("%Y-%m-%dT%H:%M:%SZ")
oid = lambda n: ("%02x" % n) * 20
user = lambda login: {"id": f"U_{login}", "is_bot": False, "login": login, "name": login.title()}

def check(name, workflow, state, number, done=True):
    conclusion = {"ok": "SUCCESS", "fail": "FAILURE", "run": None, "skip": "SKIPPED"}[state]
    return {
        "__typename": "CheckRun",
        "name": name,
        "workflowName": workflow,
        "status": "COMPLETED" if conclusion else "IN_PROGRESS",
        "conclusion": conclusion,
        "startedAt": ago(2),
        "completedAt": ago(1) if conclusion else None,
        "detailsUrl": f"https://github.com/yardsort-demo/{workflow.lower()}/actions/runs/{number}/job/1",
    }

def review(login, state, body="", hours=1):
    return {
        "id": f"PRR_{login}_{state}",
        "author": {"login": login},
        "authorAssociation": "MEMBER",
        "body": body,
        "submittedAt": ago(hours),
        "includesCreatedEdit": False,
        "reactionGroups": [],
        "state": state,
        "commit": {"oid": ""},
    }

def comment(login, body, number, hours):
    return {
        "id": f"IC_{number}_{login}",
        "author": {"login": login},
        "authorAssociation": "MEMBER",
        "body": body,
        "createdAt": ago(hours),
        "includesCreatedEdit": False,
        "isMinimized": False,
        "minimizedReason": "",
        "reactionGroups": [],
        "url": f"https://github.com/yardsort-demo/x/pull/{number}#issuecomment-{number}",
        "viewerDidAuthor": login == "ada",
    }

def pr(repo, number, title, branch, author, state, hours, *, draft=False, checks=(),
       decision="", requests=(), reviews=(), adds=0, dels=0, mergeable="MERGEABLE",
       head_oid=None, base_oid=None, body="", comments=(), full_reviews=(), files=1,
       updated=None):
    return {
        "number": number,
        "url": f"https://github.com/yardsort-demo/{repo}/pull/{number}",
        "title": title,
        "headRefName": branch,
        "baseRefName": "main",
        "state": state,
        "isDraft": draft,
        "author": user(author),
        "createdAt": ago(hours),
        "updatedAt": ago(updated if updated is not None else max(hours - 1, 0.5)),
        "headRefOid": head_oid or oid(number),
        "baseRefOid": base_oid or oid(200 + number),
        "additions": adds,
        "deletions": dels,
        "reviewDecision": decision,
        "reviewRequests": [{"__typename": "User", "login": login} for login in requests],
        "latestReviews": [review(*r) for r in reviews],
        "isCrossRepository": False,
        "mergeable": mergeable,
        "statusCheckRollup": [check(*c, number) for c in checks],
        "body": body,
        "comments": [comment(*c, number) for c in comments],
        "reviews": [review(*r) for r in full_reviews] or [review(*r) for r in reviews],
        "changedFiles": files,
    }

repos = {
    "weather-cli": [
        pr("weather-cli", 12, "Add a Celsius/Fahrenheit converter and imperial output",
           "ys/celsius-fahrenheit", "ada", "OPEN", 5, updated=0.7,
           checks=[("test", "CI", "ok"), ("lint", "CI", "ok"), ("package (macOS)", "CI", "run")],
           decision="REVIEW_REQUIRED", requests=["hedy"], reviews=[("grace", "COMMENTED")],
           adds=48, dels=3, head_oid=head, base_oid=base, files=3,
           body="## What\n\n`weather Lisbon --units imperial` prints °F and mph. The API is still "
                "asked for metric; the conversion happens before rendering, and rendering rounds "
                "last, as it did.\n\n## How to try\n\n```sh\nweather Lisbon\n"
                "weather Lisbon --units imperial\n```\n\n- [x] converter with tests\n"
                "- [x] `render` takes the unit system\n- [ ] README",
           comments=[("grace", "Tried it: `weather Lisbon --units imperial` gives "
                      "`Lisbon: 70°F, wind 7 mph`. One question on the rounding, on the line.", 1.5)],
           full_reviews=[("grace", "COMMENTED", "", 1.4)]),
        pr("weather-cli", 11, "Cache geocoding results for an hour", "ys/geocode-cache", "hedy",
           "OPEN", 26, draft=True, checks=[("test", "CI", "run")], adds=61, dels=4,
           body="Keeps the last lookups in `~/.cache/weather-cli`. Still deciding on the file format."),
        pr("weather-cli", 9, "Round wind speed to whole km/h", "ys/round-wind", "ada", "MERGED", 70,
           checks=[("test", "CI", "ok"), ("lint", "CI", "ok")], decision="APPROVED",
           reviews=[("grace", "APPROVED")], adds=6, dels=2,
           body="`11.6 km/h` read as false precision for a forecast."),
        pr("weather-cli", 8, "Try the bulk forecast endpoint", "ys/bulk-endpoint", "radia", "CLOSED",
           150, checks=[("test", "CI", "fail")], adds=120, dels=40,
           body="Closed: it is not faster for one city, which is all the CLI asks for."),
    ],
    "api-gateway": [
        pr("api-gateway", 41, "Rate limit per API key", "ys/rate-limit", "edsger", "OPEN", 50,
           checks=[("test", "CI", "fail"), ("lint", "CI", "ok")], decision="CHANGES_REQUESTED",
           reviews=[("ada", "CHANGES_REQUESTED")], adds=210, dels=18,
           body="A token bucket per key, 600 requests a minute, in Redis."),
        pr("api-gateway", 40, "Retry upstream timeouts once", "ys/retry-timeouts", "ada", "OPEN", 7,
           checks=[("test", "CI", "ok"), ("lint", "CI", "ok")], decision="APPROVED",
           reviews=[("grace", "APPROVED")], adds=34, dels=9,
           body="One retry, only for idempotent methods, with a 200 ms backoff."),
        pr("api-gateway", 38, "Pin the base image to a digest", "ys/pin-image", "grace", "MERGED",
           100, checks=[("test", "CI", "ok")], decision="APPROVED", reviews=[("ada", "APPROVED")],
           adds=1, dels=1, body="So a rebuild is the same build."),
    ],
    "docs-site": [
        pr("docs-site", 7, "Write the install page for Windows", "ys/windows-install", "radia",
           "OPEN", 3, draft=True, adds=88, dels=0,
           body="Draft. Screenshots still to take."),
        pr("docs-site", 6, "Fix the broken links on the FAQ", "ys/faq-links", "ada", "MERGED", 45,
           checks=[("build", "Site", "ok")], decision="APPROVED", reviews=[("hedy", "APPROVED")],
           adds=4, dels=4, body="Three anchors moved with the last restructure."),
    ],
}

counts = ["ACTION_REQUIRED", "CANCELLED", "COMPLETED", "FAILURE", "IN_PROGRESS", "NEUTRAL",
          "PENDING", "QUEUED", "SKIPPED", "STALE", "STARTUP_FAILURE", "SUCCESS", "TIMED_OUT",
          "WAITING"]

def open_node(p):
    tally = {state: 0 for state in counts}
    for c in p["statusCheckRollup"]:
        tally[c["conclusion"] or "IN_PROGRESS"] += 1
    node = {k: p[k] for k in ("number", "url", "title", "isDraft", "state", "createdAt",
                              "updatedAt", "headRefName", "headRefOid", "baseRefName",
                              "isCrossRepository", "additions", "deletions", "mergeable",
                              "reviewDecision")}
    node["author"] = {"login": p["author"]["login"]}
    node["reviewRequests"] = {"nodes": [{"requestedReviewer": {"__typename": "User", "login": r["login"]}}
                                        for r in p["reviewRequests"]]}
    node["latestReviews"] = {"nodes": [{"state": r["state"], "author": r["author"]}
                                       for r in p["latestReviews"]]}
    rollup = None
    if p["statusCheckRollup"]:
        rollup = {"contexts": {
            "checkRunCountsByState": [{"state": s, "count": tally[s]} for s in counts],
            "statusContextCountsByState": [],
        }}
    node["commits"] = {"nodes": [{"commit": {"statusCheckRollup": rollup}}]}
    return node

line_comments = {
    ("weather-cli", 12): [
        {"id": 1201, "in_reply_to_id": None, "login": "grace", "path": "src/units.js",
         "line": 7, "start_line": None, "side": "RIGHT", "hours": 1.4,
         "body": "Does this want rounding before the `+ 32`? 20.5°C prints 69°F here and the "
                 "metric line rounds the reading first."},
        {"id": 1202, "in_reply_to_id": 1201, "login": "ada", "path": "src/units.js",
         "line": 7, "start_line": None, "side": "RIGHT", "hours": 0.7,
         "body": "Rounding after the conversion is the more accurate of the two, and `render` "
                 "rounds last on the metric path as well. Keeping it, but the docstring should "
                 "say so."},
        {"id": 1203, "in_reply_to_id": None, "login": "grace", "path": "src/render.js",
         "line": 5, "start_line": 2, "side": "RIGHT", "hours": 1.3,
         "body": "Two template strings that differ in two characters. Worth one with the unit "
                 "names looked up?"},
    ],
}

for repo, prs in repos.items():
    out = os.path.join(demo, "gh", repo)
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "list.json"), "w") as f:
        json.dump(prs, f, indent=1)
    for p in prs:
        with open(os.path.join(out, f"view-{p['number']}.json"), "w") as f:
            json.dump(p, f, indent=1)
    opened = [open_node(p) for p in prs if p["state"] == "OPEN"]
    with open(os.path.join(out, "open.json"), "w") as f:
        json.dump({"data": {
            "viewer": {"login": "ada"},
            "repository": {"pullRequests": {
                "totalCount": len(opened),
                "pageInfo": {"hasNextPage": False, "endCursor": None},
                "nodes": opened,
            }},
        }}, f, indent=1)
    for (r, number), comments in line_comments.items():
        if r != repo:
            continue
        rows = []
        for c in comments:
            row = {
                "url": f"https://api.github.com/repos/yardsort-demo/{repo}/pulls/comments/{c['id']}",
                "pull_request_review_id": 4000 + c["id"],
                "id": c["id"],
                "node_id": f"PRRC_{c['id']}",
                "diff_hunk": "",
                "path": c["path"],
                "commit_id": head,
                "original_commit_id": head,
                "user": {"login": c["login"], "id": 1},
                "body": c["body"],
                "created_at": ago(c["hours"]),
                "updated_at": ago(c["hours"]),
                "html_url": f"https://github.com/yardsort-demo/{repo}/pull/{number}#discussion_r{c['id']}",
                "pull_request_url": f"https://api.github.com/repos/yardsort-demo/{repo}/pulls/{number}",
                "_links": {},
                "reactions": {"total_count": 0},
                "start_line": c["start_line"],
                "original_start_line": c["start_line"],
                "start_side": "RIGHT" if c["start_line"] else None,
                "line": c["line"],
                "original_line": c["line"],
                "side": c["side"],
                "author_association": "MEMBER",
                "original_position": 1,
                "position": 1,
                "subject_type": "line",
            }
            if c["in_reply_to_id"]:
                row["in_reply_to_id"] = c["in_reply_to_id"]
            rows.append(row)
        with open(os.path.join(out, f"comments-{number}.json"), "w") as f:
            json.dump([rows], f, indent=1)
print("  pull requests for weather-cli, api-gateway and docs-site under", os.path.join(demo, "gh"))
EOF
}

# The demo projects, into the profile the app is running on. The app makes the database on its
# first start, so this comes after that; the sidebar reads the projects again when the window
# is next focused, so nothing needs restarting.
seed() {
  command -v sqlite3 >/dev/null || { echo "seed needs sqlite3." >&2; exit 1; }
  local db="$SHOT/data/yardsort.db"
  if [ ! -f "$db" ]; then
    echo "No database at $db yet: start the app with the line 'setup' printed first." >&2
    exit 1
  fi
  if [ "$(sqlite3 "$db" "SELECT count(*) FROM projects WHERE removed = 0")" != 0 ]; then
    echo "The profile already has projects; nothing added." >&2
    exit 1
  fi
  local now
  now=$(date +%s000)
  sqlite3 "$db" <<EOF
INSERT INTO projects (id, name, root_path, sort_order, created_at) VALUES
  ('demo-weather-cli', 'weather-cli', '$DEMO/repos/weather-cli', 0, $now),
  ('demo-api-gateway', 'api-gateway', '$DEMO/repos/api-gateway', 1, $now),
  ('demo-docs-site', 'docs-site', '$DEMO/repos/docs-site', 2, $now);
INSERT INTO workspaces (id, project_id, kind, name, path, branch, base_branch, created_at) VALUES
  ('demo-weather-cli-local', 'demo-weather-cli', 'local', 'local', '$DEMO/repos/weather-cli', NULL, NULL, $now),
  ('demo-weather-cli-pr', 'demo-weather-cli', 'worktree', 'celsius-fahrenheit', '$DEMO/wt/weather-cli/celsius-fahrenheit', 'ys/celsius-fahrenheit', 'main', $now),
  ('demo-api-gateway-local', 'demo-api-gateway', 'local', 'local', '$DEMO/repos/api-gateway', NULL, NULL, $now),
  ('demo-docs-site-local', 'demo-docs-site', 'local', 'local', '$DEMO/repos/docs-site', NULL, NULL, $now);
-- The welcome tour wants no screenshot of its own but the onboarding one, taken by hand.
INSERT OR REPLACE INTO ui_state (key, value) VALUES ('onboarding.welcomeSeen', 'true');
EOF
  echo "Added weather-cli, api-gateway and docs-site to $db, and marked the welcome tour seen."
  echo "Focus the window to see them."
}

# --------------------------------------------------------------------------------------------

# Hyprland works in logical pixels and grim writes device pixels, so the window has to be sized
# WIDTH/scale by HEIGHT/scale for the capture to come out at exactly the size of the set.
shoot() {
  local name=${1:-}
  for tool in hyprctl grim jq; do
    command -v "$tool" >/dev/null || { echo "shoot needs $tool." >&2; exit 1; }
  done
  if [[ ! "$name" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
    echo "Use a screenshot name containing only lowercase letters, digits and hyphens." >&2
    exit 1
  fi

  # Which window to shoot is decided by the profile behind it, never by the title or by which
  # one happens to be focused: an ordinary copy of Yardsort is also called "Yardsort", and one
  # was captured that way once — real projects, real paths and all.
  local pid candidate
  for candidate in $(hyprctl clients -j | jq -r '.[] | select(.title == "Yardsort") | .pid'); do
    if tr '\0' '\n' <"/proc/$candidate/environ" 2>/dev/null |
      grep -qxF "YARDSORT_DATA_DIR=$SHOT/data"; then
      pid=$candidate
      break
    fi
  done
  if [ -z "${pid:-}" ]; then
    echo "No Yardsort window is running on the throwaway profile ($SHOT/data)." >&2
    echo "Run 'scripts/screenshots.sh setup' and start the app with the line it prints." >&2
    exit 1
  fi

  local scale logical_w logical_h window
  scale=$(hyprctl monitors -j | jq -r '[.[] | select(.focused)][0].scale')
  logical_w=$(awk -v w="$WIDTH" -v s="$scale" 'BEGIN { printf "%d", w / s }')
  logical_h=$(awk -v h="$HEIGHT" -v s="$scale" 'BEGIN { printf "%d", h / s }')
  window="address:$(hyprctl clients -j | jq -r ".[] | select(.pid == $pid) | .address")"

  # grim captures a region of the screen, not a window, so whatever is on top is what lands in
  # the file. Raise the target first.
  dispatch "hl.dsp.focus({ window = \"$window\" })" focuswindow "$window"

  # Omarchy gives every window a little transparency, and through it whatever is behind — a
  # terminal with your own work in it — shows faintly in every dark area of the shot. A rule
  # makes Yardsort windows opaque for this session; the next config reload drops it. Older
  # Hyprlands have no `eval`, and there the fallback is to put nothing behind the window.
  hyprctl eval 'hl.window_rule({ match = { title = "^Yardsort$" }, opacity = "1.0 1.0" })' \
    >/dev/null 2>&1 || echo "Note: could not make the window opaque; keep an empty workspace behind it." >&2

  # A tiled window cannot be given an exact size, so float it first.
  dispatch "hl.dsp.window.float({ window = \"$window\", action = \"enable\" })" \
    setfloating "$window"
  dispatch "hl.dsp.window.resize({ window = \"$window\", x = $logical_w, y = $logical_h })" \
    resizewindowpixel "exact $logical_w $logical_h,$window"
  dispatch "hl.dsp.window.center({ window = \"$window\" })" centerwindow "$window"
  sleep 0.5

  local x y w h
  read -r x y w h < <(hyprctl clients -j |
    jq -r ".[] | select(.pid == $pid) | \"\(.at[0]) \(.at[1]) \(.size[0]) \(.size[1])\"")
  if [ "$w" != "$logical_w" ] || [ "$h" != "$logical_h" ]; then
    echo "Note: the window measures ${w}x${h}, not ${logical_w}x${logical_h}." >&2
    echo "A maximised or fullscreen window cannot be given an exact size — leave it a normal" >&2
    echo "window and run this again." >&2
  fi

  if [ "$name" = size ]; then
    awk -v x="$x" -v y="$y" -v w="$w" -v h="$h" -v s="$scale" \
      'BEGIN { printf "at %d,%d  %dx%d logical  ->  %dx%d captured\n", x, y, w, h, w * s, h * s }'
    return
  fi

  # Keep the pointer, chart hover cards and tooltips out of the capture: below the window when
  # the screen has room there, else in its bottom-left corner, where nothing reacts to a hover.
  local screen_h cx cy
  screen_h=$(hyprctl monitors -j | jq -r '[.[] | select(.focused)][0].height')
  cx=$((x + w / 2))
  if [ $((y + h + 24)) -lt "$(awk -v h="$screen_h" -v s="$scale" 'BEGIN { printf "%d", h / s }')" ]; then
    cy=$((y + h + 24))
  else
    cx=$((x + 20))
    cy=$((y + h - 80))
  fi
  dispatch "hl.dsp.cursor.move({ x = $cx, y = $cy })" movecursor "$cx" "$cy"
  sleep 0.2
  grim -g "$x,$y ${w}x${h}" "$repo/docs/images/$name.png"
  echo "docs/images/$name.png  $(file -b "$repo/docs/images/$name.png" | grep -o '[0-9]* x [0-9]*' | tr -d ' ')" \
    "  (the set is ${WIDTH}x${HEIGHT})"
}

case "${1:-}" in
  setup) setup ;;
  seed) seed ;;
  shoot) shift; shoot "${1:?usage: scripts/screenshots.sh shoot <name>}" ;;
  size) shoot size ;;
  clean) rm -rf "$SHOT" "$AGENTS" "$DEMO"; echo "Removed $SHOT, $AGENTS and $DEMO." ;;
  -h | --help | "") usage ;;
  *) echo "Unknown command: $1" >&2; usage 2 ;;
esac
