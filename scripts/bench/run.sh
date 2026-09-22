#!/bin/sh
# Run the terminal benchmarks against a dev build and print one JSON result per run.
# See docs/design/07-terminal-benchmarks.md.
#
# usage: scripts/bench/run.sh              # every workload x both renderers
#        scripts/bench/run.sh latency      # only the latency workload
#        scripts/bench/run.sh throughput   # only cat and repaint
set -eu
cd "$(dirname "$0")/../.."
here="$(pwd)/scripts/bench"
work="${TMPDIR:-/tmp}/yardsort-bench"
only="${1:-all}"

# One run, with a profile of its own.
#
# The data directory is not only about keeping the benchmark off real projects: the daemon that
# owns the terminals outlives the app, and its socket is named after the data directory. Reusing
# one means the next run connects to the daemon an earlier run left listening — built from
# whatever the code said *then*. That silently measures the wrong binary, and the numbers look
# perfectly plausible. A fresh directory forces a daemon from the build under test.
run() {
  dir="$(mktemp -d "${TMPDIR:-/tmp}/yardsort-bench-profile.XXXXXX")"
  env YARDSORT_DATA_DIR="$dir" YARDSORT_WORKTREE_ROOT="$dir/worktrees" "$@" \
    bun tauri dev 2>&1 | grep -a --line-buffered 'YARDSORT_BENCH_RESULT' | head -1
  # The daemon lets go of it within its idle grace; nothing here needs to wait for that.
  rm -rf "$dir"
}

# How long one byte takes to get from the webview to the screen and back. Nothing else here
# measures that: the throughput workloads keep the pipeline permanently full, which is the one
# condition under which latency cannot be seen.
if [ "$only" = all ] || [ "$only" = latency ]; then
  for renderer in webgl dom; do
    run YARDSORT_RENDERER="$renderer" YARDSORT_BENCH_LATENCY=1
  done
fi

# How much output the terminal absorbs, and whether it keeps painting while it does.
if [ "$only" = all ] || [ "$only" = throughput ]; then
  mkdir -p "$work"
  [ -f "$work/big.txt" ] || python3 "$here/make-big-file.py" "$work/big.txt"
  for workload in "cat $work/big.txt" "python3 $here/repaint.py"; do
    for renderer in webgl dom; do
      run YARDSORT_RENDERER="$renderer" YARDSORT_BENCH="$workload"
    done
  done
fi
