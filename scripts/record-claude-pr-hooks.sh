#!/usr/bin/env bash
# Record what Claude Code's hooks deliver when an agent opens a pull request, as fixtures for
# the rule that keeps its link (docs/design/38-pull-requests-elsewhere.md, slice 2).
#
# Runs the *installed* `claude` non-interactively in a throwaway git repository with a nested
# clone, `lib`, and has it open a pull request twice: with `gh pr create` in the clone, and with
# a GitHub MCP server's `create_pull_request` tool. Both are stand-ins — a `gh` on PATH that
# prints a link, and an MCP server named `github` that answers as GitHub's does — so nothing is
# created anywhere; what is recorded is what Claude Code hands its hooks around those calls.
# Every hook payload is written, redacted of this machine's paths, under
# `crates/core/fixtures/claude-hooks-pull-requests/<version>/`. It spends a few hundred tokens on
# the account `claude` is logged in to.
#
# Usage: scripts/record-claude-pr-hooks.sh [model]        (default: haiku, the cheapest)
set -euo pipefail

model="${1:-haiku}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(claude --version | awk '{print $1}')"
out="$root/crates/core/fixtures/claude-hooks-pull-requests/$version"
work="$(mktemp -d -t yardsort-pr-hooks.XXXXXX)"
capture="$work/capture.sh"
raw="$work/raw"
bin="$work/bin"
mkdir -p "$raw" "$work/repo" "$bin"

cat >"$capture" <<'EOF'
#!/usr/bin/env bash
dir="$1"
stamp="$(date +%s%N)"
cat >"$dir/$stamp.json"
exit 0
EOF
chmod +x "$capture"

# The stand-in `gh`: what `gh pr create` prints — the progress line on stderr, the link alone on
# stdout — and nothing else.
cat >"$bin/gh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "pr" ] && [ "${2:-}" = "create" ]; then
  printf '\nCreating pull request for bump into main in example/lib\n\n' >&2
  echo "https://github.com/example/lib/pull/41"
  exit 0
fi
echo "stand-in gh: only pr create" >&2
exit 1
EOF
chmod +x "$bin/gh"

# The stand-in GitHub MCP server: stdio JSON-RPC with one tool, answering as
# github/github-mcp-server's create_pull_request does — the new pull request's id and its link.
cat >"$work/github-mcp.py" <<'EOF'
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    method, ident = message.get("method"), message.get("id")
    if ident is None:
        continue
    if method == "initialize":
        result = {"protocolVersion": message["params"]["protocolVersion"],
                  "capabilities": {"tools": {}},
                  "serverInfo": {"name": "github-mcp-server", "version": "0.0.0-stand-in"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "create_pull_request",
                             "description": "Create a new pull request in a GitHub repository.",
                             "inputSchema": {"type": "object",
                                             "properties": {k: {"type": "string"} for k in
                                                            ["owner", "repo", "title", "head", "base", "body"]},
                                             "required": ["owner", "repo", "title", "head", "base"]}}]}
    elif method == "tools/call":
        answer = {"id": "2989102733", "url": "https://github.com/example/lib/pull/42"}
        result = {"content": [{"type": "text", "text": json.dumps(answer)}]}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": ident, "result": result}) + "\n")
    sys.stdout.flush()
EOF
cat >"$work/mcp.json" <<EOF
{"mcpServers": {"github": {"command": "python3", "args": ["$work/github-mcp.py"]}}}
EOF

hooks=""
for event in PreToolUse PostToolUse PostToolUseFailure; do
  hooks+="\"$event\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"$capture\", \"args\": [\"$raw\"], \"timeout\": 10}]}],"
done
settings="$work/settings.json"
printf '{"hooks": {%s}}\n' "${hooks%,}" >"$settings"

git -C "$work/repo" init -q
git -C "$work/repo" commit -q --allow-empty -m "parent"
git init -q "$work/repo/lib"
git -C "$work/repo/lib" commit -q --allow-empty -m "lib"

clean=(env -u CLAUDECODE -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_EXECPATH
  -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN -u CLAUDE_CODE_SESSION_ATTENDED
  -u CLAUDE_CODE_SESSION_ID -u CLAUDE_PID PATH="$bin:$PATH")

session="$(uuidgen | tr 'A-Z' 'a-z')"
echo "recording Claude Code $version into $out (session $session)"
(cd "$work/repo" && "${clean[@]}" claude --print --model "$model" --settings "$settings" \
  --mcp-config "$work/mcp.json" --strict-mcp-config --max-turns 6 --session-id "$session" \
  --allowedTools "Bash(cd:*),Bash(gh pr create:*),mcp__github__create_pull_request" \
  -- "Do exactly two things, in order. First, with the Bash tool, run exactly: cd lib && gh pr create --title \"Bump the client\" --body \"Moves to the new client.\" --base main --head bump
Second, call the create_pull_request tool of the github MCP server with owner example, repo lib, title \"Bump the client\", head bump, base main. Reply with one word when done.") \
  >"$work/claude.out" 2>"$work/claude.err" || echo "claude exited $? (see $work/claude.err)"

rm -rf "$out"
mkdir -p "$out"
n=0
encoded_repo="$(echo "$work/repo" | tr '/' '-')"
for file in $(ls "$raw"/*.json | sort); do
  n=$((n + 1))
  event="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("hook_event_name","Unknown"))' "$file")"
  sed -e "s#$work/repo#/tmp/yardsort-fixture/repo#g" \
    -e "s#$encoded_repo#-tmp-yardsort-fixture-repo#g" \
    -e "s#$work#/tmp/yardsort-fixture#g" \
    -e "s#$HOME#/home/user#g" \
    -e "s#$session#33333333-3333-4333-8333-333333333333#g" \
    "$file" >"$out/$(printf '%02d-%s' "$n" "$event").json"
done
{
  echo "Claude Code $version, recorded $(date -u +%Y-%m-%d) with scripts/record-claude-pr-hooks.sh (model: $model)."
  echo "One session: \`cd lib && gh pr create …\` against a stand-in gh, then a stand-in GitHub MCP"
  echo "server's create_pull_request. Nothing was created on GitHub. Paths and ids are redacted."
} >"$out/README.txt"
# The repository's formatter checks every JSON file; the content is unchanged by it.
(cd "$root" && npx prettier --write "$out"/*.json >/dev/null) || echo "format $out with prettier before committing"
echo "wrote $n payloads to $out"
echo "raw output kept in $work"
