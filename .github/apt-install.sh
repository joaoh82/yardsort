#!/usr/bin/env bash
# The same packages the Linux CI jobs have always installed.
#
# GitHub's Ubuntu runners fetch these from azure.archive.ubuntu.com. That name,
# and then archive.ubuntu.com, has failed DNS ("Temporary failure resolving")
# while other hosts still resolved, and apt-get update exits 0 after ignoring
# the missed indexes. This pins a host the system resolver missed, using
# DNS-over-HTTPS, then tries the configured mirror, the Ubuntu archive, and
# kernel.org. Each attempt is deadline-bounded so a stall cannot hold the step.
set -euo pipefail

packages=(
  libwebkit2gtk-4.1-dev
  build-essential
  curl
  wget
  file
  libxdo-dev
  libssl-dev
  libayatana-appindicator3-dev
  librsvg2-dev
)

sudo tee /etc/apt/apt.conf.d/99-yardsort-retries >/dev/null <<'EOF'
Acquire::Retries "2";
Acquire::http::Timeout "12";
Acquire::https::Timeout "12";
# A missed index is a failed update. The default ignores it and exits 0,
# and the install then cannot find these packages.
APT::Update::Error-Mode "any";
EOF

state=$(mktemp -d)
sudo cp -a /etc/apt/sources.list.d "$state/"
if [[ -f /etc/apt/sources.list ]]; then
  sudo cp -a /etc/apt/sources.list "$state/sources.list"
fi
if [[ -f /etc/apt/apt-mirrors.txt ]]; then
  sudo cp -a /etc/apt/apt-mirrors.txt "$state/apt-mirrors.txt"
fi

restore_sources() {
  sudo rm -rf /etc/apt/sources.list.d
  sudo cp -a "$state/sources.list.d" /etc/apt/
  if [[ -f $state/sources.list ]]; then
    sudo cp -a "$state/sources.list" /etc/apt/sources.list
  fi
  if [[ -f $state/apt-mirrors.txt ]]; then
    sudo cp -a "$state/apt-mirrors.txt" /etc/apt/apt-mirrors.txt
  fi
}

# An address from a resolver that is not this runner's, when the runner's own
# resolver has no answer. Apt then uses the address without asking again.
pin_host() {
  local host="$1"
  if getent hosts "$host" >/dev/null 2>&1; then
    return 0
  fi
  local ip
  ip=$(curl -fsS --max-time 8 -H 'accept: application/dns-json' \
    "https://1.1.1.1/dns-query?name=${host}&type=A" |
    python3 -c 'import json,sys
data = json.load(sys.stdin)
for answer in data.get("Answer", []):
    if answer.get("type") == 1:
        print(answer["data"])
        break') || true
  if [[ -n ${ip} ]]; then
    echo "${ip} ${host}" | sudo tee -a /etc/hosts >/dev/null
    echo "Pinned ${host} to ${ip}"
  fi
}

# Point every Ubuntu archive line at one mirror. The suite (noble, noble-security)
# stays as it was, so the same pockets are installed.
use_mirror() {
  local url="$1"
  local host
  host=$(echo "$url" | sed -E 's|https?://([^/]+).*|\1|')
  pin_host "$host"
  sudo find /etc/apt -type f \( -name '*.list' -o -name '*.sources' -o -name 'apt-mirrors.txt' \) -print0 |
    sudo xargs -0 -r sed -i \
      -e "s|http://azure.archive.ubuntu.com/ubuntu|${url}|g" \
      -e "s|https://azure.archive.ubuntu.com/ubuntu|${url}|g" \
      -e "s|http://archive.ubuntu.com/ubuntu|${url}|g" \
      -e "s|https://archive.ubuntu.com/ubuntu|${url}|g" \
      -e "s|http://security.ubuntu.com/ubuntu|${url}|g" \
      -e "s|https://security.ubuntu.com/ubuntu|${url}|g" \
      -e "s|http://mirrors.kernel.org/ubuntu|${url}|g" \
      -e "s|https://mirrors.kernel.org/ubuntu|${url}|g" \
      -e "s|mirror+file:/etc/apt/apt-mirrors.txt|${url}|g"
}

try_install() {
  local update_limit="$1"
  local install_limit="$2"
  sudo timeout --kill-after=15s "$update_limit" apt-get update &&
    sudo timeout --kill-after=15s "$install_limit" apt-get install -y "${packages[@]}"
}

for host in azure.archive.ubuntu.com archive.ubuntu.com security.ubuntu.com; do
  pin_host "$host"
done
if try_install 90s 3m; then
  exit 0
fi

for mirror in http://archive.ubuntu.com/ubuntu http://mirrors.kernel.org/ubuntu; do
  echo "The configured apt mirror did not finish. Retrying against ${mirror}."
  restore_sources
  use_mirror "$mirror"
  if try_install 90s 4m; then
    exit 0
  fi
done

echo "Could not install the Linux dependencies."
exit 1
