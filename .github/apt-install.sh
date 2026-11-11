#!/usr/bin/env bash
# The same packages the Linux CI jobs have always installed.
#
# GitHub's Ubuntu runners fetch these from azure.archive.ubuntu.com. That name,
# and then archive.ubuntu.com, has failed DNS ("Temporary failure resolving")
# while other hosts still resolved, and apt-get update exits 0 after ignoring
# the missed indexes. This pins a host the system resolver missed, using
# DNS-over-HTTPS, then tries the configured mirror, the Ubuntu archive, and
# kernel.org. Each attempt is deadline-bounded so a stall cannot hold the step.
#
# Apt reads a list of Ubuntu sources only. Error-Mode any still fails the update
# when one of those indexes is missed, and a third-party source already on the
# runner (Microsoft, for example) cannot fail it.
#
# Worst case, every deadline expires and every DNS-over-HTTPS lookup misses.
# getent is signalled at 15s and killed 1s later; curl stops at 8s. A missed
# lookup is 24s. update and install share one 15s kill window per attempt,
# because install runs only when update exits in time. Before each retry,
# configure, -f install, and configure again each have their own kill window:
# a killed -f install leaves dpkg interrupted, and the second configure clears
# that before this attempt's install.
#   5 lookups × 24s                                              =  120s
#   attempt 1: 45s update + 2m install + 15s kill                =  180s
#   before attempt 2: 3 × (30s + 15s kill)                       =  135s
#   attempt 2: 45s update + 3m install + 15s kill                =  240s
#   before attempt 3: 3 × (30s + 15s kill)                       =  135s
#   attempt 3: 45s update + 3m install + 15s kill                =  240s
#                                                                -------
#                                                                 1050s
# The step timeout is 20 minutes (1200s), so the kernel.org attempt still
# finishes inside it.
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

# A config-file question would hold dpkg until the deadline killed it.
export DEBIAN_FRONTEND=noninteractive

sudo tee /etc/apt/apt.conf.d/99-yardsort-retries >/dev/null <<'EOF'
Acquire::Retries "2";
Acquire::http::Timeout "12";
Acquire::https::Timeout "12";
# A missed index is a failed update. The default ignores it and exits 0,
# and the install then cannot find these packages.
APT::Update::Error-Mode "any";
EOF

# One classic sources.list, built from the runner's Ubuntu entries. Apt does
# not read deb822 from Dir::Etc::SourceList, and SourceParts is /dev/null so
# sources.list.d (Microsoft and the rest) is not consulted.
ubuntu_list=$(mktemp)
python3 - "$ubuntu_list" /etc/apt/sources.list.d/ubuntu.sources /etc/apt/sources.list <<'PY'
import pathlib, sys

dest = pathlib.Path(sys.argv[1])
deb822_path = pathlib.Path(sys.argv[2])
classic_path = pathlib.Path(sys.argv[3])
needles = (
    "archive.ubuntu.com",
    "security.ubuntu.com",
    "ports.ubuntu.com",
    "mirrors.kernel.org",
    "apt-mirrors.txt",
)

def ubuntu(uri: str) -> bool:
    return any(needle in uri for needle in needles)

lines = []

def add_deb822(text: str) -> None:
    stanzas = []
    current = {}

    def flush() -> None:
        nonlocal current
        if current:
            stanzas.append(current)
            current = {}

    for raw in text.splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            flush()
            continue
        if ":" not in raw:
            continue
        key, value = raw.split(":", 1)
        current.setdefault(key.strip(), [])
        current[key.strip()].append(value.strip())
    flush()
    for stanza in stanzas:
        types = " ".join(stanza.get("Types", [])).split()
        uris = [uri for uri in " ".join(stanza.get("URIs", [])).split() if ubuntu(uri)]
        suites = " ".join(stanza.get("Suites", [])).split()
        components = " ".join(stanza.get("Components", [])).split()
        signed = " ".join(stanza.get("Signed-By", [])).split()
        option = ""
        if len(signed) == 1 and signed[0].startswith("/"):
            option = f"[signed-by={signed[0]}] "
        component_list = " ".join(components)
        for kind in types:
            for uri in uris:
                for suite in suites:
                    lines.append(f"{kind} {option}{uri} {suite} {component_list}".rstrip())

if deb822_path.is_file() and "Types:" in deb822_path.read_text():
    add_deb822(deb822_path.read_text())
elif classic_path.is_file():
    for raw in classic_path.read_text().splitlines():
        stripped = raw.strip()
        if (stripped.startswith("deb ") or stripped.startswith("deb-src ")) and ubuntu(stripped):
            lines.append(stripped)

if not lines:
    sys.stderr.write("No Ubuntu apt sources found.\n")
    sys.exit(1)
dest.write_text("\n".join(lines) + "\n")
PY

# An address from a resolver that is not this runner's, when the runner's own
# resolver has no answer. Apt then uses the address without asking again.
pin_host() {
  local host="$1"
  if timeout --kill-after=1s 15s getent hosts "$host" >/dev/null 2>&1; then
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
  sed -i \
    -e "s|http://azure.archive.ubuntu.com/ubuntu|${url}|g" \
    -e "s|https://azure.archive.ubuntu.com/ubuntu|${url}|g" \
    -e "s|http://archive.ubuntu.com/ubuntu|${url}|g" \
    -e "s|https://archive.ubuntu.com/ubuntu|${url}|g" \
    -e "s|http://security.ubuntu.com/ubuntu|${url}|g" \
    -e "s|https://security.ubuntu.com/ubuntu|${url}|g" \
    -e "s|http://mirrors.kernel.org/ubuntu|${url}|g" \
    -e "s|https://mirrors.kernel.org/ubuntu|${url}|g" \
    -e "s|mirror+file:/etc/apt/apt-mirrors.txt|${url}|g" \
    "$ubuntu_list"
}

# List-Cleanup 0 leaves the runner's other list files in place. This step never
# reads them; deleting them would still change the image for later steps.
run_apt() {
  local limit="$1"
  shift
  sudo timeout --kill-after=15s "$limit" apt-get \
    -o "Dir::Etc::SourceList=${ubuntu_list}" \
    -o Dir::Etc::SourceParts=/dev/null \
    -o APT::Get::List-Cleanup=0 \
    "$@"
}

# timeout can kill apt-get install after dpkg has started. The next attempt
# then fails with "dpkg was interrupted".
recover_interrupted() {
  sudo timeout --kill-after=15s 30s dpkg --configure -a || true
  run_apt 30s -f install -y || true
  sudo timeout --kill-after=15s 30s dpkg --configure -a || true
}

attempt=0
try_install() {
  local update_limit="$1"
  local install_limit="$2"
  if [[ $attempt -gt 0 ]]; then
    recover_interrupted
  fi
  attempt=$((attempt + 1))
  run_apt "$update_limit" update &&
    run_apt "$install_limit" install -y "${packages[@]}"
}

for host in azure.archive.ubuntu.com archive.ubuntu.com security.ubuntu.com; do
  pin_host "$host"
done
if try_install 45s 2m; then
  exit 0
fi

for mirror in http://archive.ubuntu.com/ubuntu http://mirrors.kernel.org/ubuntu; do
  echo "The configured apt mirror did not finish. Retrying against ${mirror}."
  use_mirror "$mirror"
  if try_install 45s 3m; then
    exit 0
  fi
done

echo "Could not install the Linux dependencies."
exit 1
