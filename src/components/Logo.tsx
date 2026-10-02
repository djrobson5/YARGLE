// The YARGLE logo, concept "01c · Song List" (ROADMAP item 13): a tilted song
// list with fret-colored gems and a cyan selected row. Shapes are verbatim
// from the design; don't restyle them. App icons are generated from the same
// marks by src-tauri/icons/source/build_icons.py.

interface LogoMarkProps {
  /** Rendered width/height in px. */
  size?: number;
  /** The 3-row variant designed for ~32 px and below. */
  compact?: boolean;
  className?: string;
}

export function LogoMark({ size = 100, compact = false, className }: LogoMarkProps) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 100 100"
      aria-hidden="true"
    >
      {compact ? (
        <g transform="skewX(-12) translate(10 0)">
          <circle cx="20" cy="17" r="11" fill="#17E289" />
          <rect x="36" y="10" width="46" height="14" rx="7" fill="#C7E0FF" />
          <circle cx="20" cy="50" r="11" fill="#FFBB0D" />
          <rect x="36" y="43" width="52" height="14" rx="7" fill="#2ED9FF" />
          <circle cx="20" cy="83" r="11" fill="#FF8413" />
          <rect x="36" y="76" width="36" height="14" rx="7" fill="#C7E0FF" />
        </g>
      ) : (
        <g transform="skewX(-12) translate(10 0)">
          <rect
            x="8"
            y="40"
            width="86"
            height="20"
            rx="10"
            fill="#2ED9FF"
            fillOpacity="0.18"
            stroke="#2ED9FF"
            strokeWidth="2"
          />
          <circle cx="20" cy="16" r="5.5" fill="#17E289" />
          <rect x="32" y="12.5" width="50" height="7" rx="3.5" fill="#C7E0FF" />
          <circle cx="20" cy="33" r="5.5" fill="#F32B37" />
          <rect x="32" y="29.5" width="38" height="7" rx="3.5" fill="#C7E0FF" />
          <circle cx="20" cy="50" r="5.5" fill="#FFBB0D" />
          <rect x="32" y="46.5" width="54" height="7" rx="3.5" fill="#FFFFFF" />
          <circle cx="20" cy="67" r="5.5" fill="#3784F9" />
          <rect x="32" y="63.5" width="30" height="7" rx="3.5" fill="#C7E0FF" />
          <circle cx="20" cy="84" r="5.5" fill="#FF8413" />
          <rect x="32" y="80.5" width="44" height="7" rx="3.5" fill="#C7E0FF" />
        </g>
      )}
    </svg>
  );
}

/** Mark + "YARGLE" wordmark (Barlow Black Italic). */
export function LogoLockup({ size = 96 }: { size?: number }) {
  return (
    <div className="logo-lockup" role="img" aria-label="YARGLE">
      <LogoMark size={size} />
      <span className="logo-wordmark" style={{ fontSize: size * 0.8 }}>
        YARGLE
      </span>
    </div>
  );
}
