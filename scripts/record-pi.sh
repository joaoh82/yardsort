#!/usr/bin/env bash
# Record what a pi-family agent's extension events deliver, as fixtures for the activity adapter.
#
# Pi (badlogic/pi-mono) and OMP (Oh My Pi, its Bun-based fork) share one extension API: a module
# exporting a factory that subscribes with `pi.on(...)`, given for one launch with `-e <file>`.
# This runs the *installed* agent headlessly in a throwaway git repository, with a capture
# extension that records every event it can subscribe to, and `--session-dir` pointed at a
# throwaway so the session lands there and not in your history (credentials stay where they
# are, so the agent stays logged in). Writes, redacted of this machine's paths and with long
# strings cut, under `crates/core/fixtures/<agent>/<version>/`: one file per event, and the
# session file the agent wrote. Spends a few hundred tokens. Re-run it when a new version
# changes a shape; the adapter's tests read these files.
#
# Usage: scripts/record-pi.sh pi|omp [model]
set -euo pipefail

agent="${1:?pi or omp}"
model="${2:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
case "$agent" in
  pi) version="$(pi --version | awk '{print $NF}')" ;;
  omp) version="$(omp --version | sed 's#.*/##')" ;;
  *) echo "pi or omp" >&2; exit 2 ;;
esac
out="$root/crates/core/fixtures/$agent/$version"
work="$(mktemp -d -t "yardsort-$agent.XXXXXX")"
raw="$work/raw"
mkdir -p "$raw" "$work/repo" "$work/sessions"

cat >"$work/capture.ts" <<'EOF'
import fs from "node:fs";
const dir = process.env.YS_CAPTURE_DIR!;
let n = 0;
const EVENTS = [
  "session_start", "session_switch", "session_shutdown", "session_stop",
  "before_agent_start", "agent_start", "agent_end", "agent_before_settle", "agent_settled",
  "turn_start", "turn_end", "message_start", "message_end", "input",
  "tool_call", "tool_result", "tool_execution_start", "tool_execution_end",
  "tool_approval_requested", "tool_approval_resolved",
  "model_select", "model_change", "auto_compaction_start", "auto_compaction_end", "compaction",
];
const trim = (v: any, depth = 0): any =>
  typeof v === "string" ? (v.length > 200 ? v.slice(0, 200) + "…" : v)
  : Array.isArray(v) ? v.slice(0, 8).map((x) => trim(x, depth + 1))
  : v && typeof v === "object" && depth < 8 ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, trim(x, depth + 1)]))
  : v;
function record(event: string, payload: unknown, ctx: any) {
  n += 1;
  const session = {
    id: ctx?.sessionManager?.getSessionId?.(),
    file: ctx?.sessionManager?.getSessionFile?.(),
    cwd: ctx?.cwd,
    model: ctx?.model ? { id: ctx.model.id, provider: ctx.model.provider } : undefined,
    mode: ctx?.mode,
  };
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => k.startsWith("YARDSORT_")));
  fs.writeFileSync(`${dir}/${String(Date.now()).padStart(13, "0")}-${String(n).padStart(3, "0")}.json`,
    JSON.stringify({ event, payload: trim(payload), session, env }));
}
export default function (pi: any) {
  const registered: string[] = [], failed: string[] = [];
  for (const name of EVENTS) {
    try { pi.on(name, (event: unknown, ctx: unknown) => { record(name, event, ctx); }); registered.push(name); }
    catch (e) { failed.push(`${name}: ${String(e).slice(0, 60)}`); }
  }
  fs.writeFileSync(`${dir}/000-registration.json`, JSON.stringify({ registered, failed }));
}
EOF

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

session="11111111-1111-4111-8111-111111111111"
args=(-p --mode json --session-dir "$work/sessions" -e "$work/capture.ts")
[ "$agent" = pi ] && args+=(--session-id "$session")
[ -n "$model" ] && args+=(--model "$model")
echo "recording $agent $version into $out"
(cd "$work/repo" && env YS_CAPTURE_DIR="$raw" \
  YARDSORT_RUN_ID=run-fixture YARDSORT_WORKSPACE_ID=workspace-fixture YARDSORT_SESSION_RECORD_ID=record-fixture \
  "$agent" "${args[@]}" -- "Create a file named hello.txt containing the single word hello. Then read it back. Then read a file named missing.txt, which does not exist. Then run ls. Reply with one word when done." </dev/null) \
  >"$work/$agent.out" 2>"$work/$agent.err" || echo "$agent exited $? (see $work/$agent.err)"
echo "  events captured: $(ls "$raw"/*.json | grep -vc registration)"

rm -rf "$out"
mkdir -p "$out"
python3 - "$raw" "$out" "$work" "$HOME" "$agent" <<'EOF'
import glob, json, os, re, sys
raw, out, work, home, agent = sys.argv[1:6]
ids = {}
def redact(text):
    text = text.replace(work + "/repo", "/tmp/yardsort-fixture/repo").replace(work, "/tmp/yardsort-fixture")
    text = text.replace(home, "/home/user")
    def stable(m):
        v = m.group(0)
        n = ids.setdefault(v, len(ids) + 1)
        return f"22222222-2222-4222-8222-{n:012d}"
    # OMP chooses its own ids; pi's was chosen. Keep them stable and recognisable.
    return re.sub(r"(?<![0-9a-f])(?!11111111-1111)[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}(?![0-9a-f])", stable, text)
n = 0
for name in sorted(os.listdir(raw)):
    if name.startswith("000-"):
        continue
    d = json.load(open(os.path.join(raw, name)))
    n += 1
    label = d["event"]
    tool = (d.get("payload") or {}).get("toolName") if isinstance(d.get("payload"), dict) else None
    if tool:
        label += f"-{tool}"
    with open(os.path.join(out, f"{n:02d}-{label}.json"), "w") as f:
        f.write(redact(json.dumps(d, indent=1)) + "\n")
files = sorted(glob.glob(os.path.join(work, "sessions", "**", "*.jsonl"), recursive=True))
if files:
    lines = []
    for line in open(files[0]):
        try:
            d = json.loads(line)
        except ValueError:
            continue
        def trim(v, depth=0):
            if isinstance(v, str):
                return v if len(v) <= 200 else v[:200] + "…"
            if isinstance(v, list):
                return [trim(x, depth + 1) for x in v[:8]]
            if isinstance(v, dict) and depth < 8:
                return {k: trim(x, depth + 1) for k, x in v.items() if k != "providerPayload"}
            return v
        lines.append(redact(json.dumps(trim(d))))
    with open(os.path.join(out, "session.jsonl"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"session file: {len(lines)} lines")
print(f"wrote {n} event fixtures")
EOF
{
  echo "$agent $version, recorded $(date -u +%Y-%m-%d) with scripts/record-pi.sh $agent${model:+ (model: $model)}."
  echo "One headless turn (-p --mode json) with a capture extension given by -e, sessions under a throwaway --session-dir."
  echo "Each NN-<event>.json is one extension event ({event, payload, session, env}); strings are cut at 200 characters."
  echo "session.jsonl is the session file the agent wrote, trimmed the same way, provider payloads left out."
  echo "Paths and ids are redacted; the session env shows the inherited YARDSORT_* variables."
} >"$out/README.txt"
echo "raw output kept in $work"
