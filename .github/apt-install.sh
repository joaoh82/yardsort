#!/usr/bin/env bash
# The same packages the Linux CI jobs have always installed. The Azure Ubuntu mirror
# (azure.archive.ubuntu.com) has ignored every fetch until the step's 15-minute limit,
# so this tries that mirror with a short deadline and, if it does not finish, the
# Ubuntu archive. Nothing here is skipped.
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
Acquire::Retries "5";
Acquire::http::Timeout "20";
Acquire::https::Timeout "20";
EOF

# A stalled mirror must fail this attempt, not hold the job until its own timeout.
# timeout is root so it kills apt-get itself, and the lock is released.
if ! sudo timeout --kill-after=15s 3m apt-get update \
  || ! sudo timeout --kill-after=15s 3m apt-get install -y "${packages[@]}"; then
  echo "The configured apt mirror did not finish. Retrying against archive.ubuntu.com."
  sudo find /etc/apt -type f \( -name '*.list' -o -name '*.sources' -o -name 'apt-mirrors.txt' \) -print0 |
    sudo xargs -0 -r sed -i \
      -e 's|http://azure.archive.ubuntu.com/ubuntu|http://archive.ubuntu.com/ubuntu|g' \
      -e 's|https://azure.archive.ubuntu.com/ubuntu|http://archive.ubuntu.com/ubuntu|g' \
      -e 's|mirror+file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu|g'
  sudo apt-get update
  sudo apt-get install -y "${packages[@]}"
fi
