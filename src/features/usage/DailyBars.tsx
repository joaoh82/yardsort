import { useState } from "react";
import type { Series } from "./AreaChart";
import { niceMax } from "./format";

/**
 * One column per day, its series stacked in their fixed order with a gap between them. Hovering
 * a day shows each series' figure and the day's total.
 */
export function DailyBars(props: {
  label: string;
  series: Series[];
  /** One per day. */
  days: string[];
  /** The indexes of `days` to name along the bottom. */
  ticks: number[];
  format: (value: number) => string;
  height?: number;
}) {
  const { series, days, format, height = 200 } = props;
  const [hover, setHover] = useState<number | null>(null);
  const totals = days.map((_, i) => series.reduce((sum, s) => sum + (s.values[i] ?? 0), 0));
  const top = niceMax(Math.max(0, ...totals));
  const slot = days.length > 0 ? 100 / days.length : 100;
  const at = (i: number) => (i + 0.5) * slot;

  return (
    <figure aria-label={props.label} className="m-0 flex gap-2">
      <div
        aria-hidden
        className="flex shrink-0 flex-col justify-between text-right text-[11px] text-ink-faint tabular-nums"
        style={{ height }}
      >
        {[top, (top * 2) / 3, top / 3, 0].map((value) => (
          <span key={value} className="-my-1.5 leading-3">
            {format(value)}
          </span>
        ))}
      </div>
      <div className="min-w-0 flex-1">
        <div
          className="relative flex items-end border-b border-line"
          style={{ height }}
          onMouseLeave={() => setHover(null)}
        >
          {[1 / 3, 2 / 3, 1].map((f) => (
            <span
              key={f}
              aria-hidden
              className="absolute inset-x-0 border-t border-dashed border-line"
              style={{ bottom: `${f * 100}%` }}
            />
          ))}
          {days.map((day, i) => (
            <div
              key={day}
              className="relative flex h-full flex-1 flex-col-reverse items-center justify-start"
              onMouseEnter={() => setHover(i)}
            >
              {hover === i && <span aria-hidden className="absolute inset-0 bg-raised/60" />}
              {series.map((s, k) => {
                const value = s.values[i] ?? 0;
                if (value <= 0) return null;
                // A gap of the surface between stacked segments; none under the lowest.
                const lowest = series.slice(0, k).every((below) => (below.values[i] ?? 0) <= 0);
                return (
                  <span
                    key={s.id}
                    className={`relative w-[70%] max-w-6 rounded-[2px] ${
                      lowest ? "" : "border-b-2 border-canvas"
                    }`}
                    style={{
                      height: `${(value / top) * 100}%`,
                      minHeight: 3,
                      background: s.color,
                    }}
                  />
                );
              })}
            </div>
          ))}
          {hover !== null && (
            <div
              role="tooltip"
              className="pointer-events-none absolute top-1 z-10 min-w-36 rounded border border-line bg-raised px-2 py-1.5 text-[11px] shadow-lg"
              style={
                at(hover) > 60
                  ? { right: `${100 - at(hover) + 2}%` }
                  : { left: `${at(hover) + 2}%` }
              }
            >
              <div className="mb-1 text-ink-muted">{days[hover]}</div>
              {series.map((s) => (
                <div key={s.id} className="flex items-center gap-2">
                  <span aria-hidden className="size-2 rounded-sm" style={{ background: s.color }} />
                  <span className="flex-1 text-ink-muted">{s.label}</span>
                  <span className="text-ink tabular-nums">{format(s.values[hover] ?? 0)}</span>
                </div>
              ))}
              {series.length > 1 && (
                <div className="mt-1 flex border-t border-line pt-1">
                  <span className="flex-1 text-ink-muted">Total</span>
                  <span className="text-ink tabular-nums">{format(totals[hover] ?? 0)}</span>
                </div>
              )}
            </div>
          )}
        </div>
        <div aria-hidden className="relative mt-1 h-4 text-[11px] text-ink-faint">
          {props.ticks.map((i) => (
            <span
              key={i}
              className="absolute whitespace-nowrap"
              style={{
                left: `${at(i)}%`,
                transform: `translateX(${i === 0 ? "-25%" : i === days.length - 1 ? "-75%" : "-50%"})`,
              }}
            >
              {days[i]}
            </span>
          ))}
        </div>
      </div>
    </figure>
  );
}
