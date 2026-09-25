#!/usr/bin/env bash
# Record what Codex CLI leaves behind and what its hooks deliver, as fixtures for the activity
# adapter.
#
# Runs the *installed* `codex` non-interactively in a throwaway git repository under a throwaway
# `CODEX_HOME` — the real one's `auth.json` symlinked in, nothing else — so the session it writes
# lands in the throwaway and not in your history, and a `hooks.json` there can point every hook
# event at a capture script without touching yours. Writes, redacted of this machine's paths,
# under `crates/core/fixtures/codex/<version>/`: the session's rollout file (`rollout.jsonl`,
# with the model's instructions and reasoning stripped: they are not what an adapter reads), and
# each hook payload under `hooks/`. Spends a few hundred tokens on the account `codex` is logged
# in to. Re-run it when a new Codex version changes a shape; the adapter's tests read these files.
#
# Usage: scripts/record-codex.sh [model]        (default: the account's default)
set -euo pipefail

model="${1:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(codex --version | awk '{print $2}')"
out="$root/crates/core/fixtures/codex/$version"
work="$(mktemp -d -t yardsort-codex.XXXXXX)"
home="$work/codex-home"
capture="$work/capture.sh"
raw="$work/raw"
mkdir -p "$raw" "$work/repo" "$home"
ln -s "${CODEX_HOME:-$HOME/.codex}/auth.json" "$home/auth.json"
trap 'rm -f "$home/auth.json"' EXIT

cat >"$capture" <<'EOF'
#!/usr/bin/env bash
dir="$1"
stamp="$(date +%s%N)"
cat >"$dir/$stamp.json"
env | grep -E '^(YARDSORT_|CODEX_)' | sort >"$dir/$stamp.env" || true
exit 0
EOF
chmod +x "$capture"
# The `notify` program gets its payload as the last argument, not on stdin.
notify="$work/notify.sh"
cat >"$notify" <<'EOF2'
#!/usr/bin/env bash
dir="$1"
stamp="$(date +%s%N)"
printf '%s' "${@: -1}" >"$dir/$stamp.json"
env | grep -E '^(YARDSORT_|CODEX_)' | sort >"$dir/$stamp.env" || true
exit 0
EOF2
chmod +x "$notify"
mkdir -p "$raw-notify"

# Claude Code's hook names, which Codex's hooks.json is shaped after, plus a few guesses; an
# event Codex does not know is simply never delivered.
events=(SessionStart SessionEnd UserPromptSubmit PreToolUse PostToolUse PostToolUseFailure
  PermissionRequest Notification Stop SubagentStart SubagentStop TurnStart TurnEnd)
hooks=""
for event in "${events[@]}"; do
  hooks+="\"$event\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"$capture $raw\", \"timeout\": 10}]}],"
done
printf '{"hooks": {%s}}\n' "${hooks%,}" >"$home/hooks.json"

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

clean=(env CODEX_HOME="$home" YARDSORT_RUN_ID=run-fixture YARDSORT_WORKSPACE_ID=workspace-fixture
  YARDSORT_SESSION_RECORD_ID=record-fixture)
model_args=()
[ -n "$model" ] && model_args=(-m "$model")

echo "recording Codex $version into $out"
(cd "$work/repo" && "${clean[@]}" codex exec --skip-git-repo-check -s workspace-write \
  --dangerously-bypass-hook-trust "${model_args[@]}" \
  -c "notify=[\"$notify\", \"$raw-notify\"]" \
  "Use apply_patch to create a file named hello.txt containing the single word hello. Then read it back with cat. Then read a file named missing.txt, which does not exist. Then run \`ls\`. Reply with one word when done." </dev/null) \
  >"$work/codex.out" 2>"$work/codex.err" || echo "codex exited $? (see $work/codex.err)"
echo "  hooks delivered: $(ls "$raw"/*.json 2>/dev/null | wc -l), notify calls: $(ls "$raw-notify"/*.json 2>/dev/null | wc -l)"
rollout="$(find "$home/sessions" -name '*.jsonl' | head -1 || true)"
echo "  rollout: ${rollout:-none}"

rm -rf "$out"
mkdir -p "$out/hooks"
redact() {
  sed -e "s#$work/repo#/tmp/yardsort-fixture/repo#g" \
    -e "s#$home#/tmp/yardsort-fixture/codex-home#g" \
    -e "s#$work#/tmp/yardsort-fixture#g" \
    -e "s#$HOME#/home/user#g"
}
if [ -n "$rollout" ]; then
  python3 - "$rollout" <<'EOF' | redact >"$out/rollout.jsonl"
import json, sys
for line in open(sys.argv[1]):
    try:
        d = json.loads(line)
    except ValueError:
        continue
    t = d.get("type"); p = d.get("payload")
    if t == "world_state":
        continue
    if t == "session_meta" and isinstance(p, dict):
        p["base_instructions"] = "<stripped>"
    if t == "response_item" and isinstance(p, dict) and p.get("type") == "reasoning":
        p["encrypted_content"] = "<stripped>"
    if t == "event_msg" and isinstance(p, dict) and p.get("type") == "item_completed":
        item = p.get("item", {})
        if item.get("type") == "Reasoning":
            item["raw_content"] = "<stripped>"
    print(json.dumps(d))
EOF
fi
n=0
for file in $(ls "$raw"/*.json 2>/dev/null | sort); do
  n=$((n + 1))
  event="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("hook_event_name","Unknown"))' "$file")"
  name="$(printf '%02d-%s' "$n" "$event")"
  for suffix in json env; do
    [ -f "${file%.json}.$suffix" ] && redact <"${file%.json}.$suffix" >"$out/hooks/$name.$suffix"
  done
done
mkdir -p "$out/notify"
m=0
for file in $(ls "$raw-notify"/*.json 2>/dev/null | sort); do
  m=$((m + 1))
  for suffix in json env; do
    [ -f "${file%.json}.$suffix" ] && redact <"${file%.json}.$suffix" >"$out/notify/$(printf '%02d' "$m").$suffix"
  done
done
{
  echo "Codex CLI $version, recorded $(date -u +%Y-%m-%d) with scripts/record-codex.sh${model:+ (model: $model)}."
  echo "rollout.jsonl: the session file codex exec wrote, instructions/reasoning stripped, paths redacted."
  echo "hooks/: what hooks.json delivered ($n payloads) under --dangerously-bypass-hook-trust; the .env files show the inherited variables."
  echo "notify/: the argument the notify program was given ($m calls), and its environment."
} >"$out/README.txt"
echo "wrote fixtures to $out; raw output kept in $work"
