/**
 * A mark for each harness.
 *
 * Agents are picked out of lists often enough — the tab strip, the composer, settings — that a
 * shape found by eye beats a word read twice, and a row of them reads as a row of *agents*
 * rather than a row of buttons. Built-ins get a glyph of their own; anything else, including
 * every harness the user adds, gets its initial in the same frame, which is still unique enough
 * to aim at. Adding a glyph for a new harness is one entry in `GLYPHS`.
 *
 * The glyphs are drawn here, from primitives, rather than shipped as image files: they inherit
 * the theme's colours, stay sharp at 14px, and cost no request. They are plain marks in the
 * spirit of each agent's own, not the trademarks themselves.
 */

interface Glyph {
  body: React.ReactNode;
  /** Only where a colour is the point — otherwise the glyph takes the text colour around it. */
  className?: string;
}

const spokes = [0, 30, 60, 90, 120, 150];

const GLYPHS: Record<string, Glyph> = {
  claude: {
    className: "text-[#d97757]",
    body: (
      <g stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        {spokes.map((deg) => (
          <line key={deg} x1="8" y1="1.6" x2="8" y2="14.4" transform={`rotate(${deg} 8 8)`} />
        ))}
      </g>
    ),
  },
  codex: {
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.2">
        {[0, 60, 120].map((deg) => (
          <ellipse key={deg} cx="8" cy="8" rx="3" ry="6.2" transform={`rotate(${deg} 8 8)`} />
        ))}
      </g>
    ),
  },
  grok: {
    body: (
      <g fill="currentColor">
        <path d="M2 2h3.6l8.4 12h-3.6z" />
        <path d="M14 2h-3.2L2 14h3.2z" />
      </g>
    ),
  },
  omp: {
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        <circle cx="8" cy="8" r="5.5" />
        <path d="M5 10V6l3 3 3-3v4" />
      </g>
    ),
  },
  cursor: {
    body: <path d="M3 1.5v12l3.5-3 2.5 4 2-1.2-2.5-4.1 4.5-.7z" fill="currentColor" />,
  },
  pi: {
    body: (
      <path
        d="M2.5 4.5h11M5.5 4.5v8M11 4.5v6.5q0 2 2 1.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    ),
  },
  opencode: {
    body: (
      <g>
        <rect
          x="2.2"
          y="2.2"
          width="11.6"
          height="11.6"
          rx="2.4"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.3"
        />
        <rect x="5.2" y="5.2" width="5.6" height="5.6" rx="1" fill="currentColor" />
      </g>
    ),
  },
};

export function HarnessIcon({
  id,
  label,
  size = 14,
  className = "",
}: {
  id: string;
  /** Used for the initial when the harness has no glyph; its id will do. */
  label?: string;
  size?: number;
  className?: string;
}) {
  const glyph = GLYPHS[id];
  const initial = (label ?? id).trim().charAt(0).toUpperCase() || "?";

  return (
    <svg
      aria-hidden
      focusable="false"
      viewBox="0 0 16 16"
      width={size}
      height={size}
      className={`shrink-0 ${glyph?.className ?? ""} ${className}`}
    >
      {glyph?.body ?? (
        <g>
          <rect
            x="1.6"
            y="1.6"
            width="12.8"
            height="12.8"
            rx="3"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
            opacity="0.5"
          />
          <text
            x="8"
            y="11.4"
            textAnchor="middle"
            fontSize="8.5"
            fontWeight="600"
            fill="currentColor"
          >
            {initial}
          </text>
        </g>
      )}
    </svg>
  );
}
