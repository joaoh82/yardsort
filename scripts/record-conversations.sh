#!/usr/bin/env bash
# Record a short conversation with each agent whose transcript Yardsort reads, as fixtures for the
# conversation digest (`yardsort_core::activity::transcript`).
#
# Runs the *installed* agents non-interactively in a throwaway git repository, two turns each, the
# second a resume of the first:
#
# - Claude Code and Codex: the first turn writes a file and reads it back, so the transcript holds
#   tool calls and their output the digest must leave out; for Claude, a fork as well. Codex runs
#   under a throwaway `CODEX_HOME` with the real one's `auth.json` symlinked in, as
#   scripts/record-codex.sh does; Claude Code writes under your own config directory, as
#   scripts/record-claude-hooks.sh does.
# - Cursor and OpenCode: the file is there already and the first turn only reads it, so neither
#   is given leave to change anything. Cursor runs with `--trust` for the throwaway repository
#   and writes under your own `~/.cursor`, in TZ=UTC so the time it puts in each message is UTC.
#   OpenCode runs a free model (`OPENCODE_MODEL`, default opencode/glm-5.3-flash-free) under a
#   throwaway `XDG_DATA_HOME`, so its database holds this conversation alone; the fixture is that
#   database's two conversation tables, as SQL.
#
# Writes, redacted of this machine's paths and ids, and stripped of the model's instructions and
# of anything that describes your own setup:
#
#   crates/core/fixtures/claude-transcript/<version>/conversation.jsonl  (and fork.jsonl)
#   crates/core/fixtures/codex/<version>/conversation.jsonl
#   crates/core/fixtures/cursor/<version>/conversation.jsonl
#   crates/core/fixtures/opencode/<version>/conversation.sql
#
# Spends a few hundred tokens on each account (OpenCode's free model, none). Re-run it when a new
# version changes a shape; the readers' tests read these files.
#
# Usage: scripts/record-conversations.sh [claude|codex|cursor|opencode]...   (default: all four)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
which=("$@")
[ ${#which[@]} -eq 0 ] && which=(claude codex cursor opencode)
work="$(mktemp -d -t yardsort-conversation.XXXXXX)"
mkdir -p "$work/repo"
git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "fixture"

first="Create a file named notes.txt containing the line: the tests need TZ=UTC. Then read it back. Then reply with one short sentence saying what you did."
second="Thanks. In one sentence, what is left to do?"
forked="In one sentence, say which file you created."
# For the agents given no leave to write: the file is there, and the first turn reads it.
note="the tests need TZ=UTC"
read_first="Read notes.txt, then reply with one short sentence saying what it says."

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

record_cursor() {
  local version out repo chat transcript
  version="$(cursor-agent --version)"
  out="$root/crates/core/fixtures/cursor/$version"
  repo="$work/repo-cursor"
  mkdir -p "$repo"
  git -C "$repo" init -q
  echo "$note" >"$repo/notes.txt"
  echo "recording Cursor $version into $out"
  chat="$(cd "$repo" && TZ=UTC cursor-agent -p --trust --output-format json "$read_first" \
    </dev/null 2>>"$work/cursor.err" | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["session_id"])')"
  (cd "$repo" && TZ=UTC cursor-agent -p --trust --resume "$chat" --output-format json "$second" \
    </dev/null) >>"$work/cursor.out" 2>>"$work/cursor.err" ||
    echo "cursor-agent exited $? (see $work/cursor.err)"
  transcript="$(find "$HOME/.cursor/projects" -path "*/agent-transcripts/$chat/$chat.jsonl" | head -1)"
  mkdir -p "$out"
  redact "$transcript" "$out/conversation.jsonl" "$chat" 33333333-3333-4333-8333-333333333333
  {
    echo "conversation.jsonl: Cursor agent $version, recorded $(date -u +%Y-%m-%d) with scripts/record-conversations.sh."
    echo "cursor-agent -p reads notes.txt, then --resume adds a second turn to the same chat, which is 3333…;"
    echo "TZ=UTC, so each message's <timestamp> is in UTC. Paths and ids are redacted. Cursor writes no tool"
    echo "output to this file."
  } >"$out/README-conversation.txt"
}

record_opencode() {
  local version out repo data db session model
  version="$(opencode --version | awk '{print $NF}' | sed 's/^v//')"
  out="$root/crates/core/fixtures/opencode/$version"
  repo="$work/repo-opencode"
  data="$work/opencode-data"
  model="${OPENCODE_MODEL:-opencode/glm-5.3-flash-free}"
  mkdir -p "$repo" "$data"
  git -C "$repo" init -q
  echo "$note" >"$repo/notes.txt"
  echo "recording OpenCode $version into $out ($model)"
  (cd "$repo" && XDG_DATA_HOME="$data" opencode run --standalone -m "$model" "$read_first" \
    </dev/null) >>"$work/opencode.out" 2>>"$work/opencode.err" ||
    echo "opencode exited $? (see $work/opencode.err)"
  db="$data/opencode/opencode.db"
  session="$(sqlite3 "$db" "SELECT id FROM session_v2 LIMIT 1")"
  (cd "$repo" && XDG_DATA_HOME="$data" opencode run --standalone -m "$model" -s "$session" \
    "$second" </dev/null) >>"$work/opencode.out" 2>>"$work/opencode.err" ||
    echo "opencode exited $? (see $work/opencode.err)"
  mkdir -p "$out"
  # The two tables a conversation is in, and nothing else of the database: its accounts,
  # credentials and projects are OpenCode's, not the conversation's.
  {
    echo "-- OpenCode $version: the session_v2 and session_message tables of a throwaway database."
    sqlite3 "$db" ".schema session_v2" ".schema session_message"
    sqlite3 "$db" ".mode insert session_v2" "SELECT * FROM session_v2"
    sqlite3 "$db" ".mode insert session_message" "SELECT * FROM session_message ORDER BY seq"
  } >"$work/opencode.sql"
  redact "$work/opencode.sql" "$out/conversation.sql" "$session" ses_fixture0000000000000001
  {
    echo "conversation.sql: OpenCode $version, recorded $(date -u +%Y-%m-%d) with scripts/record-conversations.sh"
    echo "($model). opencode run reads notes.txt, then -s adds a second turn to the same session,"
    echo "ses_fixture0000000000000001. The session_v2 and session_message tables only; paths and ids redacted."
    echo "opencode run keeps the message it was given in quotes, which the fixture keeps."
  } >"$out/README-conversation.txt"
}

for harness in "${which[@]}"; do
  "record_$harness"
done
echo "done; raw output in $work"
