# 07 — Terminal benchmarks

M1's exit criterion asks for a recorded go/no-go on terminal rendering in the system webview, with
numbers. This is that record. **Verdict: go** — see [conclusions](#conclusions).

## How to run

```sh
scripts/bench/run.sh              # every workload x both renderers
scripts/bench/run.sh latency      # just the latency workload
scripts/bench/run.sh throughput   # just cat and repaint
```

Each run is a debug build that takes the workspace panel over, measures, prints a
`YARDSORT_BENCH_RESULT {json}` line and quits. `YARDSORT_RENDERER=webgl|dom` forces the renderer.
The window must be visible: webviews stop animation frames when hidden.

Three workloads:

| Workload    | What it is                                                                                         | What it stresses                                 |
| ----------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **latency** | `YARDSORT_BENCH_LATENCY=1`: time 120 single-byte round trips into an idle terminal                 | The delay on one keystroke, end to end           |
| **cat**     | `cat` a 24 MB file of coloured log lines                                                           | PTY → IPC → parser throughput; UI liveness       |
| **repaint** | `scripts/bench/repaint.py`: redraw a 120×40 screen of 256-colour cells, 600 frames at up to 120 Hz | The renderer — this is what a streaming TUI does |

What to look at: `echoMs`/`paintMs` are the two halves of a keystroke's delay; `frameMs.p95`/`p99`
and `framesOver50ms` say whether the UI stayed smooth while output was flowing;
`megabytesPerSecond` says how fast a flood is absorbed.

**Give every run a data directory of its own** — `run.sh` does. The daemon that owns the terminals
outlives the app and its socket is named after the data directory, so reusing one means connecting
to the daemon an earlier run left listening, built from whatever the code said then. That measures
the wrong binary and the numbers look entirely plausible; it invalidated a before/after here once
already.

## Results

### Linux — the hard case

Arch (Omarchy), Hyprland/Wayland, WebKitGTK 2.52.6, hybrid NVIDIA RTX 3070 Ti + AMD Radeon 680M,
devicePixelRatio 2, debug build, 2026-09-17.

| Workload | Renderer | MB/s | fps | frame p50 | p95   | p99   | max    | frames > 50 ms |
| -------- | -------- | ---- | --- | --------- | ----- | ----- | ------ | -------------- |
| cat      | webgl    | 6.3  | 171 | 6 ms      | 14 ms | 17 ms | 79 ms  | 1              |
| cat      | dom      | 6.3  | 160 | 6 ms      | 16 ms | 20 ms | 79 ms  | 1              |
| repaint  | webgl    | 1.3  | 71  | 16 ms     | 18 ms | 35 ms | 169 ms | 2              |
| repaint  | dom      | 1.3  | 73  | 17 ms     | 17 ms | 29 ms | 38 ms  | 0              |

(`repaint` is rate-limited by the script, so its MB/s is the offered load, not a ceiling.)

Latency on the same machine, debug build, 2026-09-22 — `before` is the batch window timed from the
arriving chunk, `after` is timed from the previous delivery (see [Latency](#latency)):

| Renderer | echo p50   | p95        | paint p50  | p95        |
| -------- | ---------- | ---------- | ---------- | ---------- |
| webgl    | 10 → **2** | 11 → **2** | 11 → **5** | 17 → **5** |
| dom      | 10 → **1** | 10 → **2** | 11 → **5** | 11 → **5** |

120 samples each, none lost. Every sample comes back a whole number of milliseconds: WebKitGTK
clamps `performance.now()`, so treat sub-millisecond differences from this workload as noise and
measure inside the core when they matter.

Also verified by hand on the same machine: bash with a starship prompt, truecolor, `ls` colours;
Claude Code's full-screen TUI; plain Ctrl+B / Alt+X reaching the program; Ctrl+Shift shortcuts not
reaching it; and detach → re-attach of a running Claude Code session with 60 lines of scrollback —
the repainted screen differed from the original by 39 pixels (the blinking cursor).

### macOS, Windows

Not yet measured on hardware. CI runs the PTY host's integration tests (real processes in real
PTYs, including ConPTY) on both. Run `scripts/bench/run.sh` on each before v0.1 and add the rows —
the `latency` rows above all, since "smoother on macOS than on Linux" is the one claim on this
page that nothing has yet measured.

On Windows the `latency` workload leans on ConPTY echoing what is written to it, and ConPTY
repaints on its own terms; treat the first Windows numbers as suspect until a run confirms the
echo is what is being timed.

## Latency

Throughput and frame times were good from the start, and for a long time they were the only things
measured. They say nothing about the delay on a single keystroke, and that is what the terminal
was actually being judged on: it felt slow on Linux and not quite right on macOS while every
number on this page looked healthy.

The pump gathers output for `BATCH_WINDOW` (8 ms) before handing it on, so that a full-screen TUI
repainting at 120 Hz cannot turn into hundreds of IPC messages a second. The window was timed
from the moment a chunk arrived, which charged it to _every_ delivery — including a lone
keystroke echo arriving into a terminal that had been sitting still for a minute, where there was
nothing to coalesce it with and the whole 8 ms was pure delay. Measured through `PtyHost` with
`cat` echoing single bytes, Arch/Wayland, debug build, 2026-09-22:

| Window timed from  | min     | p50     | p95     | max     |
| ------------------ | ------- | ------- | ------- | ------- |
| the arriving chunk | 8.13 ms | 8.21 ms | 8.30 ms | 8.31 ms |
| the last delivery  | 0.06 ms | 0.11 ms | 0.60 ms | 1.00 ms |

The floor is exactly the window, on every sample; the PTY itself accounts for about 0.1 ms of it.

Timing the window from the **previous delivery** keeps both properties. Output arriving into a
quiet session goes out at once; a stream fast enough to keep refilling the window is still capped
at one delivery per 8 ms. The coalescing is unchanged — 102 deliveries/s for the 120 Hz `repaint`
workload, and a 24 MB flood still fills the 512 KB `MAX_BATCH` ceiling and delivers 5 times a
second.

Covered by `output_into_a_quiet_session_is_not_held_for_the_batch_window` in
`crates/pty-host/tests/sessions.rs`.

`deliver` also fans out to the viewers _before_ parsing the batch into the headless terminal. That
parse only exists to answer snapshots on re-attach; nothing on the delivery path needs it.

### Measuring it end to end

The table above is the core alone. The `latency` workload measures the whole path from inside the
webview, which is where the question started, and it is what the numbers under
[Results](#results) come from.

It spawns a process that holds the PTY open and prints nothing (`sleep`, or `ping` on Windows),
then times 120 single-byte round trips with a 50 ms gap, so each one arrives into a terminal that
has been sitting still — the state a keystroke actually arrives in. Nothing in the terminal can
answer: the echo comes from the PTY's own line discipline, turned around in the kernel. What is
left is Yardsort's share of the delay, with no agent's think time in it.

Each round trip is split in two, because they fail differently:

- **`echoMs`** — the write leaving the webview to the bytes reaching xterm's parser. IPC out, the
  daemon socket, the PTY, the pump, the channel back.
- **`paintMs`** — the same, up to the frame that puts it on screen. `TerminalView` reports this
  through `TerminalProbe.onPaint`, driven by xterm's `onRender`. The first frame after the echo is
  the frame carrying it: xterm parses before it renders, so no earlier frame can have shown it.

The split is what tells the two suspects apart. If `echoMs` is level across platforms and
`paintMs` is not, the renderer and the compositor are to blame; if `echoMs` itself differs, it is
the IPC and the daemon hop.

On Linux both halves are small and the renderers are within noise of each other, which says the
remaining delay is not in painting. The Linux/macOS difference the report started from is still
unmeasured — macOS needs a run on hardware, as the throughput rows do.

## Conclusions

1. **WebGL works on WebKitGTK here**, on the GPU/compositor combination known for trouble, without
   `WEBKIT_DISABLE_DMABUF_RENDERER` or any other workaround.
2. **The DOM fallback is good enough to rely on.** For agent-shaped workloads it is
   indistinguishable from WebGL, so losing the GL context degrades nothing the user would notice.
3. **The UI stays live under flood.** 24 MB in 3.7 s with p99 frame time ≤ 20 ms and a single long
   frame. Agents produce kilobytes per second; this is three orders of magnitude of headroom.
4. Throughput is identical across renderers, so it is bound by parsing and IPC, not painting. If it
   ever matters, the lever is there (bigger batches, a release build), not in the renderer.
5. **Throughput was never the problem.** The terminal absorbed 24 MB without dropping a frame
   while adding 8 ms to every keystroke. Measure latency as well as throughput, or a benchmark
   this healthy will keep saying nothing is wrong.

No reason to reconsider Tauri.

## Notes

- In `tauri dev`, a page reload while IPC requests are in flight logs _"IPC custom protocol failed,
  Tauri will now use the postMessage interface"_. It is a dev-only artefact of the reload (it does
  not occur on a clean start, with or without the CSP) and the fallback is fully functional.
