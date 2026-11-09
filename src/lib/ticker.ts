import { useSyncExternalStore } from "react";

/**
 * One clock for everything on screen that counts: a single interval, running only while
 * something listens. Every counter changes on the same tick, so however many there are, the
 * window repaints at most once a second for them. A CSS animation, or an interval per counter,
 * would instead keep the webview painting frame after frame — on Linux the whole window, every
 * refresh of the screen.
 */
const TICK_MS = 1000;

let now = Date.now();
let timer: ReturnType<typeof setInterval> | undefined;
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (timer === undefined) {
    now = Date.now();
    timer = setInterval(() => {
      now = Date.now();
      for (const notify of listeners) notify();
    }, TICK_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

const snapshot = () => now;

/** The time of the last shared tick, in epoch milliseconds. Re-renders once a second. */
export function useNow(): number {
  return useSyncExternalStore(subscribe, snapshot);
}
