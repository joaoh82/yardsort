#!/usr/bin/env bash
# Check the headless host's two images: both run their programs, and the full one has every tool
# and bundled harness on the PATH an agent gets (a login shell's), runs as `yardsort`, and takes a
# harness from an installer with `install-agent`, into the home volume, where a new container
# finds it. The installer is a stand-in: nothing is downloaded from a vendor.
#
# usage: scripts/host-image-check.sh <slim image> <full image>
# Used by CI's "Host package" job and by the host-images smoke test.
set -euo pipefail
cd "$(dirname "$0")/.."

slim="${1:?usage: host-image-check.sh <slim image> <full image>}"
full="${2:?usage: host-image-check.sh <slim image> <full image>}"

for image in "$slim" "$full"; do
  docker run --rm "$image" --version
  docker run --rm --entrypoint ys "$image" --version
done

# As agents get it: the login shell's environment.
in_full() { docker run --rm --entrypoint bash "$@"; }
in_full "$full" -lc '
  set -e
  [ "$(id -un)" = yardsort ] || { echo "runs as $(id -un), not yardsort" >&2; exit 1; }
  for tool in codex opencode pi omp; do
    printf "%-9s %s\n" "$tool" "$("$tool" --version 2>&1 | head -1)"
  done
  for tool in gh node npm bun go cargo rustc python3 rg jq git; do
    case "$tool" in go) version="$(go version)" ;; *) version="$("$tool" --version 2>&1 | head -1)" ;; esac
    printf "%-9s %s\n" "$tool" "$version"
  done
  echo "PATH=$PATH"
'

# install-agent, from a stand-in installer, into a home volume a second container sees.
volume="yardsort-host-check-$$"
trap 'docker volume rm -f "$volume" >/dev/null' EXIT
docker run --rm -v "$volume:/home/yardsort" \
  -v "$PWD/crates/host/tests/stand-in-installer.sh:/stand-in/install.sh:ro" \
  "$full" install-agent claude --installer /stand-in/install.sh
found="$(in_full -v "$volume:/home/yardsort" "$full" -lc 'command -v claude && claude --version')"
echo "$found"
case "$found" in
  /home/yardsort/.local/bin/claude*"(stand-in)"*) echo "install-agent: ok" ;;
  *) echo "install-agent: claude is not where a new container finds it" >&2; exit 1 ;;
esac
