#!/usr/bin/env bash
# Record what OpenCode's plugin hooks deliver, as fixtures for the activity adapter.
#
# Runs the *installed* `opencode run` in a throwaway git repository under a throwaway data home
# (`XDG_DATA_HOME`, so the session it writes lands there and not in your history; OpenCode's
# credentials live elsewhere, so it stays logged in), with a capture plugin given for that run
# alone through `OPENCODE_CONFIG_CONTENT` — the same channel Yardsort's own plugin uses. Every
# hook call and bus event the plugin sees is written, redacted of this machine's paths and ids,
# under `crates/core/fixtures/opencode/<version>/`, except the streaming and catalogue noise
# nobody reads. Spends a few hundred tokens on the account `opencode` uses. Re-run it when a new
# OpenCode version changes a shape; the adapter's tests read these files.
#
# Usage: scripts/record-opencode.sh [provider/model]
set -euo pipefail

model="${1:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(opencode --version | tr -d '[:space:]')"
out="$root/crates/core/fixtures/opencode/$version"
work="$(mktemp -d -t yardsort-opencode.XXXXXX)"
raw="$work/raw"
mkdir -p "$raw" "$work/repo" "$work/data"

cat >"$work/capture.js" <<'EOF'
import fs from "node:fs";
const dir = process.env.YS_CAPTURE_DIR;
let n = 0;
let directory;
function record(hook, payload) {
  n += 1;
  const stamp = `${String(Date.now()).padStart(13, "0")}-${String(n).padStart(3, "0")}`;
  fs.writeFileSync(`${dir}/${stamp}.json`, JSON.stringify({ hook, payload, directory }));
}
export const YardsortCapture = async (ctx) => {
  directory = ctx.directory;
  record("init", {
    keys: Object.keys(ctx),
    directory: ctx.directory,
    worktree: ctx.worktree,
    env: Object.fromEntries(Object.entries(process.env).filter(([k]) => k.startsWith("YARDSORT_"))),
  });
  const pass = (hook) => async (input, output) => record(hook, { input, output });
  return {
    event: async ({ event }) => record("event", event),
    "tool.execute.before": pass("tool.execute.before"),
    "tool.execute.after": pass("tool.execute.after"),
    "permission.ask": pass("permission.ask"),
    "chat.message": pass("chat.message"),
    "shell.env": pass("shell.env"),
  };
};
EOF

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

model_args=()
[ -n "$model" ] && model_args=(-m "$model")
echo "recording OpenCode $version into $out"
(cd "$work/repo" && env XDG_DATA_HOME="$work/data" YS_CAPTURE_DIR="$raw" \
  YARDSORT_RUN_ID=run-fixture YARDSORT_WORKSPACE_ID=workspace-fixture YARDSORT_SESSION_RECORD_ID=record-fixture \
  OPENCODE_CONFIG_CONTENT="{\"plugin\":[\"file://$work/capture.js\"]}" \
  opencode run --format json "${model_args[@]}" \
  "Create a file named hello.txt containing the single word hello using the write tool. Then read it back with the read tool. Then read a file named missing.txt, which does not exist. Then run \`ls\` with the bash tool. Reply with one word when done." </dev/null) \
  >"$work/opencode.out" 2>"$work/opencode.err" || echo "opencode exited $? (see $work/opencode.err)"
echo "  captured: $(ls "$raw"/*.json | wc -l) calls"

rm -rf "$out"
mkdir -p "$out"
python3 - "$raw" "$out" "$work" "$HOME" <<'EOF'
import json, os, re, sys
raw, out, work, home = sys.argv[1:5]
NOISE = {"plugin.added", "catalog.updated", "reference.updated", "integration.updated",
         "message.part.delta", "session.diff", "file.watcher.updated"}
ids = {}
def redact(text):
    text = text.replace(work + "/repo", "/tmp/yardsort-fixture/repo").replace(work, "/tmp/yardsort-fixture")
    text = text.replace(home, "/home/user")
    # OpenCode's ids are random; keep them stable and recognisable across re-recordings.
    def stable(m):
        kind, value = m.group(1), m.group(0)
        n = ids.setdefault(value, len([k for k in ids if k.startswith(kind + "_")]) + 1)
        return f"{kind}_{'fixture':0>18}{n:02d}"[:len(value)] if len(value) > 20 else value
    return re.sub(r"\b(ses|msg|prt)_[0-9a-zA-Z]{20,}", stable, text)
n = 0
for name in sorted(os.listdir(raw)):
    d = json.load(open(os.path.join(raw, name)))
    hook, payload = d["hook"], d["payload"]
    if hook == "event":
        kind = payload.get("type", "unknown")
        if kind in NOISE:
            continue
        if kind == "message.part.updated":
            part = payload.get("properties", {}).get("part", {})
            # Deltas of text and reasoning are content; a tool's states, a step's tokens are not.
            if part.get("type") in ("text", "reasoning"):
                continue
            detail = part.get("type", "part")
            if part.get("type") == "tool":
                detail = f"tool-{part.get('tool')}-{(part.get('state') or {}).get('status')}"
        elif kind == "message.updated":
            info = payload.get("properties", {}).get("info", {})
            detail = info.get("role", "message") + ("-completed" if (info.get("time") or {}).get("completed") else "")
        else:
            detail = ""
        label = f"event-{kind}" + (f"-{detail}" if detail else "")
    else:
        tool = ((payload.get("input") or {}).get("tool"))
        label = hook + (f"-{tool}" if tool else "")
    n += 1
    with open(os.path.join(out, f"{n:02d}-{label}.json"), "w") as f:
        f.write(redact(json.dumps(d, indent=1)) + "\n")
print(f"wrote {n} fixtures")
EOF
{
  echo "OpenCode $version, recorded $(date -u +%Y-%m-%d) with scripts/record-opencode.sh${model:+ (model: $model)}."
  echo "One 'opencode run' turn under a throwaway XDG_DATA_HOME, the capture plugin given through OPENCODE_CONFIG_CONTENT."
  echo "Each file is one plugin hook call or bus event ({hook, payload, directory}); streaming deltas, text and reasoning parts, and catalogue events are left out."
  echo "Paths and ids are redacted; 01-init.json shows the inherited YARDSORT_* variables."
} >"$out/README.txt"
echo "raw output kept in $work"
