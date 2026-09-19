"use client";

import { useSyncExternalStore } from "react";

export type Platform = "linux" | "mac" | "win";

const NAMES: Record<Platform, string> = { mac: "macOS", win: "Windows", linux: "Linux" };

function detect(): Platform | null {
  const ua = `${navigator.userAgent} ${navigator.platform ?? ""}`;
  if (/Mac/i.test(ua)) return "mac";
  if (/Win/i.test(ua)) return "win";
  if (/Linux|X11/i.test(ua)) return "linux";
  return null;
}

const subscribe = () => () => {};

// The visitor's platform; null while rendering on the server and without JavaScript, so the page
// reads correctly either way.
function usePlatform(): Platform | null {
  return useSyncExternalStore(subscribe, detect, () => null);
}

export function DownloadLabel() {
  const platform = usePlatform();
  return <>{platform ? `Download for ${NAMES[platform]}` : "Download"}</>;
}

export function Detected({ platform }: { platform: Platform }) {
  const current = usePlatform();
  if (current !== platform) return null;
  return <span className="font-mono text-[11px] text-accent">detected</span>;
}
