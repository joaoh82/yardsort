#!/bin/sh
# The app under test, on this run's profile, with whatever it opens in a browser fetched instead.
#
#   sh .yardsort/tests/support/app.sh seed | open | again | close | copies | drives-runs | stop-everything
set -u
profile="$YS_RUN_DIR/profile"

environment() {
  mkdir -p "$YS_RUN_DIR/bin"
  cat > "$YS_RUN_DIR/bin/xdg-open" <<'BROWSER'
#!/bin/sh
exec curl -fsSL --max-time 30 -o /dev/null "$1"
BROWSER
  chmod +x "$YS_RUN_DIR/bin/xdg-open"
  export PATH="$YS_RUN_DIR/bin:$PATH"
  export YARDSORT_DATA_DIR="$profile"
  export YARDSORT_WORKTREE_ROOT="$YS_RUN_DIR/worktrees"
  export YARDSORT_CREDENTIALS_DIR="$YS_RUN_DIR/credentials"
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
