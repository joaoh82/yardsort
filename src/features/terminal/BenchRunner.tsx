import { useEffect, useRef, useState } from "react";
import { ipc, type SessionId } from "@/lib/ipc";
import { isMac } from "@/lib/platform";
import { percentiles, round } from "./bench";
import { TerminalView, type TerminalProbe } from "./TerminalView";

/** How long output must stay quiet after the process exits before the run is considered over. */
const SETTLE_MS = 400;

/**
 * Dev-only: run a script in a terminal while recording how smoothly the webview keeps painting,
 * then report to the core (which prints the result and quits). Started with
 * `YARDSORT_BENCH='<script>' bun tauri dev`. See docs/design/07-terminal-benchmarks.md.
 */
export function BenchRunner({ script, renderer }: { script: string; renderer: string | null }) {
  const [sessionId, setSessionId] = useState<SessionId | null>(null);
  const stats = useRef({
    renderer: "unknown",
    bytes: 0,
    firstOutput: 0,
    lastOutput: 0,
    frames: [] as number[],
  });

  useEffect(() => {
    let cancelled = false;
    let raf = 0;
    let last = 0;
    const tick = (now: number) => {
      const s = stats.current;
      // Only frames painted while output is flowing say anything about rendering cost.
      if (s.firstOutput && last) s.frames.push(now - last);
      last = now;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    const windows = navigator.userAgent.includes("Windows");
    const [program, flag] = windows ? ["cmd.exe", "/C"] : ["sh", "-c"];
    void (async () => {
      // Listen before spawning: a short script can exit before `ptySpawn` even returns.
      const exited = new Set<string>();
      let onExit = () => {};
      const unlisten = await ipc.onHostEvent((event) => {
        if (event.type !== "exited") return;
        exited.add(event.id);
        onExit();
      });

      const session = await ipc.ptySpawn({
        program,
        args: [flag, script],
        cwd: null,
        workspaceId: null,
        harness: null,
        size: { cols: 120, rows: 40 },
      });
      if (cancelled) return unlisten();
      setSessionId(session.id);

      await new Promise<void>((resolve) => {
        onExit = () => exited.has(session.id) && resolve();
        onExit();
      });
      unlisten();
      // Exit is announced once the core has sent everything; wait for xterm to finish parsing.
      while (performance.now() - stats.current.lastOutput < SETTLE_MS) {
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      cancelAnimationFrame(raf);
      await ipc.benchReport(JSON.stringify(summarise(stats.current, script)));
    })().catch((error) =>
      ipc.benchReport(JSON.stringify({ error: String(error?.message ?? error) })),
    );

    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [script]);

  const probe: TerminalProbe = {
    onRenderer: (kind) => (stats.current.renderer = kind),
    onOutput: (bytes) => {
      const s = stats.current;
      const now = performance.now();
      if (!s.firstOutput) s.firstOutput = now;
      s.lastOutput = now;
      s.bytes += bytes;
    },
  };

  return sessionId ? (
    <TerminalView sessionId={sessionId} rendererOverride={renderer} probe={probe} />
  ) : null;
}

function summarise(s: BenchStats, script: string) {
  const seconds = Math.max(s.lastOutput - s.firstOutput, 1) / 1000;
  return {
    workload: "throughput",
    script,
    renderer: s.renderer,
    platform: isMac ? "macos" : navigator.userAgent.includes("Windows") ? "windows" : "linux",
    devicePixelRatio: window.devicePixelRatio,
    megabytes: round(s.bytes / 1_048_576),
    seconds: round(seconds),
    megabytesPerSecond: round(s.bytes / 1_048_576 / seconds),
    frames: s.frames.length,
    fps: round(s.frames.length / seconds),
    frameMs: percentiles(s.frames),
    framesOver50ms: s.frames.filter((ms) => ms > 50).length,
  };
}

type BenchStats = {
  renderer: string;
  bytes: number;
  firstOutput: number;
  lastOutput: number;
  frames: number[];
};
