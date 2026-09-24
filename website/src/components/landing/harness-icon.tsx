// The marks the app draws for each built-in harness — a copy of src/features/harness/HarnessIcon.tsx
// in the desktop app, which the site cannot import. Keep the two in step when a glyph changes.
// Plain marks in the spirit of each agent's own, not the trademarks themselves.

const spokes = [0, 30, 60, 90, 120, 150];

const GLYPHS: Record<string, { body: React.ReactNode; className?: string }> = {
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
  size = 16,
}: {
  id: string;
  label: string;
  size?: number;
}) {
  const glyph = GLYPHS[id];
  return (
    <svg
      aria-hidden
      focusable="false"
      viewBox="0 0 16 16"
      width={size}
      height={size}
      className={`shrink-0 ${glyph?.className ?? ""}`}
    >
      {glyph?.body ?? (
        // Anything without a glyph — every harness a user adds — is its initial in a frame.
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
            {label.charAt(0).toUpperCase()}
          </text>
        </g>
      )}
    </svg>
  );
}
