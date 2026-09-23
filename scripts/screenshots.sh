#!/usr/bin/env bash
# Retake the screenshots in docs/images/ without leaking anything of your own into them.
#
#   scripts/screenshots.sh setup     # throwaway profile, agent shims, demo projects
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
# See AGENTS.md. `shoot` finds the window by the profile behind it, so an ordinary copy of
# Yardsort left open cannot be captured by mistake. It needs Hyprland, grim and jq; on anything
# else, size the window to the logical equivalent of 1875x1175 and capture it by hand.
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
SHIMMED=(claude codex grok opencode node npm npx git)

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
  echo
  echo "Projects to add, in this order:"
  echo "  $DEMO/repos/weather-cli"
  echo "  $DEMO/repos/api-gateway"
  echo "  $DEMO/repos/docs-site"
  echo
  echo "Now start the app with the throwaway profile:"
  echo
  cat <<EOF
  YARDSORT_DATA_DIR=$SHOT/data \\
  YARDSORT_WORKTREE_ROOT=$DEMO/wt \\
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

# --------------------------------------------------------------------------------------------

# Hyprland works in logical pixels and grim writes device pixels, so the window has to be sized
# WIDTH/scale by HEIGHT/scale for the capture to come out at exactly the size of the set.
shoot() {
  local name=${1:-}
  for tool in hyprctl grim jq; do
    command -v "$tool" >/dev/null || { echo "shoot needs $tool." >&2; exit 1; }
  done
  if [ "$name" != size ] && [ ! -e "docs/images/$name.png" ]; then
    echo "There is no docs/images/$name.png to replace. One of:" >&2
    (cd docs/images && ls *.png | sed 's/\.png$/  /' | tr -d '\n' | sed 's/^/  /') >&2
    echo >&2
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

  # A tiled window cannot be given an exact size, so float it first.
  dispatch "hl.dsp.window.float({ window = \"$window\", action = \"enable\" })" \
    setfloating "$window"
  dispatch "hl.dsp.window.resize({ window = \"$window\", x = $logical_w, y = $logical_h })" \
    resizewindowpixel "exact $logical_w $logical_h,$window"
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

  grim -g "$x,$y ${w}x${h}" "$repo/docs/images/$name.png"
  echo "docs/images/$name.png  $(file -b "$repo/docs/images/$name.png" | grep -o '[0-9]* x [0-9]*' | tr -d ' ')" \
    "  (the set is ${WIDTH}x${HEIGHT})"
}

case "${1:-}" in
  setup) setup ;;
  shoot) shift; shoot "${1:?usage: scripts/screenshots.sh shoot <name>}" ;;
  size) shoot size ;;
  clean) rm -rf "$SHOT" "$AGENTS" "$DEMO"; echo "Removed $SHOT, $AGENTS and $DEMO." ;;
  -h | --help | "") usage ;;
  *) echo "Unknown command: $1" >&2; usage 2 ;;
esac
