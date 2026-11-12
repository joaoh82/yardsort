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

# As agents get it: the login shell's environment. Each program's whole output is captured, and
# its status checked, before its first line is printed: a missing binary, a loader error or a
# crash fails the check.
tools='
  [ "$(id -un)" = yardsort ] || { echo "runs as $(id -un), not yardsort" >&2; exit 1; }
  for tool in codex opencode pi omp gh node npm bun go cargo rustc python3 rg jq git; do
    case "$tool" in go) args=version ;; *) args=--version ;; esac
    if ! out="$("$tool" $args 2>&1)"; then
      printf "%s failed:\n%s\n" "$tool" "$out" >&2
      exit 1
    fi
    printf "%-9s %s\n" "$tool" "${out%%
*}"
  done
  echo "PATH=$PATH"
'
in_full() { docker run --rm --entrypoint bash "$@"; }
in_full "$full" -lc "$tools"

# The check itself: a harness that fails, first on PATH as one install-agent put there, fails it.
broken="$(mktemp)"
printf '%s\n' '#!/bin/sh' 'echo "intentional failure" >&2' 'exit 42' > "$broken"
chmod 755 "$broken"
if failing="$(in_full -v "$broken:/home/yardsort/.local/bin/pi:ro" "$full" -lc "$tools" 2>&1)"; then
  rm -f "$broken"
  echo "the check passed a pi that fails" >&2
  exit 1
fi
rm -f "$broken"
case "$failing" in
  *"pi failed:"*"intentional failure"*) echo "a failing harness fails the check: ok" ;;
  *) printf 'the check failed, but not on pi:\n%s\n' "$failing" >&2; exit 1 ;;
esac

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
