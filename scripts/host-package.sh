#!/usr/bin/env bash
# The headless host's tarball: `yardsort-host` and `ys`, built for release, with the systemd user
# unit, a README and the licence. The release workflow attaches one per architecture; the smoke
# test `headless-host` unpacks one and runs it with an empty environment. See
# docs/guide/headless-host.md.
#
#   scripts/host-package.sh [--target <rust target>] [--out <dir>]
#
# Prints the tarball's path, `<out>/yardsort-host-<version>-linux-<arch>.tar.gz`. Linux only: there
# is no host to package for anything else.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target=""
out="$root/target/host-package"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) target="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    *) echo "usage: $0 [--target <rust target>] [--out <dir>]" >&2; exit 2 ;;
  esac
done

[[ "$(uname -s)" == Linux ]] || { echo "The host's tarball is built on Linux." >&2; exit 1; }

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
triple="${target:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$triple" in
  x86_64-unknown-linux-gnu) arch=x86_64 ;;
  aarch64-unknown-linux-gnu) arch=aarch64 ;;
  *) echo "No tarball for $triple: x86_64 and aarch64 Linux only." >&2; exit 1 ;;
esac

cd "$root"
# Progress goes to stderr, so that stdout is the path alone.
cargo build --release --locked -p yardsort-host -p yardsort-cli ${target:+--target "$target"} >&2
built="target/${target:+$target/}release"

name="yardsort-host-$version-linux-$arch"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
install -Dm755 "$built/yardsort-host" "$built/ys" -t "$stage/$name/bin/"
install -Dm644 crates/host/packaging/yardsort-host.service -t "$stage/$name/systemd/"
install -Dm644 crates/host/packaging/README.md LICENSE -t "$stage/$name/"

mkdir -p "$out"
tarball="$(cd "$out" && pwd)/$name.tar.gz"
# Owned by nobody in particular, and the same bytes for the same build.
tar -C "$stage" --owner=0 --group=0 --numeric-owner --sort=name \
  --mtime="@${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}" -cf - "$name" | gzip -9n > "$tarball"
echo "$tarball"
