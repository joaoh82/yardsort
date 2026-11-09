#!/usr/bin/env bash
# Record what an agent's adapter channel delivers when the agent opens a pull request, as
# fixtures for the rule that keeps its link (docs/design/38-pull-requests-elsewhere.md, slice 3).
# Claude Code has its own script, scripts/record-claude-pr-hooks.sh.
#
# Runs the agent's own recording script (scripts/record-<agent>.sh) with a different prompt —
# open a pull request with `gh pr create`, and, where the agent takes an MCP server for one
# launch, with a GitHub server's `create_pull_request` tool — and a different fixtures directory,
# `crates/core/fixtures/<agent>-pull-requests/<version>/`. Both are stand-ins: a `gh` on PATH that
# prints a link, and an MCP server named `github` that answers as GitHub's does, so nothing is
# created anywhere. It spends a few hundred tokens on the account the agent is logged in to.
#
# Usage: scripts/record-pr-creation.sh codex|cursor|grok|opencode|pi|omp [model]
set -euo pipefail

agent="${1:?codex, cursor, grok, opencode, pi or omp}"
model="${2:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
stand_in="$(mktemp -d -t yardsort-pr-creation.XXXXXX)"
mkdir -p "$stand_in/bin"

# What `gh pr create` prints: the progress line on stderr, the link alone on stdout.
cat >"$stand_in/bin/gh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "pr" ] && [ "${2:-}" = "create" ]; then
  printf '\nCreating pull request for bump into main in example/lib\n\n' >&2
  echo "https://github.com/example/lib/pull/41"
  exit 0
fi
echo "stand-in gh: only pr create" >&2
exit 1
EOF
chmod +x "$stand_in/bin/gh"

# The stand-in GitHub MCP server: stdio JSON-RPC with one tool, answering as
# github/github-mcp-server's create_pull_request does — the new pull request's id and its link.
cat >"$stand_in/github-mcp.py" <<'EOF'
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

shell_step="Run this exact shell command: gh pr create --title Bump --body Bump"
mcp_step="Then call the create_pull_request tool of the github MCP server with owner example, repo lib, title Bump, head bump and base main."
case "$agent" in
  # These take an MCP server for one launch through the recording's own channel.
  codex | opencode)
    export YS_RECORD_MCP="$stand_in/github-mcp.py"
    prompt="$shell_step. $mcp_step Reply with the two links you got, nothing else."
    ;;
  # Cursor's shell takes PATH from the login shell, past the one given here: name the stand-in.
  cursor)
    export YS_RECORD_MCP="$stand_in/github-mcp.py"
    prompt="Run this exact shell command: $stand_in/bin/gh pr create --title Bump --body Bump. $mcp_step Reply with the two links you got, nothing else."
    ;;
  grok | pi | omp)
    prompt="$shell_step. Reply with the link it printed, nothing else."
    ;;
  *) echo "codex, cursor, grok, opencode, pi or omp" >&2; exit 2 ;;
esac

export PATH="$stand_in/bin:$PATH"
export YS_RECORD_PROMPT="$prompt"
export YS_RECORD_OUT="$root/crates/core/fixtures/$agent-pull-requests"
case "$agent" in
  pi | omp) "$root/scripts/record-pi.sh" "$agent" $model ;;
  *) "$root/scripts/record-$agent.sh" $model ;;
esac
for file in "$YS_RECORD_OUT"/*/README.txt; do
  grep -q "record-pr-creation" "$file" ||
    printf 'Recorded through scripts/record-pr-creation.sh %s: %s\n' "$agent" "$prompt" >>"$file"
done
# The stand-ins' directory is this machine's, and random.
grep -rl "$stand_in" "$YS_RECORD_OUT" | xargs -r sed -i "s#$stand_in#/tmp/yardsort-stand-in#g"
# The fixtures are checked by the repository's formatter like any other JSON.
(cd "$root" && npx prettier --log-level warn --write "$YS_RECORD_OUT" >/dev/null) || true
echo "stand-ins kept in $stand_in"
