import { useEffect } from "react";
import { useAppStore } from "@/stores/app";
import { CHECK_EVERY_MS, checkIsStale, FIRST_CHECK_MS, useUpdatesStore } from "@/stores/updates";

/**
 * Look for updates shortly after start and once a day — if the user has not switched it off.
 *
 * The timer alone is not enough: a laptop that spends the night asleep wakes with its interval
 * still pending, so coming back to the window is also a moment to look, but only when the last
 * check is a day old. Nothing is downloaded either way; a newer version just lights up the pill
 * beside Settings.
 */
export function useUpdateChecks() {
  const enabled = useAppStore((s) => s.checkForUpdates);
  const isDevBuild = useAppStore((s) => s.info?.debug ?? true);

  useEffect(() => {
    if (!enabled || isDevBuild) return;
    const check = () => void useUpdatesStore.getState().check();
    const first = setTimeout(check, FIRST_CHECK_MS);
    const daily = setInterval(check, CHECK_EVERY_MS);
    const onFocus = () => {
      if (checkIsStale(useUpdatesStore.getState().checkedAt)) check();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      clearTimeout(first);
      clearInterval(daily);
      window.removeEventListener("focus", onFocus);
    };
  }, [enabled, isDevBuild]);
}
