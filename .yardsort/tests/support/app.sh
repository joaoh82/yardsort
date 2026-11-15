#!/bin/sh
# The app under test, on this run's profile, with whatever it opens in a browser fetched instead.
#
#   sh .yardsort/tests/support/app.sh seed | open | again | close | copies | drives-runs | stop-everything
#   sh .yardsort/tests/support/app.sh seed-a-test | seed-a-pull-request | ys [args…]
#
# Its profile, worktrees, sign-ins and stand-in browser are in the run's folder, or in
# $YS_APP_DIR when a test runs another program there on a profile of its own.
set -u
app="${YS_APP_DIR:-$YS_RUN_DIR}"
profile="$app/profile"

environment() {
  mkdir -p "$app/bin"
  cat > "$app/bin/xdg-open" <<'BROWSER'
#!/bin/sh
exec curl -fsSL --max-time 30 -o /dev/null "$1"
BROWSER
  chmod +x "$app/bin/xdg-open"
  export PATH="$app/bin:$PATH"
  export YARDSORT_DATA_DIR="$profile"
  export YARDSORT_WORKTREE_ROOT="$app/worktrees"
  export YARDSORT_CREDENTIALS_DIR="$app/credentials"
  # A test that stands in for `gh` has the app find programs through a login shell of its own,
  # so the stand-in is found and the real one, with a real person's sign-in, is not.
  if [ -x "$app/loginshell" ]; then export SHELL="$app/loginshell"; fi
}

# On this profile: the daemon too, which is the same program.
everything() {
  for pid in $(pgrep -x yardsort); do
    if grep -qz "^YARDSORT_DATA_DIR=$profile\$" "/proc/$pid/environ" 2>/dev/null; then echo "$pid"; fi
  done
}

copies() {
  for pid in $(everything); do
    if ! grep -qz -e --yardsort-daemon "/proc/$pid/cmdline" 2>/dev/null; then echo "$pid"; fi
  done
}

case "$1" in
  seed)
    # A repository to start workspaces in, and an agent that is really the shell, whose first
    # message is what it runs: nothing is asked of a real agent.
    repo="$YS_RUN_DIR/repo"
    mkdir -p "$repo" "$profile"
    git -C "$repo" init -q -b trunk
    git -C "$repo" -c user.name=Smoke -c user.email=smoke@example.invalid -c commit.gpgsign=false \
      commit -q --allow-empty -m Start
    cat > "$profile/settings.toml" <<'SETTINGS'
[[harness]]
id = "sh-agent"
builtin = false
label = "Smoke agent"
command = "sh"
prompt_args = ["-c", "{prompt}"]
session_id_mode = "assigned"
SETTINGS
    echo "seeded $repo"
    ;;
  seed-a-test)
    # A smoke test of the repository's own that nobody has approved: it prints one line.
    mkdir -p "$YS_RUN_DIR/repo/.yardsort/tests"
    cat > "$YS_RUN_DIR/repo/.yardsort/tests/hello.yaml" <<'TEST'
id: hello
name: Hello from a test
version: 1
steps:
  - id: says_so
    action: run
    program: echo
    args: [let-go-from-the-phone]
    expect:
      stdout_contains: let-go-from-the-phone
TEST
    echo "seeded hello.yaml"
    ;;
  seed-a-pull-request)
    # The repository as one on a forge, on a branch with a pull request, and a `gh` that answers
    # for it from a file: no forge is asked, and nobody's sign-in is used.
    # The second pull request is someone else's: its head is a commit this checkout never had,
    # since one whose head was made here is this checkout's own, whatever its branch is called.
    repo="$YS_RUN_DIR/repo"
    git -C "$repo" remote add origin https://github.com/orchard/demo.git
    git -C "$repo" checkout -q -b ys/add-option
    git -C "$repo" -c user.name=Smoke -c user.email=smoke@example.invalid -c commit.gpgsign=false \
      commit -q --allow-empty -m "Add an option"
    head=$(git -C "$repo" rev-parse HEAD)
    base=$(git -C "$repo" rev-parse trunk)
    mkdir -p "$app/bin" "$app/gh"
    cat > "$app/gh/list.json" <<LIST
