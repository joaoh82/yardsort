import { useEffect, useRef, useState } from "react";
import { ipc, type SessionId } from "@/lib/ipc";
import { isMac } from "@/lib/platform";
import { percentiles } from "./bench";
import { TerminalView, type TerminalProbe } from "./TerminalView";

/** How many round trips to time. Enough for a p99 without making the run tedious. */
const SAMPLES = 120;
/**
 * Gap between samples. A keystroke arrives into a terminal that has been sitting still, and that
 * is the case worth measuring: output batching behaves differently mid-stream, which the
 * throughput workloads already cover.
 */
const GAP_MS = 50;
/** Let the holder process start and the view attach before timing anything. */
const SETTLE_MS = 1_000;
/** A round trip this slow means something is wrong; report it rather than hang. */
const GIVE_UP_MS = 5_000;

/**
 * Dev-only: time how long a byte takes to go from the webview, through the daemon and the PTY,
 * and back onto the screen. Started with `YARDSORT_BENCH_LATENCY=1 bun tauri dev`. See
 * docs/design/07-terminal-benchmarks.md.
 *
 * Nothing is running in the terminal that could answer: the echo comes from the PTY's own line
 * discipline, which turns the byte around in the kernel. What is left is Yardsort's share of the
 * delay — IPC, the daemon hop, the pump, the parser and the renderer — with no agent's think
 * time mixed into it.
 */
export function LatencyRunner({ renderer }: { renderer: string | null }) {
  const [sessionId, setSessionId] = useState<SessionId | null>(null);
  const stats = useRef({ renderer: "unknown", echo: 0, paint: 0, wake: () => {} });

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const windows = navigator.userAgent.includes("Windows");
      // A process that holds the PTY open and prints nothing, so every byte that comes back is
      // the echo of one we sent.
      const [program, args] = windows
        ? ["cmd.exe", ["/C", "ping -n 60 127.0.0.1 >NUL"]]
        : ["sh", ["-c", "sleep 60"]];
      const session = await ipc.ptySpawn({
        program,
        args,
        cwd: null,
        workspaceId: null,
        harness: null,
        size: { cols: 120, rows: 40 },
      });
      if (cancelled) return;
      setSessionId(session.id);
      await pause(SETTLE_MS);

      const echoes: number[] = [];
      const paints: number[] = [];
      for (let i = 0; i < SAMPLES && !cancelled; i++) {
        const s = stats.current;
        s.echo = 0;
        s.paint = 0;
        const settled = new Promise<void>((resolve) => (s.wake = resolve));
        const sent = performance.now();
        await ipc.ptyWrite(session.id, "x");
        await Promise.race([settled, pause(GIVE_UP_MS)]);
        if (s.echo && s.paint) {
          echoes.push(s.echo - sent);
          paints.push(s.paint - sent);
        }
        await pause(GAP_MS);
      }
      if (cancelled) return;

      await ipc.ptyClose(session.id).catch(() => {});
      await ipc.benchReport(
        JSON.stringify({
          workload: "latency",
          renderer: stats.current.renderer,
          platform: isMac ? "macos" : windows ? "windows" : "linux",
          devicePixelRatio: window.devicePixelRatio,
          samples: echoes.length,
          lost: SAMPLES - echoes.length,
          echoMs: percentiles(echoes, 2),
          paintMs: percentiles(paints, 2),
        }),
      );
    })().catch((error) =>
      ipc.benchReport(JSON.stringify({ error: String(error?.message ?? error) })),
    );

    return () => {
      cancelled = true;
    };
  }, []);

  const probe: TerminalProbe = {
    onRenderer: (kind) => (stats.current.renderer = kind),
    // The byte has been parsed by xterm but is not on screen yet.
    onOutput: () => {
      const s = stats.current;
      if (!s.echo) s.echo = performance.now();
    },
    // The first frame painted after the echo arrived is the frame carrying it: xterm parses
    // before it renders, so a frame that preceded the echo cannot have shown it.
    onPaint: () => {
      const s = stats.current;
      if (s.echo && !s.paint) {
        s.paint = performance.now();
        s.wake();
      }
    },
  };

  return sessionId ? (
    <TerminalView sessionId={sessionId} rendererOverride={renderer} probe={probe} />
  ) : null;
}

const pause = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
