import React from "react";
import { instrumentIconUrl, type YargInstrument } from "./YargIcon";

// Difficulty-tier names, index 0 (no part) .. 7 (impossible).
export const TIER_LABELS = [
  "No Part", "Warmup", "Apprentice", "Solid",
  "Moderate", "Challenging", "Nightmare", "Impossible",
];

// A segmented ring showing a difficulty tier (0-7): `tier` of 7 segments are
// filled. With `instrument`, YARG's instrument sprite sits in the middle (as in
// YARG's song sidebar), dimmed when the part isn't charted; otherwise the tier
// number does. `size` scales the whole thing proportionally (default 44 = the
// editor's size; the browse list uses smaller).
export function DifficultyRing({
  tier,
  size = 44,
  instrument,
}: {
  tier: number;
  size?: number;
  instrument?: YargInstrument;
}) {
  const k = size / 44; // scale factor relative to the original 44px design
  const cx = size / 2;
  const cy = size / 2;
  const r = 17 * k;
  const segments = 7;
  const gapDeg = 18;
  const segmentAngle = (360 - segments * gapDeg) / segments;
  const strokeW = 5 * k;

  const arcPath = (i: number) => {
    const startDeg = -90 + i * (segmentAngle + gapDeg);
    const endDeg = startDeg + segmentAngle;
    const startRad = (startDeg * Math.PI) / 180;
    const endRad = (endDeg * Math.PI) / 180;
    const x1 = cx + r * Math.cos(startRad);
    const y1 = cy + r * Math.sin(startRad);
    const x2 = cx + r * Math.cos(endRad);
    const y2 = cy + r * Math.sin(endRad);
    return `M ${x1} ${y1} A ${r} ${r} 0 0 1 ${x2} ${y2}`;
  };

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      {/* Empty track segments — visible dark slots */}
      {Array.from({ length: segments }, (_, i) => (
        <path
          key={`bg-${i}`}
          d={arcPath(i)}
          fill="none"
          style={{ stroke: "var(--ring-track)" }}
          strokeWidth={strokeW}
          strokeLinecap="round"
        />
      ))}
      {/* Filled segments */}
      {Array.from({ length: segments }, (_, i) => {
        if (i >= tier) return null;
        const isDevil = tier === 7;
        return (
          <path
            key={`fg-${i}`}
            d={arcPath(i)}
            fill="none"
            style={{ stroke: isDevil ? "var(--tier-devil)" : "var(--text-primary)" }}
            strokeWidth={strokeW}
            strokeLinecap="round"
          />
        );
      })}
      {/* Center: instrument sprite, or the tier number */}
      {instrument ? (
        <image
          href={instrumentIconUrl(instrument)}
          x={cx - 12 * k}
          y={cy - 12 * k}
          width={24 * k}
          height={24 * k}
          opacity={tier > 0 ? 1 : 0.28}
        />
      ) : tier > 0 && (
        <text
          x={cx}
          y={cy + k}
          textAnchor="middle"
          dominantBaseline="central"
          style={{
            fill: tier === 7 ? "var(--tier-devil)" : "var(--text-primary)",
            fontFamily: "var(--font-ui)",
          }}
          fontSize={14 * k}
          fontWeight="700"
        >
          {tier}
        </text>
      )}
    </svg>
  );
}
