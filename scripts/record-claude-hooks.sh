#!/usr/bin/env bash
# Record what Claude Code's hooks deliver, as fixtures for the activity adapter.
#
# Runs the *installed* `claude` non-interactively in a throwaway git repository with every hook
# event Yardsort listens to pointed at a capture script, then a resume and a fork of that
# session, and writes each payload — redacted of this machine's paths — under
# `crates/core/fixtures/claude-hooks/<version>/`. It spends a few hundred tokens on the account
# `claude` is logged in to. Re-run it when a new Claude Code version changes what a hook sends;
# the adapter's tests read these files.
#
# Usage: scripts/record-claude-hooks.sh [model]        (default: haiku, the cheapest)
set -euo pipefail

model="${1:-haiku}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(claude --version | awk '{print $1}')"
out="$root/crates/core/fixtures/claude-hooks/$version"
work="$(mktemp -d -t yardsort-hooks.XXXXXX)"
capture="$work/capture.sh"
raw="$work/raw"
mkdir -p "$raw" "$work/repo"

# The hook: stdin to a file, plus the variables that prove the environment is inherited. Exit 0
# and print nothing, exactly as Yardsort's own hook will.
cat >"$capture" <<'EOF'
#!/usr/bin/env bash
dir="$1"
stamp="$(date +%s%N)"
cat >"$dir/$stamp.json"
env | grep -E '^(YARDSORT_|CLAUDE_PROJECT_DIR=|CLAUDE_SESSION_ID=|CLAUDECODE=)' | sort >"$dir/$stamp.env" || true
exit 0
EOF
chmod +x "$capture"

events=(SessionStart SessionEnd UserPromptSubmit PreToolUse PostToolUse PostToolUseFailure
  PermissionRequest PermissionDenied Notification Stop StopFailure SubagentStart SubagentStop
  PreCompact PostCompact PostModelSwitch)
hooks=""
for event in "${events[@]}"; do
  hooks+="\"$event\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"$capture\", \"args\": [\"$raw\"], \"timeout\": 10}]}],"
done
settings="$work/settings.json"
printf '{"hooks": {%s}}\n' "${hooks%,}" >"$settings"

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

# The markers this enclosing session would otherwise pass down (see crates/core/src/env.rs).
clean=(env -u CLAUDECODE -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_EXECPATH
  -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN -u CLAUDE_CODE_SESSION_ATTENDED
  -u CLAUDE_CODE_SESSION_ID -u CLAUDE_PID
  YARDSORT_RUN_ID=run-fixture YARDSORT_WORKSPACE_ID=workspace-fixture YARDSORT_SESSION_RECORD_ID=record-fixture)

session="$(uuidgen | tr 'A-Z' 'a-z')"
fork="$(uuidgen | tr 'A-Z' 'a-z')"
run_claude() {
  (cd "$work/repo" && "${clean[@]}" claude --print --model "$model" --settings "$settings" --max-turns 6 "$@") \
    >>"$work/claude.out" 2>>"$work/claude.err" || echo "claude exited $? (see $work/claude.err)"
}

echo "recording Claude Code $version into $out (session $session)"
# Fresh: a write, a read, a read of a file that does not exist (the failure path), a command.
run_claude --session-id "$session" --allowedTools "Write,Read,Bash(ls:*)" \
  -- "Create a file named hello.txt containing the single word hello. Then read it back. Then read a file named missing.txt, which does not exist. Then run \`ls\` with the Bash tool. Reply with one word when done."
echo "  fresh: $(ls "$raw"/*.json 2>/dev/null | wc -l) payloads"
# Resumed: the same conversation, one more turn.
run_claude --resume "$session" -- "Reply with the single word ok."
echo "  resumed: $(ls "$raw"/*.json 2>/dev/null | wc -l) payloads"
# Forked: a copy, with an id we chose, as Yardsort's Fork does.
run_claude --resume "$session" --fork-session --session-id "$fork" -- "Reply with the single word ok."
echo "  forked: $(ls "$raw"/*.json 2>/dev/null | wc -l) payloads"

# Redact: this machine's paths and the ids, then name each file by order and event.
rm -rf "$out"
mkdir -p "$out"
n=0
for file in $(ls "$raw"/*.json | sort); do
  n=$((n + 1))
  event="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("hook_event_name","Unknown"))' "$file")"
  name="$(printf '%02d-%s' "$n" "$event")"
  encoded_repo="$(echo "$work/repo" | tr '/' '-')"
  for suffix in json env; do
    [ -f "${file%.json}.$suffix" ] || continue
    sed -e "s#$work/repo#/tmp/yardsort-fixture/repo#g" \
      -e "s#$encoded_repo#-tmp-yardsort-fixture-repo#g" \
      -e "s#$work#/tmp/yardsort-fixture#g" \
      -e "s#$HOME#/home/user#g" \
      -e "s#$session#11111111-1111-4111-8111-111111111111#g" \
      -e "s#$fork#22222222-2222-4222-8222-222222222222#g" \
      "${file%.json}.$suffix" >"$out/$name.$suffix"
  done
done
{
  echo "Claude Code $version, recorded $(date -u +%Y-%m-%d) with scripts/record-claude-hooks.sh (model: $model)."
  echo "Session 1111…: fresh (--session-id), then --resume, then --resume --fork-session --session-id 2222…."
  echo "Paths and ids are redacted; the .env files show which variables the hook process inherited."
} >"$out/README.txt"
echo "wrote $n payloads to $out"
echo "raw output kept in $work"
