#!/usr/bin/env bash
# Record what the Cursor agent CLI's hooks deliver, as fixtures for the activity adapter.
#
# Cursor's hooks are commands named in a `hooks.json`, given the event as JSON on stdin. A plugin
# directory (`--plugin-dir <dir>`) supplies one for a single process, merged with the user's own.
# This runs the *installed* `cursor-agent` headlessly in a throwaway git repository, with a plugin
# whose hooks capture every event Cursor documents, deciding ones included (they answer nothing,
# so Cursor treats them as no opinion). Writes, redacted of this machine's paths and with long
# strings cut, under `crates/core/fixtures/cursor/<version>/`: one file per hook call. Spends a few
# hundred tokens. Needs `cursor-agent login` done first. Re-run it when a new version changes a
# shape; the adapter's tests read these files once they exist.
#
# Usage: scripts/record-cursor.sh [model]
set -euo pipefail

model="${1:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(cursor-agent --version 2>/dev/null | tail -1 | awk '{print $NF}')"
out="$root/crates/core/fixtures/cursor/$version"
work="$(mktemp -d -t yardsort-cursor.XXXXXX)"
raw="$work/raw"
plugin="$work/plugin"
mkdir -p "$raw" "$work/repo" "$plugin/.cursor-plugin" "$plugin/hooks"

cat >"$work/capture.sh" <<'EOF'
#!/usr/bin/env bash
# One hook call: the event's JSON on stdin, the event's name as $1. Answers nothing.
dir="${YS_CAPTURE_DIR:?}"
cat >"$dir/$(date +%s%N)-$1.json"
env | grep '^YARDSORT_' >"$dir/env.txt" 2>/dev/null || true
exit 0
EOF
chmod +x "$work/capture.sh"

events=(
  sessionStart sessionEnd beforeSubmitPrompt preToolUse postToolUse postToolUseFailure
  beforeShellExecution afterShellExecution beforeMCPExecution afterMCPExecution
  beforeReadFile afterFileEdit beforeTabFileRead subagentStart subagentStop
  preCompact stop afterAgentResponse afterAgentThought
)
python3 - "$plugin" "$work/capture.sh" "${events[@]}" <<'EOF'
import json, sys
plugin, capture, *events = sys.argv[1:]
json.dump({"name": "yardsort-capture", "description": "Records every hook for a fixture.", "hooks": "hooks/hooks.json"},
          open(f"{plugin}/.cursor-plugin/plugin.json", "w"), indent=1)
hooks = {e: [{"type": "command", "command": f"'{capture}' {e}", "timeout": 10}] for e in events}
json.dump({"version": 1, "hooks": hooks}, open(f"{plugin}/hooks/hooks.json", "w"), indent=1)
EOF

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

args=(-p --output-format json --plugin-dir "$plugin" --force)
[ -n "$model" ] && args+=(--model "$model")
echo "recording cursor-agent $version into $out"
(cd "$work/repo" && env YS_CAPTURE_DIR="$raw" \
  YARDSORT_RUN_ID=run-fixture YARDSORT_WORKSPACE_ID=workspace-fixture YARDSORT_SESSION_RECORD_ID=record-fixture \
  cursor-agent "${args[@]}" "Create a file named hello.txt containing the single word hello. Then read it back. Then read a file named missing.txt, which does not exist. Then run ls. Reply with one word when done." </dev/null) \
  >"$work/cursor.out" 2>"$work/cursor.err" || echo "cursor-agent exited $? (see $work/cursor.err)"
echo "  hook calls captured: $(ls "$raw"/*.json 2>/dev/null | wc -l)"
if [ -s "$raw/env.txt" ]; then
  echo "  the launch environment reached the hooks:"; sed 's/^/    /' "$raw/env.txt"
else
  echo "  the launch environment did NOT reach the hooks (no YARDSORT_* variables seen)"
fi

rm -rf "$out"
mkdir -p "$out"
python3 - "$raw" "$out" "$work" "$HOME" <<'EOF'
import json, os, re, sys
raw, out, work, home = sys.argv[1:5]
ids = {}
def redact(text):
    text = text.replace(work + "/repo", "/tmp/yardsort-fixture/repo").replace(work, "/tmp/yardsort-fixture")
    text = text.replace(home, "/home/user")
    def stable(m):
        v = m.group(0)
        n = ids.setdefault(v, len(ids) + 1)
        return f"22222222-2222-4222-8222-{n:012d}"
    return re.sub(r"(?<![0-9a-f])[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(?![0-9a-f])", stable, text)
def trim(v, depth=0):
    if isinstance(v, str):
        return v if len(v) <= 200 else v[:200] + "…"
    if isinstance(v, list):
        return [trim(x, depth + 1) for x in v[:8]]
    if isinstance(v, dict) and depth < 8:
        return {k: trim(x, depth + 1) for k, x in v.items()}
    return v
n = 0
for name in sorted(os.listdir(raw)):
    if not name.endswith(".json"):
        continue
    try:
        d = json.load(open(os.path.join(raw, name)))
    except ValueError:
        continue
    n += 1
    label = d.get("hook_event_name") or name.split("-", 1)[1].removesuffix(".json")
    tool = d.get("tool_name")
    if tool:
        label += f"-{tool}"
    with open(os.path.join(out, f"{n:02d}-{label}.json"), "w") as f:
        f.write(redact(json.dumps(trim(d), indent=1)) + "\n")
print(f"wrote {n} hook fixtures")
EOF
{
  echo "cursor-agent $version, recorded $(date -u +%Y-%m-%d) with scripts/record-cursor.sh${model:+ (model: $model)}."
  echo "One headless turn (-p --output-format json) with a capture plugin given by --plugin-dir."
  echo "Each NN-<event>[-<tool>].json is one hook call's stdin; strings are cut at 200 characters."
  echo "Paths and ids are redacted. The environment the hooks saw is noted above the fixtures in the script's output."
} >"$out/README.txt"
echo "raw output kept in $work"
