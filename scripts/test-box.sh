#!/usr/bin/env bash
# The owner's test box for Remote: a Linux server with no screen, reached over SSH, where
# `yardsort-host` runs against staging and a phone pairs with it. See
# docs/design/35-remote-r4-r5.md §6.
#
#   scripts/test-box.sh up       # create it, set it up, copy this checkout over and build it
#   scripts/test-box.sh sync     # copy this checkout over again and rebuild
#   scripts/test-box.sh status   # whether it exists, and what yardsort-host says
#   scripts/test-box.sh ssh [command…]
#   scripts/test-box.sh down     # sign it out everywhere, then delete it
#
# It is a Hetzner Cloud server, billed for as long as it exists, powered off or not: only `down`
# stops the bill. Everything goes through the hcloud context named by TEST_BOX_CONTEXT
# (`yardsort`), never whichever one is active, and `gh` as you are signed in here.
#
# What `up` cannot do, because each is a sign-in of yours, it prints at the end: Claude Code,
# `gh` on the box, `yardsort-host sign-in`, and pairing the phone again.
#
# What the box holds is worth protecting: those sign-ins, and an SSH key on your GitHub account
# that can push to everything you can. `down` signs the host out at the server, so the phone
# does not keep a computer that never comes back, signs `gh` out, removes that key from your
# account, and only then deletes the server. Nothing is kept: no snapshot, no backup, since
# either would hold the sign-ins.
#
# Settings, all optional:
#   TEST_BOX_CONTEXT   hcloud context (yardsort)       TEST_BOX_NAME      server name (yardsort-box)
#   TEST_BOX_TYPE      server type (cx33)              TEST_BOX_LOCATION  fsn1
#   TEST_BOX_KEY       public key to log in with (~/.ssh/id_ed25519.pub)
#   TEST_BOX_REPO      repository the agents work in (<you>/yardsort-box-demo, made private if new)
#   TEST_BOX_GIT_NAME, TEST_BOX_GIT_EMAIL   git identity on the box (your GitHub profile's)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
context="${TEST_BOX_CONTEXT:-yardsort}"
name="${TEST_BOX_NAME:-yardsort-box}"
type="${TEST_BOX_TYPE:-cx33}"
location="${TEST_BOX_LOCATION:-fsn1}"
key="${TEST_BOX_KEY:-$HOME/.ssh/id_ed25519.pub}"
firewall="ssh-only"
# GitHub's published ed25519 host key fingerprint
# (docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints).
github_fingerprint="SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"

hc() { hcloud --context "$context" "$@"; }
say() { printf '%s\n' "$*" >&2; }
die() {
  say "$*"
  exit 1
}

need() {
  for tool in "$@"; do
    command -v "$tool" >/dev/null || die "Needs $tool."
  done
}

exists() { hc server describe "$name" >/dev/null 2>&1; }
address() { hc server ip "$name"; }
on_box() {
  local user="$1"
  shift
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$user@$(address)" "$@"
}

# The key's id in the project, adding it the first time. Hetzner refuses the same key twice
# under two names, so it is found by fingerprint.
login_key() {
  [[ -f "$key" ]] || die "No public key at $key; set TEST_BOX_KEY."
  local fingerprint id
  fingerprint="$(ssh-keygen -E md5 -lf "$key" | awk '{print $2}' | sed 's/^MD5://')"
  id="$(hc ssh-key list -o noheader -o columns=id,fingerprint | awk -v f="$fingerprint" '$2 == f {print $1}')"
  if [[ -z "$id" ]]; then
    id="$(hc ssh-key create --name "$(whoami)@$(hostname -s)" --public-key-from-file "$key" -o json | jq -r '.ssh_key.id // .id')"
  fi
  printf '%s\n' "$id"
}

ensure_firewall() {
  hc firewall describe "$firewall" >/dev/null 2>&1 && return
  hc firewall create --name "$firewall" >/dev/null
  hc firewall add-rule "$firewall" --direction in --protocol tcp --port 22 \
    --source-ips 0.0.0.0/0 --source-ips ::/0 --description ssh >/dev/null
}

wait_for_setup() {
  say "Waiting for the first boot's setup…"
  ssh-keygen -R "$(address)" >/dev/null 2>&1 || true
  for _ in $(seq 1 60); do
    if ssh -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=accept-new \
      "root@$(address)" 'cloud-init status --wait >/dev/null 2>&1; cloud-init status' 2>/dev/null |
      grep -q 'status: done'; then
      return
    fi
    sleep 10
  done
  die "The setup did not finish. Look with: $0 ssh sudo cloud-init status --long"
}

# Copies this checkout to ~/src/yardsort, as a repository of its own: a worktree's `.git` is a
# file pointing at a folder that is not there. Not under ~/yardsort, the worktree root, where
# Yardsort would take it for a workspace.
sync() {
  say "Copying $root to the box…"
  rsync -az --delete --exclude target --exclude node_modules --exclude .git \
    --exclude .yardsort/runs "$root/" "yardsort@$(address):src/yardsort/"
  on_box yardsort 'set -e
    cd ~/src/yardsort
    [ -d .git ] || git init -q
    git add -A
    git diff --cached --quiet || git commit -qm "Snapshot from the owner'"'"'s checkout"
    . ~/.cargo/env
    cargo build --release -p yardsort-host -p yardsort-cli 2>&1 | tail -1
    install -D target/release/yardsort-host target/release/ys -t ~/.local/bin/'
  say "Built and installed. A running serve keeps the old build until it is restarted."
}

