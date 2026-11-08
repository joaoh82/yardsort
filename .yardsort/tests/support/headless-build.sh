#!/bin/sh
# headless-host.yaml's build: the stand-in server and the pretend phone's, always; then the host and
# `ys` as the `build` input says. `debug` builds them here. `package` builds the tarball
# (scripts/host-package.sh) and unpacks it in the run's folder, as someone installing it would.
# Either way, the folder they are in is written to $YS_RUN_DIR/bin, for headless.sh and host.ts.
set -e
cargo build -q -p yardsort-server --example stand-in
case "$YS_INPUT_BUILD" in
  package)
    tarball="$(scripts/host-package.sh --out "$YS_RUN_DIR/package")"
    tar -xzf "$tarball" -C "$YS_RUN_DIR/package"
    bin="$YS_RUN_DIR/package/$(basename "$tarball" .tar.gz)/bin"
    ;;
  *)
    cargo build -q -p yardsort-host -p yardsort-cli
    bin="$PWD/target/debug"
    ;;
esac
"$bin/yardsort-host" --version
printf '%s\n' "$bin" > "$YS_RUN_DIR/bin"
