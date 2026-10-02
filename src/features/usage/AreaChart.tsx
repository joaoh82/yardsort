import { useState } from "react";
import { niceMax } from "./format";

export interface Series {
  id: string;
  label: string;
  /** A CSS colour, normally one of the `--color-agent-*` tokens. */
  color: string;
  values: number[];
}

const W = 1000;

/**
 * Overlaid areas, one per series, sharing one axis. Lines are drawn without smoothing, so a
 * peak is where the data has it. Hovering shows every series at that point.
 */
export function AreaChart(props: {
  label: string;
  series: Series[];
  /** One per point, for the tooltip; also the x-axis when `ticks` is given. */
  points: string[];
  /** Where each point sits across the chart, 0 to 1. Evenly spread when absent. */
  xs?: number[];
  format: (value: number) => string;
  height?: number;
  /** Axis labels along the bottom: the indexes of `points` to name. */
  ticks?: number[];
  /** Show the value scale on the left. */
  scale?: boolean;
  /** A fixed top, e.g. 100 for a percentage; otherwise rounded up from the data. */
  max?: number;
}) {
  const { series, points, format, height = 200 } = props;
  const [hover, setHover] = useState<number | null>(null);
  const count = points.length;
  const xs = props.xs ?? points.map((_, i) => (count > 1 ? i / (count - 1) : 0.5));
  const top = props.max ?? niceMax(Math.max(0, ...series.flatMap((s) => s.values)));
  const H = 100;
  const x = (i: number) => xs[i]! * W;
  const y = (value: number) => H - (Math.min(value, top) / top) * H;

  const pathOf = (values: number[]) =>
    values.map((v, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(v).toFixed(2)}`).join("");

  const onMove = (event: React.MouseEvent<HTMLDivElement>) => {
    if (count === 0) return;
    const box = event.currentTarget.getBoundingClientRect();
    const at = (event.clientX - box.left) / box.width;
    let nearest = 0;
    for (let i = 1; i < count; i++)
      if (Math.abs(xs[i]! - at) < Math.abs(xs[nearest]! - at)) nearest = i;
    setHover(nearest);
  };

  return (
    <figure aria-label={props.label} className="m-0 flex gap-2">
      {props.scale && (
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
      )}
      <div className="min-w-0 flex-1">
        <div
          className="relative"
          style={{ height }}
          onMouseMove={onMove}
          onMouseLeave={() => setHover(null)}
        >
          <svg
            viewBox={`0 0 ${W} ${H}`}
            preserveAspectRatio="none"
            className="absolute inset-0 size-full overflow-visible"
            aria-hidden
          >
            {props.scale &&
              [0, 1 / 3, 2 / 3].map((f) => (
                <line
                  key={f}
                  x1={0}
                  x2={W}
                  y1={f * H}
                  y2={f * H}
                  stroke="var(--color-line)"
                  strokeDasharray="2 4"
                  vectorEffect="non-scaling-stroke"
                />
              ))}
            <line
              x1={0}
              x2={W}
              y1={H}
              y2={H}
              stroke="var(--color-line)"
              vectorEffect="non-scaling-stroke"
            />
            {count > 0 &&
              series.map((s) => (
                <g key={s.id}>
                  <path
                    d={`${pathOf(s.values)}L${x(count - 1)},${H}L${x(0)},${H}Z`}
                    fill={s.color}
                    fillOpacity={0.12}
                  />
                  <path
                    d={pathOf(s.values)}
                    fill="none"
                    stroke={s.color}
                    strokeWidth={2}
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                  />
                </g>
              ))}
            {hover !== null && (
              <line
                x1={x(hover)}
                x2={x(hover)}
                y1={0}
                y2={H}
                stroke="var(--color-ink-faint)"
                vectorEffect="non-scaling-stroke"
              />
            )}
          </svg>
          {hover !== null && (
            <div
              role="tooltip"
              className="pointer-events-none absolute top-1 z-10 min-w-32 rounded border border-line bg-raised px-2 py-1.5 text-[11px] shadow-lg"
              style={
                xs[hover]! > 0.6
                  ? { right: `${(1 - xs[hover]!) * 100 + 1}%` }
                  : { left: `${xs[hover]! * 100 + 1}%` }
              }
            >
              <div className="mb-1 text-ink-muted">{points[hover]}</div>
              {series.map((s) => (
                <div key={s.id} className="flex items-center gap-2">
                  <span aria-hidden className="size-2 rounded-sm" style={{ background: s.color }} />
                  <span className="flex-1 text-ink-muted">{s.label}</span>
                  <span className="text-ink tabular-nums">{format(s.values[hover] ?? 0)}</span>
                </div>
              ))}
            </div>
          )}
        </div>
        {props.ticks && (
          <div aria-hidden className="relative mt-1 h-4 text-[11px] text-ink-faint">
            {props.ticks.map((i) => (
              <span
                key={i}
                className="absolute whitespace-nowrap"
                style={{
                  left: `${xs[i]! * 100}%`,
                  transform: `translateX(${xs[i]! < 0.05 ? "0" : xs[i]! > 0.95 ? "-100%" : "-50%"})`,
                }}
              >
                {points[i]}
              </span>
            ))}
          </div>
        )}
      </div>
    </figure>
  );
}
