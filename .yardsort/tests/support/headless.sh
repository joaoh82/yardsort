#!/bin/sh
# The operator's shell on a computer with no screen, for headless-host.yaml: this run's profile,
# no display at all, and none of a launched session's markers. Runs what it is given.
#
#   sh .yardsort/tests/support/headless.sh <command> [args…]
export YARDSORT_DATA_DIR="$YS_RUN_DIR/profile" YARDSORT_WORKTREE_ROOT="$YS_RUN_DIR/worktrees"
exec env -u DISPLAY -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE \
  -u YARDSORT_RUN_ID -u YARDSORT_WORKSPACE_ID -u YARDSORT_SESSION_RECORD_ID "$@"
