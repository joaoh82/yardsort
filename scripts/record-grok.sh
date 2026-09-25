#!/usr/bin/env bash
# Record what Grok leaves in its session directory, and what its hooks deliver, as fixtures for
# the activity adapter.
#
# Runs the *installed* `grok` headlessly in a throwaway git repository under a throwaway
# `GROK_HOME` — the real one's `auth.json` symlinked in, nothing else — so the session it writes
# lands there and not in your history, and a hooks file there can point every hook event at a
# capture script without touching yours. Writes, redacted of this machine's paths and ids,
# under `crates/core/fixtures/grok/<version>/`: the session's `events.jsonl`, `usage.json` and
# `summary.json` (what the adapter reads), and each hook payload under `hooks/` (what it does
# not, kept as the record of the alternative). Spends a few hundred tokens on the account `grok`
# is logged in to. Re-run it when a new Grok version changes a shape; the adapter's tests read
# these files.
#
# Usage: scripts/record-grok.sh [model]
set -euo pipefail

model="${1:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(grok --version | awk '{print $2}')"
out="$root/crates/core/fixtures/grok/$version"
work="$(mktemp -d -t yardsort-grok.XXXXXX)"
home="$work/grok-home"
raw="$work/raw"
mkdir -p "$raw" "$work/repo" "$home/hooks"
for file in auth.json agent_id; do
  [ -e "${GROK_HOME:-$HOME/.grok}/$file" ] && ln -s "${GROK_HOME:-$HOME/.grok}/$file" "$home/$file"
done
trap 'rm -f "$home/auth.json" "$home/agent_id"' EXIT

cat >"$work/capture.sh" <<'EOF'
#!/usr/bin/env bash
dir="$1"
stamp="$(date +%s%N)"
cat >"$dir/$stamp.json"
env | grep -E '^(YARDSORT_|GROK_HOOK|GROK_SESSION|GROK_WORKSPACE|CLAUDE_PROJECT)' | sort >"$dir/$stamp.env" || true
exit 0
EOF
chmod +x "$work/capture.sh"
events=(SessionStart SessionEnd UserPromptSubmit PreToolUse PostToolUse PostToolUseFailure
  PermissionDenied Notification Stop StopFailure StopCancelled SubagentStart SubagentStop PreCompact PostCompact)
hooks=""
for event in "${events[@]}"; do
  hooks+="\"$event\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"$work/capture.sh $raw\", \"timeout\": 10}]}],"
done
printf '{"hooks": {%s}}\n' "${hooks%,}" >"$home/hooks/capture.json"

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

session="$(uuidgen | tr 'A-Z' 'a-z')"
model_args=()
[ -n "$model" ] && model_args=(-m "$model")
echo "recording Grok $version into $out (session $session)"
(cd "$work/repo" && env GROK_HOME="$home" \
  YARDSORT_RUN_ID=run-fixture YARDSORT_WORKSPACE_ID=workspace-fixture YARDSORT_SESSION_RECORD_ID=record-fixture \
  grok --session-id "$session" --always-approve --output-format json --max-turns 8 "${model_args[@]}" \
  -p "Create a file named hello.txt containing the single word hello. Then read it back. Then read a file named missing.txt, which does not exist. Then run \`ls\`. Reply with one word when done." </dev/null) \
  >"$work/grok.out" 2>"$work/grok.err" || echo "grok exited $? (see $work/grok.err)"
echo "  hooks delivered: $(ls "$raw"/*.json 2>/dev/null | wc -l)"
dir="$(find "$home/sessions" -type d -name "$session" | head -1 || true)"
echo "  session directory: ${dir:-none}"

rm -rf "$out"
mkdir -p "$out/hooks"
redact() {
  sed -e "s#$work/repo#/tmp/yardsort-fixture/repo#g" \
    -e "s#$home#/tmp/yardsort-fixture/grok-home#g" \
    -e "s#$work#/tmp/yardsort-fixture#g" \
    -e "s#$HOME#/home/user#g" \
    -e "s#$session#11111111-1111-4111-8111-111111111111#g"
}
if [ -n "$dir" ]; then
  for file in events.jsonl usage.json summary.json; do
    [ -f "$dir/$file" ] && redact <"$dir/$file" >"$out/$file"
  done
  # The event log names the machine's MCP servers; the adapter never reads those lines.
  [ -f "$out/events.jsonl" ] && grep -v '"type": *"mcp_' "$out/events.jsonl" >"$out/events.tmp" && mv "$out/events.tmp" "$out/events.jsonl"
  # The summary carries the machine's git remotes and the session's own summary text; neither
  # is read, and neither belongs in the tree.
  [ -f "$out/summary.json" ] && python3 - "$out/summary.json" <<'EOF'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
for key in ("git_remotes", "session_summary", "last_turn_summary", "info",
            "agent_id", "attempt_id", "request_id", "last_turn_summary_prompt_id"):
    if key in d:
        d[key] = "<stripped>"
json.dump(d, open(p, "w"), indent=1)
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
{
  echo "Grok $version, recorded $(date -u +%Y-%m-%d) with scripts/record-grok.sh${model:+ (model: $model)}."
  echo "One headless turn (grok -p) under a throwaway GROK_HOME, session id 1111… chosen up front as Yardsort does."
  echo "events.jsonl, usage.json, summary.json: the session directory, which the adapter reads (summary stripped of remotes and summaries)."
  echo "hooks/: what a hooks file delivered ($n payloads), kept as the record of the alternative; the .env files show the inherited variables."
} >"$out/README.txt"
echo "wrote fixtures to $out; raw output kept in $work"
