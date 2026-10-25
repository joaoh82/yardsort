#!/usr/bin/env bash
# Record a short conversation with Claude Code and one with Codex, as fixtures for the
# conversation digest (`yardsort_core::activity::transcript`).
#
# Runs the *installed* `claude` and `codex` non-interactively in a throwaway git repository: two
# turns each — the first writes a file and reads it back, so the transcript holds tool calls and
# their output the digest must leave out; the second is resumed — and, for Claude, a fork of the
# conversation. Codex runs under a throwaway `CODEX_HOME` with the real one's `auth.json` symlinked
# in, as scripts/record-codex.sh does; Claude Code writes under your own config directory, as
# scripts/record-claude-hooks.sh does. Writes, redacted of this machine's paths and ids, and
# stripped of the model's instructions and of anything that describes your own setup:
#
#   crates/core/fixtures/claude-transcript/<version>/conversation.jsonl  (and fork.jsonl)
#   crates/core/fixtures/codex/<version>/conversation.jsonl
#
# Spends a few hundred tokens on each account. Re-run it when a new version changes a shape; the
# readers' tests read these files.
#
# Usage: scripts/record-conversations.sh [claude|codex]...        (default: both)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
which=("$@")
[ ${#which[@]} -eq 0 ] && which=(claude codex)
work="$(mktemp -d -t yardsort-conversation.XXXXXX)"
mkdir -p "$work/repo"
git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

first="Create a file named notes.txt containing the line: the tests need TZ=UTC. Then read it back. Then reply with one short sentence saying what you did."
second="Thanks. In one sentence, what is left to do?"
forked="In one sentence, say which file you created."

redact() { # <in> <out> <id> <placeholder> [<id> <placeholder>]...
  local in="$1" out="$2"
  shift 2
  local args=(-e "s#$work/repo#/tmp/yardsort-fixture/repo#g"
    -e "s#$(echo "$work/repo" | tr '/.' '--')#-tmp-yardsort-fixture-repo#g"
    -e "s#$work#/tmp/yardsort-fixture#g" -e "s#$HOME#/home/user#g")
  while [ $# -ge 2 ]; do
    args+=(-e "s#$1#$2#g")
    shift 2
  done
  sed "${args[@]}" "$in" >"$out"
}

record_claude() {
  local version out session fork dir
  version="$(claude --version | awk '{print $1}')"
  out="$root/crates/core/fixtures/claude-transcript/$version"
  session="$(uuidgen | tr 'A-Z' 'a-z')"
  fork="$(uuidgen | tr 'A-Z' 'a-z')"
  # The markers this enclosing session would otherwise pass down (see crates/core/src/env.rs).
  local clean=(env -u CLAUDECODE -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_CODE_ENTRYPOINT
    -u CLAUDE_CODE_EXECPATH -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN
    -u CLAUDE_CODE_SESSION_ATTENDED -u CLAUDE_CODE_SESSION_ID -u CLAUDE_PID)
  run() {
    (cd "$work/repo" && "${clean[@]}" claude --print --model haiku --max-turns 6 \
      --allowedTools "Write,Read" "$@") >>"$work/claude.out" 2>>"$work/claude.err" ||
      echo "claude exited $? (see $work/claude.err)"
  }
  echo "recording Claude Code $version into $out (session $session)"
  run --session-id "$session" -- "$first"
  run --resume "$session" -- "$second"
  run --resume "$session" --fork-session --session-id "$fork" -- "$forked"
  dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/projects"
  mkdir -p "$out"
  local name id
  for name in conversation fork; do
    id="$session"
    [ "$name" = fork ] && id="$fork"
    # Only the conversation's own lines are kept whole. The rest — attachments, the requests
    # Claude Code sent — carry your system prompt, memory, skills and connectors; they keep their
    # type, so the reader is still shown skipping them, and lose everything else.
    python3 -I -c '
import json, sys
keep = {"user", "assistant", "queue-operation", "last-prompt", "mode"}
for line in open(sys.argv[1]):
    entry = json.loads(line)
    if entry.get("type") not in keep:
        entry = {key: entry[key] for key in ("type", "uuid", "timestamp", "sessionId") if key in entry}
        entry["stripped"] = True
    print(json.dumps(entry, ensure_ascii=False))
' "$(find "$dir" -name "$id.jsonl" | head -1)" >"$work/$name.jsonl"
    redact "$work/$name.jsonl" "$out/$name.jsonl" \
      "$session" 11111111-1111-4111-8111-111111111111 "$fork" 22222222-2222-4222-8222-222222222222
  done
  {
    echo "conversation.jsonl, fork.jsonl: Claude Code $version transcripts, recorded $(date -u +%Y-%m-%d)"
    echo "with scripts/record-conversations.sh (model: haiku). Session 1111…: a turn that writes and reads"
    echo "notes.txt, then --resume for a second turn; 2222… is --resume --fork-session of it, one more turn."
    echo "Paths and ids are redacted."
  } >"$out/README-conversation.txt"
}

record_codex() {
  local version out home thread rollout
  version="$(codex --version | awk '{print $2}')"
  out="$root/crates/core/fixtures/codex/$version"
  home="$work/codex-home"
  mkdir -p "$home"
  ln -s "${CODEX_HOME:-$HOME/.codex}/auth.json" "$home/auth.json"
  echo "recording Codex $version into $out"
  (cd "$work/repo" && CODEX_HOME="$home" codex exec --skip-git-repo-check -s workspace-write \
    "$first" </dev/null) >>"$work/codex.out" 2>>"$work/codex.err" ||
    echo "codex exited $? (see $work/codex.err)"
  rollout="$(find "$home/sessions" -name 'rollout-*.jsonl' | head -1)"
  thread="$(basename "$rollout" .jsonl | grep -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')"
  (cd "$work/repo" && CODEX_HOME="$home" codex exec --skip-git-repo-check -s workspace-write \
    resume "$thread" "$second" </dev/null) >>"$work/codex.out" 2>>"$work/codex.err" ||
    echo "codex exited $? (see $work/codex.err)"
  rm -f "$home/auth.json"
  mkdir -p "$out"
  # The instructions and the reasoning are the model's, not the conversation, and are large.
  python3 -I -c '
import json, re, sys
for line in open(sys.argv[1]):
    entry = json.loads(line)
    payload = entry.get("payload")
    if isinstance(payload, dict):
        if "base_instructions" in payload:
            payload["base_instructions"] = "<stripped>"
        for key in ("creator_user_id", "creator_account_id"):
            if key in payload:
                payload[key] = "<redacted>"
        if payload.get("type") == "reasoning":
            payload["encrypted_content"] = "<stripped>"
            payload["summary"] = []
        if entry.get("type") == "world_state":
            payload["state"] = "<stripped>"
        if payload.get("role") == "developer":
            payload["content"] = [{"type": "input_text", "text": "<stripped>"}]
    text = json.dumps(entry, ensure_ascii=False)
    print(re.sub(r"<timezone>[^<]*</timezone>", "<timezone>UTC</timezone>", text))
' "$rollout" >"$work/rollout.jsonl"
  redact "$work/rollout.jsonl" "$out/conversation.jsonl" \
    "$thread" 01a0d83f-0000-7000-8000-000000000001
  {
    echo "conversation.jsonl: Codex CLI $version, recorded $(date -u +%Y-%m-%d) with scripts/record-conversations.sh."
    echo "codex exec writes and reads notes.txt, then codex exec resume adds a second turn to the same thread,"
    echo "which is 01a0d83f-0000-7000-8000-000000000001. Instructions and reasoning stripped, paths redacted."
  } >"$out/README-conversation.txt"
}

for harness in "${which[@]}"; do
  "record_$harness"
done
echo "done; raw output in $work"
