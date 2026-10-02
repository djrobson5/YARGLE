// YARG's own instrument and difficulty sprites, sliced from its Unity sprite
// sheets into public/icons/yarg/ by scripts/slice_yarg_sprites.py.

export type YargInstrument =
  | "guitar"
  | "bass"
  | "drums"
  | "vocals"
  | "keys"
  | "band"
  | "harmVocals"
  | "realGuitar"
  | "realBass"
  | "realDrums"
  | "realKeys"
  | "rhythm"
  | "guitarCoop"
  | "ghDrums"
  | "eliteDrums";

export type YargDifficulty = "Beginner" | "Easy" | "Medium" | "Hard" | "Expert" | "ExpertPlus";

export function instrumentIconUrl(instrument: YargInstrument): string {
  return `/icons/yarg/instruments/${instrument}.png`;
}

/** Map a free-form part name ("Guitar", "PART DRUMS", "Pro Keys"…) to a sprite. */
export function instrumentFromName(name: string): YargInstrument | null {
  const n = name.toLowerCase().replace(/^part\s+/, "");
  if (n.startsWith("pro ") || n.startsWith("real")) {
    if (n.includes("guitar")) return "realGuitar";
    if (n.includes("bass")) return "realBass";
    if (n.includes("drum")) return "realDrums";
    if (n.includes("key")) return "realKeys";
  }
  if (n.includes("harm")) return "harmVocals";
  if (n.includes("rhythm")) return "rhythm";
  if (n.includes("coop")) return "guitarCoop";
  if (n.includes("guitar")) return "guitar";
  if (n.includes("bass")) return "bass";
  if (n.includes("drum")) return "drums";
  if (n.includes("vocal")) return "vocals";
  if (n.includes("key")) return "keys";
  if (n.includes("band")) return "band";
  return null;
}

interface InstrumentIconProps {
  instrument: YargInstrument;
  size?: number;
  /** Not charted: drawn dimmed. */
  off?: boolean;
  title?: string;
  className?: string;
}

export function InstrumentIcon({ instrument, size = 20, off = false, title, className }: InstrumentIconProps) {
  return (
    <img
      className={`yarg-icon${off ? " yarg-icon-off" : ""}${className ? ` ${className}` : ""}`}
      src={instrumentIconUrl(instrument)}
      width={size}
      height={size}
      alt={title ?? instrument}
      title={title}
      draggable={false}
    />
  );
}

export function DifficultyIcon({
  difficulty,
  size = 20,
  className,
}: {
  difficulty: YargDifficulty;
  size?: number;
  className?: string;
}) {
  return (
    <img
      className={`yarg-icon${className ? ` ${className}` : ""}`}
      src={`/icons/yarg/difficulty/${difficulty}.png`}
      width={size}
      height={size}
      alt={difficulty}
      draggable={false}
    />
  );
}
