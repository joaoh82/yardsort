/** Numbers as the Usage view shows them. */

export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

export function formatPercent(percent: number): string {
  if (percent > 0 && percent < 0.1) return "<0.1%";
  return `${percent.toFixed(1)}%`;
}

/** 6.8B, 81.4M, 17.8K, 950. */
export function formatTokens(tokens: number): string {
  for (const [size, unit] of [
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ] as const) {
    if (tokens >= size) return `${(tokens / size).toFixed(1)}${unit}`;
  }
  return String(Math.round(tokens));
}

/** Whole dollars once they are large enough for cents to be noise. */
export function formatCost(dollars: number): string {
  if (dollars >= 1000)
    return `$${Math.round(dollars).toLocaleString("en-US", { maximumFractionDigits: 0 })}`;
  if (dollars >= 100) return `$${Math.round(dollars)}`;
  if (dollars > 0 && dollars < 0.01) return "<$0.01";
  return `$${dollars.toFixed(2)}`;
}

/** A share of a whole, rounded; tiny but non-zero shares do not read as nothing. */
export function share(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const percent = (part / whole) * 100;
  if (percent > 0 && percent < 1) return "<1%";
  return `${Math.round(percent)}%`;
}

/** How long until a moment: 3h 54m, 5d 19h, 12m. */
export function formatUntil(ms: number): string {
  const minutes = Math.max(0, Math.round(ms / 60_000));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes % 60}m`;
  return `${minutes}m`;
}

/** A rate-limit window by its length, as the vendors name them. */
export function windowName(minutes: number | null): string {
  if (minutes === null) return "Limit";
  if (minutes === 300) return "Session (5h)";
  if (minutes === 10080) return "Weekly";
  if (minutes % 1440 === 0) return `${minutes / 1440}-day`;
  if (minutes % 60 === 0) return `${minutes / 60}-hour`;
  return `${minutes}-minute`;
}

/** "Sep 24" from "2026-09-24", without the time zone moving it a day. */
export function shortDate(date: string): string {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(year!, month! - 1, day!).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
}

/** A step up from `max` that reads well on an axis: 1, 2, 2.5 or 5 times a power of ten. */
export function niceMax(max: number): number {
  if (max <= 0) return 1;
  const power = 10 ** Math.floor(Math.log10(max));
  for (const step of [1, 2, 2.5, 5, 10]) if (step * power >= max) return step * power;
  return 10 * power;
}
