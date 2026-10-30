#!/usr/bin/env bash
# Checklist § 26 (conversation context) on macOS, on a throwaway profile. Linux works too.
#
#   scripts/checklist-26.sh setup          # profile, demo repository, the dev `ys`; prints the steps
#   scripts/checklist-26.sh check [name]   # what can be read from outside the window, after the steps
#   scripts/checklist-26.sh clean          # remove everything setup made
#
# Windows has its own: scripts/checklist-26.ps1. The rows are in docs/design/08-manual-checklist.md;
# on macOS and Windows they are the first two and the one with the other five agents.
#
# The window is still yours to click: this prepares what the rows need and then checks what
# they leave behind — the conversations `ys workspace context` reads, whether Codex ran it before
# answering, and which running agents were started with the note. What only the window shows
# (the line under the tabs, the mark, Show) is yours to look at.
#
# Agents must find this build's `ys`, which has `workspace context`, before any installed one.
# They get PATH from the login-shell probe (crates/core/src/env.rs runs `$SHELL -i -l -c`), and
# your rc files would put an installed `ys` first, so `setup` writes a stand-in login shell that
# runs yours and puts this build in front. Needs git and python3.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(pwd)

# Resolved: on macOS /tmp is a link to /private/tmp, and a path that is spelt two ways is a
# finding of its own, not one to stumble on here.
ROOT=${YS_C26_ROOT:-$(cd "${TMPDIR:-/tmp}" && pwd -P)/ys-c26}
YS="$repo/target/debug/ys"