github() {
  local login git_name git_email repo pub
  login="$(gh api user -q .login)"
  git_name="${TEST_BOX_GIT_NAME:-$(gh api user -q '.name // .login')}"
  git_email="${TEST_BOX_GIT_EMAIL:-$(gh api user -q '.email // empty')}"
  [[ -n "$git_email" ]] || git_email="$(gh api user -q .id)+$login@users.noreply.github.com"
  repo="${TEST_BOX_REPO:-$login/yardsort-box-demo}"

  on_box yardsort "git config --global user.name '$git_name' &&
    git config --global user.email '$git_email' && git config --global init.defaultBranch main"

  pub="$(on_box yardsort 'test -f ~/.ssh/id_ed25519 ||
      ssh-keygen -q -t ed25519 -N "" -C "'"$name"' (test box)" -f ~/.ssh/id_ed25519
    cat ~/.ssh/id_ed25519.pub')"
  gh ssh-key add <(printf '%s\n' "$pub") --title "$name (test box)" >/dev/null
  on_box yardsort "ssh-keyscan -t ed25519 github.com 2>/dev/null > /tmp/github.key &&
    ssh-keygen -lf /tmp/github.key | grep -q '$github_fingerprint' &&
    cat /tmp/github.key >> ~/.ssh/known_hosts; rm -f /tmp/github.key" ||
    die "github.com did not offer the host key GitHub publishes."

  gh repo view "$repo" >/dev/null 2>&1 ||
    gh repo create "$repo" --private --add-readme \
      --description "Throwaway repository for trying Yardsort on a headless box" >/dev/null
  on_box yardsort "test -d ~/src/demo || git clone -q git@github.com:$repo.git ~/src/demo"
  say "GitHub: a key on your account for the box, and $repo at ~/src/demo."
}

up() {
  need hcloud gh jq rsync ssh ssh-keygen
  exists && die "$name exists already, at $(address). Use sync, or down first."
  local price
  price="$(hc server-type describe "$type" -o json |
    jq -r --arg l "$location" '.prices[] | select(.location == $l) |
      "\(.price_hourly.gross | tonumber * 10000 | round / 10000) an hour, at most \(.price_monthly.gross | tonumber * 100 | round / 100) a month"')"
  say "Creating $name: $type in $location, $price (EUR, before the IPv4 address)."
  ensure_firewall
  hc server create --name "$name" --type "$type" --image ubuntu-24.04 --location "$location" \
    --ssh-key "$(login_key)" --firewall "$firewall" \
    --user-data-from-file "$here/test-box/cloud-init.yaml" >/dev/null
  wait_for_setup
  curl_claude='curl -fsSL https://claude.ai/install.sh | bash >/dev/null 2>&1'
  on_box root 'set -e
    install -d -m 755 /etc/apt/keyrings
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /etc/apt/keyrings/githubcli-archive-keyring.gpg
    chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq gh >/dev/null 2>&1'
  on_box yardsort "$curl_claude"
  github
  sync
  cat >&2 <<EOF

$name is up: ssh yardsort@$(address)
Left to you, there:
  claude                                    # sign in
  gh auth login --hostname github.com --git-protocol ssh --skip-ssh-key --web
  yardsort-host sign-in https://remote-staging.yardsort.sh --name "Build box"
  tmux new -s yardsort, then: yardsort-host serve
  ys project add ~/src/demo
  yardsort-host pair                        # from your own SSH login, then the phone
EOF
}

down() {
  need hcloud gh jq ssh
  exists || die "There is no $name in the $context project."
  local ip pub id
  ip="$(address)"
  say "Work on the box that is not pushed:"
  on_box yardsort 'for repo in ~/src/demo ~/yardsort/*/*/; do
      [ -e "$repo/.git" ] || continue
      changed="$(git -C "$repo" status --porcelain)"
      unpushed="$(git -C "$repo" log --branches --not --remotes --oneline)"
      [ -n "$changed$unpushed" ] && printf "  %s\n%s\n%s\n" "$repo" "$changed" "$unpushed"
    done; true' >&2 || say "  (could not reach the box to look)"
  read -r -p "Type $name to sign it out and delete it: " answer
  [[ "$answer" == "$name" ]] || die "Kept."

  pub="$(on_box yardsort 'cat ~/.ssh/id_ed25519.pub 2>/dev/null' || true)"
  on_box yardsort 'bash -lc "yardsort-host sign-out"' >&2 ||
    say "yardsort-host did not sign out; remove the computer from the phone by hand."
  on_box yardsort 'gh auth logout --hostname github.com </dev/null' >/dev/null 2>&1 || true
  if [[ -n "$pub" ]]; then
    id="$(gh api user/keys | jq -r --arg k "$(awk '{print $1" "$2}' <<<"$pub")" '.[] | select(.key == $k) | .id')"
    [[ -n "$id" ]] && gh ssh-key delete "$id" --yes >/dev/null && say "Removed its key from GitHub."
  fi
  hc server delete "$name" >/dev/null
  ssh-keygen -R "$ip" >/dev/null 2>&1 || true
  say "$name is deleted, and its bill stops. The demo repository is kept: gh repo delete removes it."
}

status() {
  need hcloud
  if ! exists; then
    say "No $name in the $context project: nothing is billed for it."
    return
  fi
  hc server list -o columns=name,status,type,ipv4,location,age
  on_box yardsort 'bash -lc "yardsort-host status"' || true
}

case "${1:-}" in
  up) up ;;
  sync) sync ;;
  status) status ;;
  down) down ;;
  ssh)
    shift
    exec ssh "yardsort@$(address)" "$@"
    ;;
  *) die "Usage: $0 up | sync | status | ssh [command…] | down" ;;
esac
