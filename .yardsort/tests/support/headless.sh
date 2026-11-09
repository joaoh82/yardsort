#!/bin/sh
# The operator's shell on a computer with no screen, for headless-host.yaml: this run's profile,
# no display at all, and none of a launched session's markers. Runs what it is given, with
# `yardsort-host` and `ys` the ones headless-build.sh put in $YS_RUN_DIR/bin.
#
#   sh .yardsort/tests/support/headless.sh <yardsort-host | ys> [args…]
#
# Built as the tarball, they run with an empty environment: `HOME`, a `PATH` of the tarball's
# folder and the system's, and the run's profile, nothing else. What they need beyond that, they
# would not find on a server either.
if [ "$YS_INPUT_BUILD" = image ]; then
  # CI's, for the container image: its entry point is the host, on this run's profile and
  # sign-in, reaching the stand-in on this computer's loopback.
  [ "$1" = yardsort-host ] || { echo "The image runs yardsort-host, not $1." >&2; exit 2; }
  shift
  exec docker run -i --rm --network host -v "$YS_RUN_DIR/profile:/data" \
    -v "$YS_RUN_DIR/credentials:/credentials" yardsort-host "$@"
fi
bin="$(cat "$YS_RUN_DIR/bin")"
program="$bin/$1"
shift
data="$YS_RUN_DIR/profile" worktrees="$YS_RUN_DIR/worktrees"
if [ "$YS_INPUT_BUILD" = package ]; then
  exec env -i HOME="$HOME" PATH="$bin:/usr/bin:/bin" \
    YARDSORT_DATA_DIR="$data" YARDSORT_WORKTREE_ROOT="$worktrees" "$program" "$@"
fi
export YARDSORT_DATA_DIR="$data" YARDSORT_WORKTREE_ROOT="$worktrees"
exec env -u DISPLAY -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE \
  -u YARDSORT_RUN_ID -u YARDSORT_WORKSPACE_ID -u YARDSORT_SESSION_RECORD_ID "$program" "$@"