usage() {
  sed -n '4,6p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# `ys` against the throwaway profile, and never against an enclosing agent's workspace.
ys() {
  env -u YARDSORT_RUN_ID -u YARDSORT_WORKSPACE_ID -u YARDSORT_SESSION_RECORD_ID \
    "$YS" --data-dir "$ROOT/data" "$@"
}

steps() {
  cat <<EOF

Start the app on the throwaway profile, from this folder:

  SHELL=$ROOT/loginshell YARDSORT_DATA_DIR=$ROOT/data YARDSORT_WORKTREE_ROOT=$ROOT/wt \\
    YARDSORT_YS_DIR=$ROOT/ys-install just dev

Then, in the window:

  1. Add project → $ROOT/demo/notes-app. Memory… for it: "Share conversations between this
     project's agents" is ticked. Create a workspace in it (a worktree); call it tidy-calc.
  2. Start Claude from the tab bar and ask:
       In one sentence, what does calc.py do? Don't change anything.
     Start Codex from the tab bar: no question, no message, no turn. A line under the tabs says
     "Told about claude's conversation" for a minute; a ⇄ mark stays on the tab.
  3. Ask Codex:
       What was the other agent asked?
     It runs \`ys workspace context\` before answering, and answers from it. Show on the mark:
     the note, and the digest without Codex's own conversation.
  4. Start Cursor, OpenCode, Grok, OMP and pi in the workspace, and give each a message:
       In one sentence, what is in README.md? Don't change anything.
     Grok, OMP and pi start from the tab bar without the question, told as background context.
  5. Quit OMP and Resume it: its tab says "Told about …" again. Quit Grok and resume it: it does
     not. While both resumed tabs are open, run:

       scripts/checklist-26.sh check tidy-calc

  6. Show on a new agent's line, or the digest check prints: every conversation is there with
     its messages, and none of the tools they ran.

EOF
}

setup() {
  command -v git >/dev/null || { echo "needs git" >&2; exit 1; }
  command -v python3 >/dev/null || { echo "needs python3" >&2; exit 1; }
  rm -rf "$ROOT"
  mkdir -p "$ROOT/data" "$ROOT/wt" "$ROOT/bin" "$ROOT/demo/notes-app"

  echo "Building ys…"
  cargo build -q -p yardsort-cli
  ln -s "$YS" "$ROOT/bin/ys"

  # The demo repository the questions are about. Small enough that every answer is one sentence.
  (
    cd "$ROOT/demo/notes-app"
    git init -q -b main
    printf '# notes-app\n\nA tiny demo repository for trying Yardsort.\n' >README.md
    printf 'def add(a, b):\n    return a + b\n' >calc.py
    printf 'Remember to water the plants.\n' >notes.txt
    git add -A
    git -c user.name=Demo -c user.email=demo@example.invalid commit -q -m "First commit"
  )

  # Your own login shell, with this build of `ys` first. Only `-c` is honoured: that is all the
  # probe and the terminals' launches pass.
  local real=${SHELL:-/bin/zsh}
  cat >"$ROOT/loginshell" <<EOF
#!/bin/sh
# Stands in for the login shell Yardsort probes: $real, with this build's ys first.
cmd=""
while [ \$# -gt 0 ]; do
  case \$1 in
    -c) shift; cmd=\$1 ;;
  esac
  shift
done
if [ -z "\$cmd" ]; then
  exec "$real" -i -l
fi
exec "$real" -i -l -c "export PATH=$ROOT/bin:\\\$PATH; \$cmd"
EOF
  chmod +x "$ROOT/loginshell"
  # The digest only counts what was written after this, when it looks for Codex's run.
  touch "$ROOT/stamp"

  echo "Set up under $ROOT; this build's ys: $("$YS" --version)."
  steps
}

# The one worktree workspace in the profile, or the one named.
workspace() {
  ys workspace list --json | python3 -c '
import json, sys
wanted = sys.argv[1]
spaces = [w for w in json.load(sys.stdin) if w["kind"] == "worktree" and not w["archived"]]
if wanted:
    spaces = [w for w in spaces if w["name"] == wanted or w["id"].startswith(wanted)]
if len(spaces) != 1:
    names = ", ".join(w["name"] for w in spaces) or "none"
    sys.exit(f"Name the workspace: worktree workspaces here are {names}.")
print(spaces[0]["name"] + "\t" + spaces[0]["path"])
' "${1:-}"
}

check() {
  [ -f "$ROOT/data/yardsort.db" ] || {
    echo "No profile at $ROOT/data yet: run setup, then start the app as it says." >&2
    exit 1
  }
  local found name path
  found=$(workspace "${1:-}")
  name=${found%%$'\t'*}
  path=${found#*$'\t'}
  echo "Workspace $name, at $path"
  echo

  ys workspace context "$name" >"$ROOT/context.md" || {
    echo "FAIL  ys workspace context refused; is sharing on in the project's Memory…?"
    exit 1
  }
  ys workspace context "$name" --json >"$ROOT/context.json"
  echo "The digest is in $ROOT/context.md — read it for tool output, which must not be there."
  echo

  # Codex writes its session files under CODEX_HOME; the one that ran the command since setup.
  local codex_home=${CODEX_HOME:-$HOME/.codex} codex_ran=""
  if [ -d "$codex_home/sessions" ]; then
    codex_ran=$(find "$codex_home/sessions" -name '*.jsonl' -newer "$ROOT/stamp" \
      -exec grep -l "workspace context" {} + 2>/dev/null |
      xargs grep -l "$(basename "$path")" 2>/dev/null | head -1 || true)
  fi

  ps -axww -o pid=,args= >"$ROOT/processes.txt"

  python3 - "$ROOT/context.json" "$ROOT/processes.txt" "$codex_ran" <<'EOF'
import json, os, sys

digest = json.load(open(sys.argv[1]))["conversations"]
processes = open(sys.argv[2], errors="replace").read().splitlines()
codex_ran = sys.argv[3]

def line(verdict, text):
    print(f"{verdict:5} {text}")

said = {}
for c in digest:
    said.setdefault(c["harness"], []).append(c)

def spoke(harness):
    return any(c["turnCount"] > 0 for c in said.get(harness, []))

print("Row 1 · Claude, then Codex from the tab bar")
line("PASS" if spoke("claude") else "FAIL", "a Claude conversation with a turn is in the digest")

marker = "Other agents have worked in this workspace:"
harnesses = ["claude", "codex", "cursor-agent", "opencode", "grok", "omp", "pi"]
told = []
for p in processes:
    if marker not in p:
        continue
    pid, args = p.strip().split(None, 1)
    before = args.split(marker)[0].split()
    names = [os.path.basename(t) for t in before]
    program = next((n for n in names if n in harnesses), names[0] if names else "?")
    # The whole note, or the command line lost the end of it on the way.
    told.append((pid, program, before[-1] if before else "", "refers to their work." in args))
def told_as(harness):
    return [t for t in told if t[1] == harness]
codex_told = told_as("codex")
if codex_told:
    line("PASS", "a running Codex was started with the note")
else:
    line("LOOK", "no running Codex carries the note; check while its first tab is open")

print("\nRow 2 · Codex reads the conversation before answering")
asked = [t for c in said.get("codex", []) for t in c["turns"] if "other agent" in t["user"].lower()]
line("PASS" if asked else "FAIL", "Codex was asked what the other agent was asked")
line("PASS" if codex_ran else "FAIL",
     "Codex ran `ys workspace context` in this workspace" + (f" ({os.path.basename(codex_ran)})" if codex_ran else ""))
if asked and asked[0].get("reply"):
    print(f"      its answer: {' '.join(asked[0]['reply'].split())[:200]}")

print("\nRow 7 · the other five agents")
for harness in ["cursor", "opencode", "grok", "omp", "pi"]:
    if harness not in said:
        line("FAIL", f"no {harness} conversation in the digest")
    else:
        line("PASS" if spoke(harness) else "FAIL", f"{harness}'s messages are in the digest")
omp, grok = told_as("omp"), told_as("grok")
line("PASS" if omp else "LOOK", "a running OMP carries the note" + ("" if omp else "; check while the resumed tab is open"))
line("LOOK", f"running Grok processes with the note: {len(grok)} — after a resume there should be none")

unread = [c for c in digest if c["unreadable"]]
if unread:
    print("\nNot read — fine for one opened and closed without a message:")
    for c in unread:
        print(f"      {c['harness']} {c['id']}: {c['unreadable']}")

print("\nRunning agents started with the note:")
for pid, program, flag, whole in told:
    print(f"  {pid:>7}  {program} {flag}<the note>" + ("" if whole else "  ← cut short"))
if not told:
    print("  none")
if any(not whole for *_, whole in told):
    line("FAIL", "a note did not reach its agent whole")
EOF

  echo
  results
}

results() {
  local os chip
  if [ "$(uname)" = Darwin ]; then
    os="macOS $(sw_vers -productVersion)"
    chip=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m)
  else
    os="$(. /etc/os-release && echo "$PRETTY_NAME")"
    chip=$(uname -m)
  fi
  cat <<EOF
For 08 § Results, once every row passes in the window too:

### $os, $chip, $("$YS" --version | awk '{print $NF}') (dev) — $(date +%Y-%m-%d)
§ 26: the first two rows and the row with the other five agents pass.
Found: …
EOF
}

clean() {
  rm -rf "$ROOT"
  echo "Removed $ROOT. The agents' own conversations (~/.claude, ~/.codex, …) are kept."
}

case "${1:-}" in
  setup) setup ;;
  steps) steps ;;
  check) check "${2:-}" ;;
  clean) clean ;;
  -h | --help | help) usage ;;
  *) usage 1 ;;
esac