[
  {
    "additions": 184, "deletions": 22,
    "author": { "login": "juno-p" },
    "baseRefName": "trunk", "baseRefOid": "$base",
    "headRefName": "ys/add-option", "headRefOid": "$head",
    "createdAt": "2026-11-14T09:00:00Z", "updatedAt": "2026-11-15T09:00:00Z",
    "isCrossRepository": false, "isDraft": false,
    "latestReviews": [{ "author": { "login": "mira-k" }, "state": "CHANGES_REQUESTED" }],
    "mergeable": "CONFLICTING",
    "number": 12,
    "reviewDecision": "CHANGES_REQUESTED",
    "reviewRequests": [],
    "state": "OPEN",
    "statusCheckRollup": [
      { "__typename": "CheckRun", "name": "build", "workflowName": "CI", "status": "COMPLETED",
        "conclusion": "SUCCESS", "detailsUrl": "https://github.com/orchard/demo/actions/runs/1" },
      { "__typename": "CheckRun", "name": "test", "workflowName": "CI", "status": "COMPLETED",
        "conclusion": "FAILURE", "detailsUrl": "https://github.com/orchard/demo/actions/runs/2" }
    ],
    "title": "Add an option",
    "url": "https://github.com/orchard/demo/pull/12"
  },
  {
    "additions": 3, "deletions": 1,
    "author": { "login": "tomas-r" },
    "baseRefName": "trunk", "baseRefOid": "$base",
    "headRefName": "someone-elses", "headRefOid": "1111111111111111111111111111111111111111",
    "createdAt": "2026-11-10T09:00:00Z", "updatedAt": "2026-11-12T09:00:00Z",
    "isCrossRepository": false, "isDraft": false,
    "latestReviews": [],
    "mergeable": "MERGEABLE",
    "number": 9,
    "reviewDecision": "",
    "reviewRequests": [],
    "state": "OPEN",
    "statusCheckRollup": [],
    "title": "Fix a typo",
    "url": "https://github.com/orchard/demo/pull/9"
  }
]
LIST
    cat > "$app/bin/gh" <<GH
#!/bin/sh
# Stands in for gh in a smoke test: the pull requests of the one repository, from a file.
echo "\$*" >> "$app/gh/asked"
case "\$1 \$2" in
  "pr list") cat "$app/gh/list.json" ;;
  *) echo "gh: this is a smoke test's stand-in; '\$1 \$2' is not answered" >&2; exit 1 ;;
esac
GH
    chmod +x "$app/bin/gh"
    cat > "$app/loginshell" <<SHELL
#!/bin/sh
# Stands in for the login shell the app asks where programs are.
PATH=$app/bin:/usr/local/bin:/usr/bin:/bin
export PATH
while [ \$# -gt 0 ]; do
  case \$1 in
    -c) shift; exec /bin/sh -c "\$1" ;;
    *) shift ;;
  esac
done
exec /bin/sh
SHELL
    chmod +x "$app/loginshell"
    echo "seeded pull requests 12 and 9, and a gh that answers for them"
    ;;
  ys)
    # `ys` in the repository, on this run's profile, with none of a launched session's markers:
    # whoever runs the outer test is not this profile's agent.
    shift
    environment
    ys="$PWD/target/debug/ys"
    cd "$YS_RUN_DIR/repo" || exit 1
    exec env -u YARDSORT_RUN_ID -u YARDSORT_WORKSPACE_ID -u YARDSORT_SESSION_RECORD_ID "$ys" "$@"
    ;;
  open)
    environment
    WEBKIT_INSPECTOR_HTTP_SERVER="127.0.0.1:$YS_RUN_PORT" \
      setsid target/debug/yardsort >> "$YS_RUN_DIR/app.log" 2>&1 < /dev/null &
    ;;
  again)
    environment
    timeout 20 target/debug/yardsort >> "$YS_RUN_DIR/app.log" 2>&1 < /dev/null
    echo "the second start exited with $?"
    echo "copies: $(copies | wc -l)"
    ;;
  close)
    for pid in $(copies); do kill "$pid"; done
    for _ in $(seq 100); do
      [ -z "$(copies)" ] && exit 0
      sleep 0.2
    done
    echo "The app did not close." >&2
    exit 1
    ;;
  stop-everything)
    for pid in $(everything); do kill "$pid"; done
    ;;
  drives-runs)
    for pid in $(copies); do
      if grep -qx workflows /proc/"$pid"/task/*/comm; then echo "drives workflow runs"; fi
    done
    ;;
  copies)
    echo "copies: $(copies | wc -l)"
    ;;
esac
